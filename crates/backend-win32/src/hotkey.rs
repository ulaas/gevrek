use gevrek_engine::gesture;

pub use gesture::{
    GestureEvent, GesturePhase, GestureRecognizer, GestureRegistration, TouchpadContact,
    TouchpadFrame,
};
use gevrek_engine::input::{matching_registration_snapshot, should_consume, RawInputEvent};
pub use gevrek_engine::input::{Hotkey, HotkeyEvent};
pub use gevrek_model::keys::key_name_to_vk;
use gevrek_model::keys::vk_to_key_name;

use crate::settings::{BindingDevice, BindingTrigger};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetCaps, HidP_GetUsageValue, HidP_GetValueCaps, HidP_Input, HIDP_CAPS,
    HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList, RegisterRawInputDevices,
    HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICELIST, RAWINPUTHEADER, RIDEV_DEVNOTIFY,
    RIDEV_INPUTSINK, RIDI_DEVICEINFO, RIDI_PREPARSEDDATA, RID_DEVICE_INFO, RID_INPUT, RIM_TYPEHID,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Owns the low-level input thread and exposes resolved events to the runtime.
pub struct HotkeyManager {
    event_receiver: Receiver<HotkeyEvent>,
    gesture_event_receiver: Receiver<GestureEvent>,
    control_sender: Sender<ControlMessage>,
    gesture_backend_status: Arc<Mutex<GestureBackendStatus>>,
}

enum ControlMessage {
    Register(Hotkey, i32),
    Unregister(i32),
    ConfigureGestures {
        registrations: Vec<GestureRegistration>,
        idle_timeout_ms: u32,
        axis_lock_ratio: f32,
    },
    Shutdown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GestureBackendStatus {
    pub available: bool,
    pub backend: String,
    pub device_count: u32,
    pub last_error: Option<String>,
}

impl HotkeyManager {
    #[cfg(test)]
    pub fn input_is_suspended() -> bool {
        input_suspended()
    }

    /// A manager without the input thread or OS hooks, for runtime tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn inert() -> Self {
        let (_event_sender, event_receiver) = channel();
        let (_gesture_sender, gesture_event_receiver) = channel();
        let (control_sender, control_receiver) = channel();
        // Registrations are accepted and dropped.
        std::mem::forget(control_receiver);
        Self {
            event_receiver,
            gesture_event_receiver,
            control_sender,
            gesture_backend_status: Arc::new(Mutex::new(GestureBackendStatus::default())),
        }
    }

    /// Suspend (pause) or resume all bindings. Suspending also drops input
    /// events already queued, so nothing pressed during a pause replays on
    /// resume.
    pub fn set_input_suspended(&self, suspended: bool) {
        INPUT_SUSPENDED.store(suspended, std::sync::atomic::Ordering::Release);
        crate::elevated_helper::client::set_suspended(suspended);
        if suspended {
            let _ = self.poll_events();
            let _ = self.poll_gesture_events();
        }
    }

    pub fn new() -> Result<Self, String> {
        let (event_sender, event_receiver) = channel();
        let (gesture_event_sender, gesture_event_receiver) = channel();
        let (control_sender, control_receiver) = channel();
        let (startup_sender, startup_receiver) = channel();
        let gesture_backend_status = Arc::new(Mutex::new(GestureBackendStatus {
            backend: "Windows Precision Touchpad Raw Input".to_string(),
            ..GestureBackendStatus::default()
        }));
        let thread_gesture_status = Arc::clone(&gesture_backend_status);

        std::thread::spawn(move || {
            if let Err(error) = run_input_loop(
                event_sender,
                gesture_event_sender,
                control_receiver,
                startup_sender,
                thread_gesture_status,
            ) {
                log::error!("Advanced input engine stopped: {error}");
            }
        });

        match startup_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(error) => {
                let _ = control_sender.send(ControlMessage::Shutdown);
                return Err(format!(
                    "Timed out while starting the advanced input engine: {error}"
                ));
            }
        }

        Ok(Self {
            event_receiver,
            gesture_event_receiver,
            control_sender,
            gesture_backend_status,
        })
    }

    pub fn register(&self, hotkey: Hotkey, id: i32) -> Result<(), String> {
        validate_hotkey(&hotkey)?;
        self.control_sender
            .send(ControlMessage::Register(hotkey, id))
            .map_err(|error| format!("Failed to send input registration: {error}"))
    }

    pub fn unregister(&self, id: i32) -> Result<(), String> {
        self.control_sender
            .send(ControlMessage::Unregister(id))
            .map_err(|error| format!("Failed to send input unregistration: {error}"))
    }

    pub fn poll_events(&self) -> Vec<HotkeyEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_receiver.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn configure_gestures(
        &self,
        registrations: Vec<GestureRegistration>,
        idle_timeout_ms: u32,
        axis_lock_ratio: f32,
    ) -> Result<(), String> {
        self.control_sender
            .send(ControlMessage::ConfigureGestures {
                registrations,
                idle_timeout_ms,
                axis_lock_ratio,
            })
            .map_err(|error| format!("Failed to configure gestures: {error}"))
    }

    pub fn poll_gesture_events(&self) -> Vec<GestureEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.gesture_event_receiver.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn gesture_backend_status(&self) -> GestureBackendStatus {
        self.gesture_backend_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        let _ = self.control_sender.send(ControlMessage::Shutdown);
    }
}

/// How long a key of the keyboard that is held is not heard of again
/// before it counts as let go. A held key is told again and again (it
/// repeats, a few times a second at the slowest); one that is silent for
/// this long was let go where no hook heard of it.
const HELD_KEY_SILENCE: Duration = Duration::from_secs(2);

/// Whether a button of the mouse is down now, by its name in a binding.
/// None for a name that is no button (the wheel).
fn mouse_button_is_down(name: &str) -> Option<bool> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1, VK_XBUTTON2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    // (The state is the physical button's: with the buttons swapped, the
    // left one a binding names is the right one of the device.)
    // SAFETY: plain queries.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let key = match (name.to_ascii_uppercase().as_str(), swapped) {
        ("LEFT", false) | ("RIGHT", true) => VK_LBUTTON,
        ("RIGHT", false) | ("LEFT", true) => VK_RBUTTON,
        ("MIDDLE", _) => VK_MBUTTON,
        ("X1", _) => VK_XBUTTON1,
        ("X2", _) => VK_XBUTTON2,
        _ => return None,
    };
    // SAFETY: a plain query.
    Some(unsafe { GetAsyncKeyState(key.0 as i32) } < 0)
}

/// How often the hooks are put in anew. Windows takes a hook away, and
/// says nothing, when its answer once took too long (the machine stood
/// still for a moment): all of gevrek's keys were then dead until it was
/// started again.
const HOOKS_RENEWED_EVERY: Duration = Duration::from_secs(30);

/// Something happened after which what is held is not known any more
/// (the desktop went away for Windows' own question or the lock, and
/// came back): the input thread forgets what it holds and renews its
/// hooks.
static FORGET_HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The keys and buttons that are held are not known any more.
pub fn forget_what_is_held() {
    FORGET_HELD.store(true, std::sync::atomic::Ordering::Release);
    crate::windows_key::cancel_drag();
}

/// Whether a press that is told while its key or button already counts
/// as held is a new press: its release was not heard. A button of the
/// mouse never repeats; a key that repeats is not silent for long.
fn is_a_new_press(device: BindingDevice, since_last_heard: Duration) -> bool {
    device != BindingDevice::Keyboard || since_last_heard >= HELD_KEY_SILENCE
}

#[derive(Debug)]
struct PressedInput {
    raw: RawInputEvent,
    started: Instant,
    /// When it was last told of as down.
    last_heard: Instant,
    registrations: HashMap<i32, Hotkey>,
    long_press_emitted: HashSet<i32>,
}

#[derive(Clone)]
struct HookContext {
    raw_sender: Sender<RawInputEvent>,
    registrations: HashMap<i32, Hotkey>,
    consumed_inputs: HashSet<(BindingDevice, String)>,
}

static HOOK_CONTEXT: OnceLock<Mutex<Option<HookContext>>> = OnceLock::new();

/// While set (the WM is paused), the input hooks pass everything through:
/// no binding consumes keys or buttons and no input events are produced.
static INPUT_SUSPENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whose the input is that the hooks of this program answer.
///
/// A window manager that does not run as an administrator has a helper
/// that does (`elevated_helper`), and Windows keeps the keys and presses
/// that go to an elevated program's window from the hooks of one that is
/// not. So there are two pairs of hooks, with one rule between them: what
/// goes to a window of an elevated program is the helper's to answer,
/// everything else the window manager's. Each of the two leaves the
/// other's alone, whether Windows would have shown it to it or not.
static ROLE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(ROLE_ALONE);
/// No helper: all of it.
const ROLE_ALONE: u8 = 0;
/// The window manager, with a helper: all but what goes to elevated windows.
const ROLE_WITH_HELPER: u8 = 1;
/// The helper: only what goes to elevated windows.
const ROLE_HELPER: u8 = 2;

/// Whether the input that goes to this window (a handle's value; 0: to
/// none) is another's to answer, for a role.
fn others_to_answer(role: u8, goes_to_elevated: bool) -> bool {
    match role {
        ROLE_WITH_HELPER => goes_to_elevated,
        ROLE_HELPER => !goes_to_elevated,
        _ => false,
    }
}

/// Whether a key is another's to answer: by the window with the keyboard.
fn key_is_anothers() -> bool {
    let role = ROLE.load(std::sync::atomic::Ordering::Relaxed);
    if role == ROLE_ALONE {
        return false;
    }
    // SAFETY: a plain question.
    let window = unsafe { GetForegroundWindow() };
    others_to_answer(role, crate::elevated::window_is_elevated(window.0 as isize))
}

/// Whether a press of the mouse is another's to answer: by the window
/// under the pointer.
fn press_is_anothers(at: (i32, i32)) -> bool {
    let role = ROLE.load(std::sync::atomic::Ordering::Relaxed);
    if role == ROLE_ALONE {
        return false;
    }
    // SAFETY: plain questions about the window at a point.
    let window = unsafe {
        let under = WindowFromPoint(POINT { x: at.0, y: at.1 });
        let root = GetAncestor(under, GA_ROOT);
        if root.is_invalid() {
            under
        } else {
            root
        }
    };
    others_to_answer(role, crate::elevated::window_is_elevated(window.0 as isize))
}

/// The window manager has a helper whose hooks answer for elevated
/// windows (or has none any more).
pub fn set_helper_answers(answers: bool) {
    ROLE.store(
        if answers {
            ROLE_WITH_HELPER
        } else {
            ROLE_ALONE
        },
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// A key or a press the helper heard, as if this program's own hooks had:
/// the window manager's input thread makes of it what it makes of its own
/// (a press, a repeat, a release, a long press of a binding).
pub fn heard_by_helper(raw: RawInputEvent) {
    let context = hook_context()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(context) = context.as_ref() {
        let _ = context.raw_sender.send(raw);
    }
}

/// What the helper's hooks go by: the bindings, and whether they rest.
/// (The helper's own side of `sync_hook_registrations`.)
pub fn set_helper_table(registrations: HashMap<i32, Hotkey>, suspended: bool) {
    INPUT_SUSPENDED.store(suspended, std::sync::atomic::Ordering::Release);
    sync_hook_registrations(&registrations);
}

/// Whether the helper may hear a binding's key. A program with an
/// administrator's rights is kept apart from the others so that they do
/// not read what is typed into it; the helper passes on only what a
/// binding names, and a binding that names a plain letter would pass on
/// the letters. So a key of the keyboard must be held with the Windows
/// key, with Ctrl or with Alt (not with Ctrl and Alt alone, which is how
/// many layouts type their third signs), or be a key that types nothing
/// (F1 to F24, the keys for sound and players). A button of the mouse
/// says nothing of what is typed.
pub fn may_be_heard_by_helper(hotkey: &Hotkey) -> bool {
    if hotkey.device != BindingDevice::Keyboard {
        return true;
    }
    let third_signs = hotkey.ctrl && hotkey.alt && !hotkey.win;
    if (hotkey.win || hotkey.ctrl || hotkey.alt) && !third_signs {
        return true;
    }
    let key = hotkey.key.to_ascii_uppercase();
    let function = key
        .strip_prefix('F')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()));
    function
        || ["MEDIA_", "VOLUME_", "BROWSER_", "LAUNCH_"]
            .iter()
            .any(|kind| key.starts_with(kind))
}

/// Run the helper's hooks on a thread of their own: the same two hooks as
/// the window manager's, answering only what goes to elevated windows.
/// `heard` is called (on another thread) with each key and press that a
/// binding names, and with the release of each that was told of.
pub fn run_helper_hooks(heard: impl Fn(RawInputEvent) + Send + 'static) -> Result<(), String> {
    let (raw_sender, raw_receiver) = channel::<RawInputEvent>();
    {
        let mut context = hook_context()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *context = Some(HookContext {
            raw_sender,
            registrations: HashMap::new(),
            consumed_inputs: HashSet::new(),
        });
    }
    ROLE.store(ROLE_HELPER, std::sync::atomic::Ordering::Relaxed);
    let (started_sender, started) = channel::<Result<(), String>>();
    std::thread::spawn(move || {
        // SAFETY: the hooks are this thread's, which runs the loop of
        // messages they are called from until the process ends.
        unsafe {
            let keyboard = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook_proc),
                HINSTANCE::default(),
                0,
            );
            let mouse =
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), HINSTANCE::default(), 0);
            match (&keyboard, &mouse) {
                (Ok(_), Ok(_)) => {
                    let _ = started_sender.send(Ok(()));
                }
                (keyboard, mouse) => {
                    let _ = started_sender.send(Err(format!(
                        "the helper's hooks were not installed: {:?} {:?}",
                        keyboard.as_ref().err(),
                        mouse.as_ref().err()
                    )));
                    return;
                }
            }
            // (Renewed now and then, as the window manager's own are:
            // a timer of this thread wakes its loop for it.)
            let (mut keyboard, mut mouse) = match (keyboard, mouse) {
                (Ok(keyboard), Ok(mouse)) => (keyboard, mouse),
                _ => return,
            };
            let timer = SetTimer(None, 0, HOOKS_RENEWED_EVERY.as_millis() as u32, None);
            use crate::input_watch::{part, Part};
            crate::input_watch::renewed();
            crate::input_watch::start("the helper's");
            let mut message = MSG::default();
            while {
                part(Part::Waiting);
                GetMessageW(&mut message, HWND::default(), 0, 0).as_bool()
            } {
                if message.message == WM_TIMER && message.wParam.0 == timer {
                    part(Part::Renewing);
                    let _ = UnhookWindowsHookEx(keyboard);
                    let _ = UnhookWindowsHookEx(mouse);
                    if let Ok(renewed) = SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(keyboard_hook_proc),
                        HINSTANCE::default(),
                        0,
                    ) {
                        keyboard = renewed;
                    }
                    if let Ok(renewed) = SetWindowsHookExW(
                        WH_MOUSE_LL,
                        Some(mouse_hook_proc),
                        HINSTANCE::default(),
                        0,
                    ) {
                        mouse = renewed;
                    }
                    crate::input_watch::renewed();
                    continue;
                }
                part(Part::Messages);
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    });
    started
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("the helper's hooks did not start: {error}"))??;
    std::thread::spawn(move || {
        // The keys and buttons whose press was told of: their release is.
        let mut told: HashSet<(BindingDevice, String)> = HashSet::new();
        while let Ok(raw) = raw_receiver.recv() {
            let identity = (raw.device, raw.key.clone());
            let named = {
                let context = hook_context()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                context.as_ref().is_some_and(|context| {
                    context
                        .registrations
                        .values()
                        .any(|hotkey| hotkey.matches(&raw))
                })
            };
            let tell = if raw.momentary {
                named
            } else if raw.down {
                if named {
                    told.insert(identity);
                }
                named
            } else {
                told.remove(&identity)
            };
            if tell {
                heard(raw);
            }
        }
    });
    Ok(())
}

fn input_suspended() -> bool {
    INPUT_SUSPENDED.load(std::sync::atomic::Ordering::Acquire)
}

#[derive(Debug, Default)]
struct PartialTouchpadContact {
    id: Option<u32>,
    x: Option<f32>,
    y: Option<f32>,
}

struct TouchpadDeviceParser {
    preparsed_data: Vec<u8>,
    value_caps: Vec<HIDP_VALUE_CAPS>,
}

impl TouchpadDeviceParser {
    unsafe fn load(device: HANDLE) -> Result<Self, String> {
        let mut preparsed_size = 0u32;
        let query = GetRawInputDeviceInfoW(device, RIDI_PREPARSEDDATA, None, &mut preparsed_size);
        if query == u32::MAX || preparsed_size == 0 {
            return Err(format!(
                "Could not query Precision Touchpad HID descriptor (result {query})"
            ));
        }
        let mut preparsed_data = vec![0u8; preparsed_size as usize];
        let read = GetRawInputDeviceInfoW(
            device,
            RIDI_PREPARSEDDATA,
            Some(preparsed_data.as_mut_ptr().cast()),
            &mut preparsed_size,
        );
        if read == u32::MAX {
            return Err("Could not read Precision Touchpad HID descriptor".to_string());
        }
        let preparsed = PHIDP_PREPARSED_DATA(preparsed_data.as_ptr() as isize);
        let mut caps = HIDP_CAPS::default();
        if HidP_GetCaps(preparsed, &mut caps) != HIDP_STATUS_SUCCESS {
            return Err("Could not read Precision Touchpad HID capabilities".to_string());
        }
        let mut value_caps = vec![HIDP_VALUE_CAPS::default(); caps.NumberInputValueCaps as usize];
        let mut value_caps_len = caps.NumberInputValueCaps;
        if value_caps_len > 0
            && HidP_GetValueCaps(
                HidP_Input,
                value_caps.as_mut_ptr(),
                &mut value_caps_len,
                preparsed,
            ) != HIDP_STATUS_SUCCESS
        {
            return Err("Could not read Precision Touchpad HID value capabilities".to_string());
        }
        value_caps.truncate(value_caps_len as usize);
        value_caps.sort_by_key(|capability| capability.LinkCollection);
        Ok(Self {
            preparsed_data,
            value_caps,
        })
    }

    unsafe fn parse_reports(&self, raw_data: &[u8], report_size: usize) -> TouchpadFrame {
        let preparsed = PHIDP_PREPARSED_DATA(self.preparsed_data.as_ptr() as isize);
        let mut contact_count = None;
        let mut contacts = HashMap::<u16, PartialTouchpadContact>::new();
        for report in raw_data.chunks(report_size.max(1)) {
            for capability in &self.value_caps {
                let usage = if capability.IsRange.0 != 0 {
                    capability.Anonymous.Range.UsageMin
                } else {
                    capability.Anonymous.NotRange.Usage
                };
                let mut value = 0u32;
                if HidP_GetUsageValue(
                    HidP_Input,
                    capability.UsagePage,
                    capability.LinkCollection,
                    usage,
                    &mut value,
                    preparsed,
                    report,
                ) != HIDP_STATUS_SUCCESS
                {
                    continue;
                }
                if capability.LinkCollection == 0 {
                    if capability.UsagePage == 0x0D && usage == 0x54 {
                        contact_count = Some(value as usize);
                    }
                    continue;
                }
                let contact = contacts.entry(capability.LinkCollection).or_default();
                match (capability.UsagePage, usage) {
                    (0x0D, 0x51) => contact.id = Some(value),
                    (0x01, 0x30) => contact.x = Some(normalize_hid_value(value, capability)),
                    (0x01, 0x31) => contact.y = Some(normalize_hid_value(value, capability)),
                    _ => {}
                }
            }
        }
        let mut contacts = contacts
            .into_iter()
            .filter_map(|(link, contact)| {
                Some(TouchpadContact {
                    id: contact.id.unwrap_or(u32::from(link)),
                    x: contact.x?,
                    y: contact.y?,
                })
            })
            .collect::<Vec<_>>();
        contacts.sort_by_key(|contact| contact.id);
        contacts.truncate(contact_count.unwrap_or(contacts.len()));
        TouchpadFrame { contacts }
    }
}

fn normalize_hid_value(value: u32, capability: &HIDP_VALUE_CAPS) -> f32 {
    let minimum = capability.LogicalMin as f32;
    let maximum = capability.LogicalMax as f32;
    if maximum <= minimum {
        return 0.0;
    }
    ((value as f32 - minimum) / (maximum - minimum)).clamp(0.0, 1.0)
}

struct TouchpadContext {
    frame_sender: Sender<TouchpadFrame>,
    parsers: HashMap<isize, TouchpadDeviceParser>,
    status: Arc<Mutex<GestureBackendStatus>>,
}

static TOUCHPAD_CONTEXT: OnceLock<Mutex<Option<TouchpadContext>>> = OnceLock::new();

fn touchpad_context() -> &'static Mutex<Option<TouchpadContext>> {
    TOUCHPAD_CONTEXT.get_or_init(|| Mutex::new(None))
}

fn clear_touchpad_context() {
    let mut context = touchpad_context()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *context = None;
}

unsafe extern "system" fn touchpad_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_INPUT => {
            let mut context = touchpad_context()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(context) = context.as_mut() {
                if let Err(error) = parse_touchpad_input(lparam, context) {
                    let mut status = context
                        .status
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if status.last_error.as_deref() != Some(error.as_str()) {
                        log::warn!("Precision Touchpad input error: {error}");
                    }
                    status.last_error = Some(error);
                }
            }
        }
        WM_INPUT_DEVICE_CHANGE => {
            let count = precision_touchpad_device_count().unwrap_or(0);
            let mut context = touchpad_context()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(context) = context.as_mut() {
                context.parsers.clear();
                let mut status = context
                    .status
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                status.device_count = count;
                status.available = count > 0;
                status.last_error = None;
            }
        }
        WM_DESTROY => {
            PostQuitMessage(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn parse_touchpad_input(
    lparam: LPARAM,
    context: &mut TouchpadContext,
) -> Result<(), String> {
    let mut size = 0u32;
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    if GetRawInputData(
        HRAWINPUT(lparam.0 as *mut _),
        RID_INPUT,
        None,
        &mut size,
        header_size,
    ) != 0
        || size < std::mem::size_of::<RAWINPUT>() as u32
    {
        return Err("Could not query raw touchpad input size".to_string());
    }
    let word_size = std::mem::size_of::<usize>();
    let mut raw_words = vec![0usize; (size as usize).div_ceil(word_size)];
    let read = GetRawInputData(
        HRAWINPUT(lparam.0 as *mut _),
        RID_INPUT,
        Some(raw_words.as_mut_ptr().cast()),
        &mut size,
        header_size,
    );
    if read == u32::MAX || read != size {
        return Err("Could not read raw touchpad input".to_string());
    }
    let raw = &*(raw_words.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEHID.0 {
        return Ok(());
    }
    let hid = &raw.data.hid;
    let data_len = hid.dwSizeHid.saturating_mul(hid.dwCount) as usize;
    if data_len == 0 {
        return Ok(());
    }
    let data = std::slice::from_raw_parts(hid.bRawData.as_ptr(), data_len);
    let device_key = raw.header.hDevice.0 as isize;
    if let std::collections::hash_map::Entry::Vacant(entry) = context.parsers.entry(device_key) {
        entry.insert(TouchpadDeviceParser::load(raw.header.hDevice)?);
    }
    let frame = context
        .parsers
        .get(&device_key)
        .expect("touchpad parser inserted")
        .parse_reports(data, hid.dwSizeHid as usize);
    context
        .frame_sender
        .send(frame)
        .map_err(|error| format!("Gesture frame receiver was dropped: {error}"))
}

unsafe fn precision_touchpad_device_count() -> Result<u32, String> {
    let mut count = 0u32;
    let list_size = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    if GetRawInputDeviceList(None, &mut count, list_size) != 0 {
        return Err("Could not query raw input device count".to_string());
    }
    let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
    let listed = GetRawInputDeviceList(Some(devices.as_mut_ptr()), &mut count, list_size);
    if listed == u32::MAX {
        return Err("Could not enumerate raw input devices".to_string());
    }
    let mut touchpads = 0u32;
    for device in devices.into_iter().take(listed as usize) {
        if device.dwType != RIM_TYPEHID {
            continue;
        }
        let mut info = RID_DEVICE_INFO {
            cbSize: std::mem::size_of::<RID_DEVICE_INFO>() as u32,
            ..RID_DEVICE_INFO::default()
        };
        let mut info_size = info.cbSize;
        let result = GetRawInputDeviceInfoW(
            device.hDevice,
            RIDI_DEVICEINFO,
            Some((&mut info as *mut RID_DEVICE_INFO).cast()),
            &mut info_size,
        );
        if result == u32::MAX {
            continue;
        }
        let hid = info.Anonymous.hid;
        if hid.usUsagePage == 0x0D && hid.usUsage == 0x05 {
            touchpads += 1;
        }
    }
    Ok(touchpads)
}

unsafe fn create_touchpad_input_window(
    frame_sender: Sender<TouchpadFrame>,
    status: Arc<Mutex<GestureBackendStatus>>,
) -> Result<HWND, String> {
    let class_name = windows::core::w!("Gevrek_Touchpad_Input");
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(touchpad_window_proc),
        lpszClassName: class_name,
        ..WNDCLASSW::default()
    };
    let _ = RegisterClassW(&window_class);
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class_name,
        windows::core::w!("Gevrek Touchpad Input"),
        WINDOW_STYLE::default(),
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        None,
        None,
        None,
    )
    .map_err(|error| format!("Could not create the touchpad input window: {error}"))?;
    {
        let mut context = touchpad_context()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *context = Some(TouchpadContext {
            frame_sender,
            parsers: HashMap::new(),
            status: Arc::clone(&status),
        });
    }
    let registration = RAWINPUTDEVICE {
        usUsagePage: 0x0D,
        usUsage: 0x05,
        dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
        hwndTarget: hwnd,
    };
    if let Err(error) = RegisterRawInputDevices(
        &[registration],
        std::mem::size_of::<RAWINPUTDEVICE>() as u32,
    ) {
        let _ = DestroyWindow(hwnd);
        clear_touchpad_context();
        return Err(format!(
            "Could not register the Precision Touchpad HID collection: {error}"
        ));
    }
    let device_count = precision_touchpad_device_count()?;
    let mut status = status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    status.device_count = device_count;
    status.available = device_count > 0;
    status.last_error = None;
    Ok(hwnd)
}

fn hook_context() -> &'static Mutex<Option<HookContext>> {
    HOOK_CONTEXT.get_or_init(|| Mutex::new(None))
}

fn clear_hook_context() {
    let mut context = hook_context()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *context = None;
}

fn validate_hotkey(hotkey: &Hotkey) -> Result<(), String> {
    let key = hotkey.normalized_key();
    if key.is_empty() {
        return Err("Input key cannot be empty".to_string());
    }
    match hotkey.device {
        BindingDevice::Keyboard if key_name_to_vk(&key).is_none() => {
            Err(format!("Unknown keyboard key '{key}'"))
        }
        BindingDevice::Mouse
            if !matches!(
                key.as_str(),
                "LEFT" | "RIGHT" | "MIDDLE" | "X1" | "X2" | "WHEEL_UP" | "WHEEL_DOWN"
            ) =>
        {
            Err(format!("Unknown mouse input '{key}'"))
        }
        BindingDevice::Mouse
            if key.starts_with("WHEEL_") && hotkey.trigger != BindingTrigger::Press =>
        {
            Err("Mouse wheel inputs only support the press trigger".to_string())
        }
        _ => Ok(()),
    }
}

fn run_input_loop(
    event_sender: Sender<HotkeyEvent>,
    gesture_event_sender: Sender<GestureEvent>,
    control_receiver: Receiver<ControlMessage>,
    startup_sender: Sender<Result<(), String>>,
    gesture_backend_status: Arc<Mutex<GestureBackendStatus>>,
) -> Result<(), String> {
    let (raw_sender, raw_receiver) = channel();
    let (touchpad_frame_sender, touchpad_frame_receiver) = channel();
    let mut registrations = HashMap::<i32, Hotkey>::new();
    let mut pressed = HashMap::<(BindingDevice, String), PressedInput>::new();
    let mut gesture_recognizer = GestureRecognizer::default();
    let gesture_clock = Instant::now();

    {
        let mut context = hook_context()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *context = Some(HookContext {
            raw_sender,
            registrations: registrations.clone(),
            consumed_inputs: HashSet::new(),
        });
    }

    let mut keyboard_hook = match unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook_proc),
            HINSTANCE::default(),
            0,
        )
    } {
        Ok(hook) => hook,
        Err(error) => {
            let error = format!("Failed to install keyboard hook: {error}");
            let _ = startup_sender.send(Err(error.clone()));
            clear_hook_context();
            return Err(error);
        }
    };
    let mut mouse_hook = match unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), HINSTANCE::default(), 0)
    } {
        Ok(hook) => hook,
        Err(error) => {
            let _ = unsafe { UnhookWindowsHookEx(keyboard_hook) };
            let error = format!("Failed to install mouse hook: {error}");
            let _ = startup_sender.send(Err(error.clone()));
            clear_hook_context();
            return Err(error);
        }
    };

    let touchpad_window = match unsafe {
        create_touchpad_input_window(touchpad_frame_sender, Arc::clone(&gesture_backend_status))
    } {
        Ok(hwnd) => {
            let status = gesture_backend_status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            log::info!(
                "Precision Touchpad backend ready: {} device{}",
                status.device_count,
                if status.device_count == 1 { "" } else { "s" }
            );
            Some(hwnd)
        }
        Err(error) => {
            log::warn!("Precision Touchpad gestures unavailable: {error}");
            let mut status = gesture_backend_status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            status.available = false;
            status.last_error = Some(error);
            None
        }
    };

    let _ = startup_sender.send(Ok(()));
    log::info!("Unified keyboard and mouse input engine started");
    crate::input_watch::renewed();
    crate::input_watch::start("the window manager's");
    use crate::input_watch::{part, Part};
    let mut running = true;
    let mut hooks_renewed = Instant::now();
    while running {
        // What is held is not known any more: forgotten, here and in the
        // hooks, and the hooks are put in anew.
        let forget = FORGET_HELD.swap(false, std::sync::atomic::Ordering::AcqRel);
        if forget {
            pressed.clear();
            if let Some(context) = hook_context()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_mut()
            {
                context.consumed_inputs.clear();
            }
        }
        if forget || hooks_renewed.elapsed() >= HOOKS_RENEWED_EVERY {
            hooks_renewed = Instant::now();
            part(Part::Renewing);
            // (Taken out and put in on the thread they are called on,
            // between two of its looks at its messages: no key is told
            // to neither, and none to both.)
            // SAFETY: the hooks are this thread's own.
            unsafe {
                let _ = UnhookWindowsHookEx(keyboard_hook);
                let _ = UnhookWindowsHookEx(mouse_hook);
                match (
                    SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(keyboard_hook_proc),
                        HINSTANCE::default(),
                        0,
                    ),
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), HINSTANCE::default(), 0),
                ) {
                    (Ok(keyboard), Ok(mouse)) => {
                        keyboard_hook = keyboard;
                        mouse_hook = mouse;
                        crate::input_watch::renewed();
                    }
                    (keyboard, mouse) => {
                        log::error!(
                            "The input hooks could not be renewed (keyboard: {:?}, mouse: {:?}); gevrek's keys may not answer",
                            keyboard.as_ref().err(),
                            mouse.as_ref().err()
                        );
                        if let Ok(keyboard) = keyboard {
                            keyboard_hook = keyboard;
                        }
                        if let Ok(mouse) = mouse {
                            mouse_hook = mouse;
                        }
                    }
                }
            }
        }
        part(Part::Control);
        while let Ok(message) = control_receiver.try_recv() {
            match message {
                ControlMessage::Register(hotkey, id) => {
                    registrations.insert(id, hotkey);
                    sync_hook_registrations(&registrations);
                }
                ControlMessage::Unregister(id) => {
                    registrations.remove(&id);
                    for state in pressed.values_mut() {
                        state.registrations.remove(&id);
                        state.long_press_emitted.remove(&id);
                    }
                    sync_hook_registrations(&registrations);
                }
                ControlMessage::ConfigureGestures {
                    registrations,
                    idle_timeout_ms,
                    axis_lock_ratio,
                } => {
                    gesture_recognizer.configure(registrations, idle_timeout_ms, axis_lock_ratio);
                }
                ControlMessage::Shutdown => {
                    running = false;
                }
            }
        }

        part(Part::Keys);
        while let Ok(raw) = raw_receiver.try_recv() {
            let identity = (raw.device, raw.key.clone());
            if raw.momentary {
                emit_matching(
                    &event_sender,
                    &registrations,
                    &raw,
                    BindingTrigger::Press,
                    0,
                    None,
                );
                continue;
            }

            // Held already, and told as pressed: a repeat, unless its
            // release was not heard (`is_a_new_press`), then it is let go
            // first and pressed anew.
            if raw.down
                && pressed
                    .get(&identity)
                    .is_some_and(|state| is_a_new_press(raw.device, state.last_heard.elapsed()))
            {
                pressed.remove(&identity);
            }
            if raw.down {
                if let Some(state) = pressed.get_mut(&identity) {
                    state.last_heard = Instant::now();
                    emit_matching(
                        &event_sender,
                        &state.registrations,
                        &raw,
                        BindingTrigger::Repeat,
                        elapsed_ms(state.started),
                        None,
                    );
                } else {
                    let registration_snapshot =
                        matching_registration_snapshot(&registrations, &raw);
                    emit_matching(
                        &event_sender,
                        &registration_snapshot,
                        &raw,
                        BindingTrigger::Press,
                        0,
                        None,
                    );
                    pressed.insert(
                        identity,
                        PressedInput {
                            raw,
                            started: Instant::now(),
                            last_heard: Instant::now(),
                            registrations: registration_snapshot,
                            long_press_emitted: HashSet::new(),
                        },
                    );
                }
            } else if let Some(state) = pressed.remove(&identity) {
                emit_matching(
                    &event_sender,
                    &state.registrations,
                    &state.raw,
                    BindingTrigger::Release,
                    elapsed_ms(state.started),
                    None,
                );
            }
        }

        // A key that has been silent for long is not held any more: no
        // long press comes of it.
        pressed.retain(|(device, _), state| match device {
            BindingDevice::Keyboard => state.last_heard.elapsed() < HELD_KEY_SILENCE,
            // A button of the mouse says nothing while it is held, so it
            // is asked: one that is up is not held, whoever heard it come
            // up. (Let go over a window of a program with more rights, its
            // release reached nobody here, and it was held for good: its
            // long press came with the button up.)
            // (Not in its first moments: a press that was made up, and
            // not by a button, is let go by whoever made it up.)
            BindingDevice::Mouse => {
                state.started.elapsed() < HELD_KEY_SILENCE
                    || mouse_button_is_down(&state.raw.key).unwrap_or(true)
            }
        });
        for state in pressed.values_mut() {
            let held_ms = elapsed_ms(state.started);
            let due_ids = state
                .registrations
                .iter()
                .filter(|(id, hotkey)| {
                    hotkey.trigger == BindingTrigger::LongPress
                        && hotkey.matches(&state.raw)
                        && held_ms >= hotkey.long_press_ms
                        && !state.long_press_emitted.contains(id)
                })
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            if !due_ids.is_empty() {
                state.long_press_emitted.extend(due_ids.iter().copied());
                emit_matching(
                    &event_sender,
                    &state.registrations,
                    &state.raw,
                    BindingTrigger::LongPress,
                    held_ms,
                    Some(due_ids),
                );
            }
        }

        part(Part::Touchpad);
        while let Ok(frame) = touchpad_frame_receiver.try_recv() {
            for event in gesture_recognizer.feed_frame(frame, elapsed_ms_u64(gesture_clock)) {
                if gesture_event_sender.send(event).is_err() {
                    log::warn!("Gesture event receiver was dropped");
                }
                gevrek_host::wake::now();
            }
        }
        if let Some(event) = gesture_recognizer.tick(elapsed_ms_u64(gesture_clock)) {
            if gesture_event_sender.send(event).is_err() {
                log::warn!("Gesture event receiver was dropped");
            }
            gevrek_host::wake::now();
        }

        part(Part::Messages);
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    running = false;
                    break;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        // Wait for a message, at most as long as the pass used to sleep.
        // The hooks of this thread are called only while it waits for or
        // takes messages: asleep, every key and every movement of the
        // mouse, of every program, waited for it to wake.
        part(Part::Waiting);
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                MsgWaitForMultipleObjectsEx, MWMO_INPUTAVAILABLE, QS_ALLINPUT,
            };
            MsgWaitForMultipleObjectsEx(None, 4, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }

    let _ = unsafe { UnhookWindowsHookEx(mouse_hook) };
    let _ = unsafe { UnhookWindowsHookEx(keyboard_hook) };
    if let Some(hwnd) = touchpad_window {
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    clear_touchpad_context();
    clear_hook_context();
    log::info!("Unified keyboard and mouse input engine stopped");
    Ok(())
}

fn sync_hook_registrations(registrations: &HashMap<i32, Hotkey>) {
    let mut context = hook_context()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(context) = context.as_mut() {
        context.registrations = registrations.clone();
    }
    drop(context);
    // (The helper's hooks go by the same bindings: `elevated_helper`.)
    if ROLE.load(std::sync::atomic::Ordering::Relaxed) != ROLE_HELPER {
        crate::elevated_helper::client::set_bindings(registrations);
    }
}

fn emit_matching(
    sender: &Sender<HotkeyEvent>,
    registrations: &HashMap<i32, Hotkey>,
    raw: &RawInputEvent,
    trigger: BindingTrigger,
    held_ms: u32,
    explicit_ids: Option<Vec<i32>>,
) {
    let registration_ids = explicit_ids.unwrap_or_else(|| {
        registrations
            .iter()
            .filter(|(_, hotkey)| hotkey.trigger == trigger && hotkey.matches(raw))
            .map(|(id, _)| *id)
            .collect()
    });
    if registration_ids.is_empty() {
        return;
    }
    let event = HotkeyEvent {
        registration_ids,
        key: raw.key.clone(),
        ctrl: raw.ctrl,
        alt: raw.alt,
        shift: raw.shift,
        win: raw.win,
        device: raw.device,
        trigger,
        held_ms,
    };
    if sender.send(event).is_err() {
        log::warn!("Advanced input event receiver was dropped");
    }
    gevrek_host::wake::now();
}

fn elapsed_ms(started: Instant) -> u32 {
    started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

fn elapsed_ms_u64(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // (Counted and timed, so that hooks that stopped hearing are said:
    // `input_watch`.)
    let _inside = crate::input_watch::KEYBOARD.enter();
    if code < 0 || input_suspended() {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    let message = wparam.0 as u32;
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
    if !down && !up {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }

    let data = *(lparam.0 as *const KBDLLHOOKSTRUCT);
    // What goes to a window of an elevated program is the helper's to
    // answer, and all else the window manager's (`ROLE`).
    if key_is_anothers() {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    // A key gevrek pressed itself for the system and its programs (the
    // shell's key for "play", asked for by a key that is bound to the
    // shell; `gevrek_system::every_key::OWN_MARK`) is not answered again.
    if data.dwExtraInfo == 0x6765_7672 {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    // Escape while a window is being picked ends that (`pick.rs`).
    if data.vkCode == 0x1B && crate::pick::escape(down) {
        return LRESULT(1);
    }
    // The Windows key itself: Windows hears of it (so that it knows when
    // the key is let go), and is told that it was used with something,
    // where the key is gevrek's (`windows_key.rs`).
    if matches!(data.vkCode, 0x5B | 0x5C) {
        if down {
            crate::windows_key::key_pressed();
        } else {
            crate::windows_key::key_released();
        }
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    let Some(key) = vk_to_key_name(data.vkCode as u16) else {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    };
    let (ctrl, alt, shift, win) = current_modifiers();
    let raw = RawInputEvent {
        key,
        ctrl,
        alt,
        shift,
        win,
        device: BindingDevice::Keyboard,
        down,
        momentary: false,
    };

    // (The lock is let go before the input goes on: `CallNextHookEx` and
    // `SendInput` can have Windows call this thread's other hook, which
    // takes the same lock, and a thread waiting for itself waits for good.)
    let consume = {
        let mut context_guard = hook_context()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(context) = context_guard.as_mut() else {
            drop(context_guard);
            return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
        };
        let identity = (raw.device, raw.key.clone());
        let configured_consume = should_consume(&context.registrations, &raw);
        // A key pressed with the Windows key is gevrek's whether it is
        // bound or not, where the key is gevrek's: Windows' own shortcuts
        // on it are not carried out. (Only as it goes down: a key that
        // was down before the Windows key is let go of for everyone.)
        // (Not Shift, Ctrl or Alt themselves: they are part of a chord with
        // the Windows key, and Windows must know that they are held, or
        // the key that follows would be read without them.)
        let owned = down
            && raw.win
            && crate::windows_key::owns_key()
            && !matches!(raw.key.as_str(), "SHIFT" | "CTRL" | "ALT" | "WIN");
        let consume = if down {
            if configured_consume || owned {
                context.consumed_inputs.insert(identity);
            } else {
                // (Its last release was never seen, behind a locked
                // screen or a program with more rights: this press goes to
                // the programs, and so will its release.)
                context.consumed_inputs.remove(&identity);
            }
            configured_consume || owned
        } else {
            // (Only when its press was kept from the programs too: one
            // that saw a key go down and never come up has it held for good.)
            context.consumed_inputs.remove(&identity)
        };
        let _ = context.raw_sender.send(raw.clone());
        consume
    };
    if consume && down {
        mask_modifier_release(&raw);
    }
    if consume {
        return LRESULT(1);
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

/// Unassigned virtual key used to mask a modifier release. Its name
/// lookup fails, so the hook passes it through untouched.
const MENU_MASK_VK: u16 = 0xE8;

/// Keeps a lone Win/Alt release from opening Start or the menu bar after a
/// consumed hotkey, without swallowing the release itself.
///
/// Swallowing the key-up (the old approach) left the modifier logically held
/// for the whole system. Instead, while the modifier is still down, tap an
/// unassigned key: the shell then sees "Win/Alt used in a combination" and
/// ignores the release, which reaches the system normally.
fn mask_modifier_release(raw: &RawInputEvent) {
    if !(raw.win || raw.alt) {
        return;
    }
    let key_input = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(MENU_MASK_VK),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key_input(KEYBD_EVENT_FLAGS(0)), key_input(KEYEVENTF_KEYUP)];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        log::debug!("Modifier mask key injection was blocked ({sent} of 2 sent)");
    }
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let _inside = crate::input_watch::MOUSE.enter();
    if code < 0 || input_suspended() {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    let data = *(lparam.0 as *const MSLLHOOKSTRUCT);
    let message = wparam.0 as u32;
    let at = (data.pt.x, data.pt.y);
    // A window is being picked by a press on it (`pick.rs`).
    match message {
        WM_LBUTTONDOWN if crate::pick::pressed(at) => return LRESULT(1),
        WM_LBUTTONUP if crate::pick::released() => return LRESULT(1),
        _ => {}
    }
    // A window dragged with the Windows key and the mouse (`windows_key.rs`).
    match message {
        WM_MOUSEMOVE => {
            crate::windows_key::pointer_moved(at);
            return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
        }
        // What goes to a window of an elevated program is the helper's to
        // answer, and all else the window manager's (`ROLE`). A drag that
        // this program began is its own to its end, wherever the pointer
        // is by then.
        _ if !crate::windows_key::is_dragging() && press_is_anothers(at) => {
            return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            crate::windows_key::note_press(at);
            let (ctrl, alt, shift, win) = current_modifiers();
            if win
                && !ctrl
                && !alt
                && !shift
                && crate::windows_key::pressed(message == WM_LBUTTONDOWN, at)
            {
                return LRESULT(1);
            }
        }
        WM_LBUTTONUP | WM_RBUTTONUP if crate::windows_key::released(message == WM_LBUTTONUP) => {
            return LRESULT(1);
        }
        _ => {}
    }
    let (key, down, momentary) = match message {
        WM_LBUTTONDOWN => ("LEFT", true, false),
        WM_LBUTTONUP => ("LEFT", false, false),
        WM_RBUTTONDOWN => ("RIGHT", true, false),
        WM_RBUTTONUP => ("RIGHT", false, false),
        WM_MBUTTONDOWN => ("MIDDLE", true, false),
        WM_MBUTTONUP => ("MIDDLE", false, false),
        WM_XBUTTONDOWN => {
            let key = if (data.mouseData >> 16) as u16 == XBUTTON1 {
                "X1"
            } else {
                "X2"
            };
            (key, true, false)
        }
        WM_XBUTTONUP => {
            let key = if (data.mouseData >> 16) as u16 == XBUTTON1 {
                "X1"
            } else {
                "X2"
            };
            (key, false, false)
        }
        WM_MOUSEWHEEL => {
            let key = if ((data.mouseData >> 16) as i16) > 0 {
                "WHEEL_UP"
            } else {
                "WHEEL_DOWN"
            };
            (key, true, true)
        }
        _ => return CallNextHookEx(HHOOK::default(), code, wparam, lparam),
    };
    let (ctrl, alt, shift, win) = current_modifiers();
    let raw = RawInputEvent {
        key: key.to_string(),
        ctrl,
        alt,
        shift,
        win,
        device: BindingDevice::Mouse,
        down,
        momentary,
    };

    // (The lock is let go before the input goes on, as in the keyboard's
    // hook.)
    let consume = {
        let mut context_guard = hook_context()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(context) = context_guard.as_mut() else {
            drop(context_guard);
            return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
        };
        let identity = (raw.device, raw.key.clone());
        let configured_consume = should_consume(&context.registrations, &raw);
        let consume = if raw.momentary {
            configured_consume
        } else if down {
            if configured_consume {
                context.consumed_inputs.insert(identity);
            } else {
                context.consumed_inputs.remove(&identity);
            }
            configured_consume
        } else {
            // (Only when its press was kept from the programs too: one
            // that saw a key go down and never come up has it held for good.)
            context.consumed_inputs.remove(&identity)
        };
        let _ = context.raw_sender.send(raw.clone());
        consume
    };
    if consume && down {
        mask_modifier_release(&raw);
    }
    if consume {
        return LRESULT(1);
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

fn current_modifiers() -> (bool, bool, bool, bool) {
    unsafe {
        let down = |key: VIRTUAL_KEY| GetAsyncKeyState(key.0 as i32) < 0;
        (
            down(VK_CONTROL),
            down(VK_MENU),
            down(VK_SHIFT),
            down(VK_LWIN) || down(VK_RWIN),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pausing_suspends_bindings_and_drops_queued_input() {
        let manager = HotkeyManager::inert();
        manager.set_input_suspended(true);
        assert!(HotkeyManager::input_is_suspended());
        assert!(manager.poll_events().is_empty());
        manager.set_input_suspended(false);
        assert!(!HotkeyManager::input_is_suspended());
    }

    #[test]
    fn key_names_round_trip_for_supported_keyboard_surface() {
        for name in [
            "A",
            "9",
            "F1",
            "F24",
            "SPACE",
            "PAGEUP",
            "BACKTICK",
            "VOLUME_UP",
            "MEDIA_PLAY_PAUSE",
        ] {
            let vk = key_name_to_vk(name).unwrap();
            assert_eq!(key_name_to_vk(&vk_to_key_name(vk).unwrap()), Some(vk));
        }
    }

    #[test]
    fn registration_validation_rejects_invalid_device_inputs() {
        let mut hotkey = Hotkey::new("NOT_A_KEY".to_string(), false, false, false, true);
        assert!(validate_hotkey(&hotkey).is_err());
        hotkey.device = BindingDevice::Mouse;
        hotkey.key = "WHEEL_UP".to_string();
        assert!(validate_hotkey(&hotkey).is_ok());
        hotkey.trigger = BindingTrigger::Release;
        assert!(validate_hotkey(&hotkey).is_err());
    }

    #[test]
    fn trigger_resolution_selects_only_matching_registration_ids() {
        let (sender, receiver) = channel();
        let press = Hotkey {
            key: "H".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
            device: BindingDevice::Keyboard,
            trigger: BindingTrigger::Press,
            consume: true,
            long_press_ms: 500,
        };
        let repeat = Hotkey {
            trigger: BindingTrigger::Repeat,
            ..press.clone()
        };
        let registrations = HashMap::from([(10, press), (11, repeat)]);
        let raw = RawInputEvent {
            key: "H".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
            device: BindingDevice::Keyboard,
            down: true,
            momentary: false,
        };

        emit_matching(
            &sender,
            &registrations,
            &raw,
            BindingTrigger::Press,
            0,
            None,
        );
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.registration_ids, vec![10]);
        assert_eq!(event.trigger, BindingTrigger::Press);

        emit_matching(
            &sender,
            &registrations,
            &raw,
            BindingTrigger::Repeat,
            440,
            None,
        );
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.registration_ids, vec![11]);
        assert_eq!(event.held_ms, 440);
    }

    #[test]
    fn consume_policy_is_scoped_to_exact_device_and_modifiers() {
        let mouse = Hotkey {
            key: "X1".to_string(),
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
            device: BindingDevice::Mouse,
            trigger: BindingTrigger::Release,
            consume: true,
            long_press_ms: 500,
        };
        let registrations = HashMap::from([(1, mouse)]);
        let mut raw = RawInputEvent {
            key: "X1".to_string(),
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
            device: BindingDevice::Mouse,
            down: false,
            momentary: false,
        };
        assert!(should_consume(&registrations, &raw));
        raw.ctrl = false;
        assert!(!should_consume(&registrations, &raw));
        raw.ctrl = true;
        raw.device = BindingDevice::Keyboard;
        assert!(!should_consume(&registrations, &raw));
    }

    #[test]
    fn held_input_snapshot_does_not_adopt_new_submap_registrations() {
        let raw = RawInputEvent {
            key: "H".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
            device: BindingDevice::Keyboard,
            down: true,
            momentary: false,
        };
        let old_release = Hotkey {
            key: "H".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
            device: BindingDevice::Keyboard,
            trigger: BindingTrigger::Release,
            consume: true,
            long_press_ms: 500,
        };
        let snapshot =
            matching_registration_snapshot(&HashMap::from([(10, old_release.clone())]), &raw);
        let live_registrations = HashMap::from([(
            20,
            Hotkey {
                trigger: BindingTrigger::Release,
                ..old_release
            },
        )]);

        assert!(snapshot.contains_key(&10));
        assert!(!snapshot.contains_key(&20));
        assert!(live_registrations.contains_key(&20));
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn a_press_whose_release_was_not_heard_is_a_press_again() {
        // A held key repeats: told again soon, it is a repeat.
        assert!(!is_a_new_press(
            BindingDevice::Keyboard,
            Duration::from_millis(40)
        ));
        assert!(!is_a_new_press(
            BindingDevice::Keyboard,
            Duration::from_millis(600)
        ));
        // Silent for long and then down again: it had been let go.
        assert!(is_a_new_press(BindingDevice::Keyboard, HELD_KEY_SILENCE));
        // A button of the mouse does not repeat at all.
        assert!(is_a_new_press(
            BindingDevice::Mouse,
            Duration::from_millis(1)
        ));
    }

    fn key(name: &str, ctrl: bool, alt: bool, win: bool) -> Hotkey {
        Hotkey::new(name.to_string(), ctrl, alt, false, win)
    }

    #[test]
    fn each_of_the_two_answers_its_own_input_and_leaves_the_others() {
        // No helper: the window manager answers all of it.
        assert!(!others_to_answer(ROLE_ALONE, true));
        assert!(!others_to_answer(ROLE_ALONE, false));
        // With one: what goes to an elevated window is the helper's.
        assert!(others_to_answer(ROLE_WITH_HELPER, true));
        assert!(!others_to_answer(ROLE_WITH_HELPER, false));
        assert!(!others_to_answer(ROLE_HELPER, true));
        assert!(others_to_answer(ROLE_HELPER, false));
    }

    #[test]
    fn the_helper_hears_no_binding_that_would_pass_on_what_is_typed() {
        // A key held with the Windows key, with Ctrl or with Alt.
        assert!(may_be_heard_by_helper(&key("J", false, false, true)));
        assert!(may_be_heard_by_helper(&key("C", true, false, false)));
        assert!(may_be_heard_by_helper(&key("TAB", false, true, false)));
        assert!(may_be_heard_by_helper(&key("Q", true, true, true)));
        // A plain letter, with Shift or without, and the third signs of
        // a layout (Ctrl and Alt alone): those are what is typed.
        assert!(!may_be_heard_by_helper(&key("A", false, false, false)));
        assert!(!may_be_heard_by_helper(&Hotkey::new(
            "A".to_string(),
            false,
            false,
            true,
            false
        )));
        assert!(!may_be_heard_by_helper(&key("Q", true, true, false)));
        // Keys that type nothing.
        assert!(may_be_heard_by_helper(&key("F12", false, false, false)));
        assert!(may_be_heard_by_helper(&key(
            "VOLUME_UP",
            false,
            false,
            false
        )));
        assert!(!may_be_heard_by_helper(&key("F", false, false, false)));
        // A button of the mouse says nothing of what is typed.
        let mut button = key("MIDDLE", false, false, false);
        button.device = BindingDevice::Mouse;
        assert!(may_be_heard_by_helper(&button));
    }
}
