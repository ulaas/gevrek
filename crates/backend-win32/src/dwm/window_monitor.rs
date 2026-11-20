use crate::dwm::HwndExt;
use crate::dwm::WindowHandle;
pub use gevrek_engine::events::WindowEvent;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct WindowMonitor {
    event_receiver: Receiver<WindowEvent>,
    _event_sender: Sender<WindowEvent>,
    /// Display topology/settings notifications from the watcher window.
    display_receiver: Receiver<()>,
    // Message hook handle (for drag detection)
    _msg_hook_handle: Option<std::sync::Arc<std::sync::Mutex<HHOOK>>>,
    /// The window of the last `Focused` event handed out by `poll_events`.
    last_reported_focus: std::cell::Cell<Option<WindowHandle>>,
    /// Whether `poll_events` also looks at the foreground window itself
    /// (not in tests: they have no desktop of their own).
    watch_foreground: bool,
}

impl WindowMonitor {
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        let (display_sender, display_receiver) = channel();

        // Clone sender for the hook callback
        let hook_sender = sender.clone();

        // Set up Windows event hook in a separate thread
        std::thread::spawn(move || {
            unsafe {
                // The hook only transports OS facts. Rule decisions belong to
                // the runtime thread, which owns window metadata and lifecycle.
                HOOK_SENDER.with(|s| {
                    *s.borrow_mut() = Some(hook_sender);
                });
                DISPLAY_SENDER.with(|s| {
                    *s.borrow_mut() = Some(display_sender);
                });
                if create_display_watch_window().is_none() {
                    log::error!("Display change watcher could not be created; dock/undock will not be noticed");
                }

                // Hook only the event ranges we actually care about.
                // Previously hooked 0x00000001..0x7FFFFFFF which included every cursor,
                // tooltip, and accessibility event on the entire system - massive CPU overhead.

                // Hook 1: System events (movesizestart/end, minimize start/end)
                let hook_system = SetWinEventHook(
                    0x0001, // EVENT_SYSTEM_SOUND (start of system events)
                    0x0017, // EVENT_SYSTEM_MINIMIZEEND
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );

                // Hook 2: Object events (show, destroy, focus, locationchange, etc.)
                let hook_object = SetWinEventHook(
                    0x8000, // EVENT_OBJECT_CREATE
                    0x800C, // EVENT_OBJECT_NAMECHANGE
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );

                // Hook 3: cloak state (EVENT_OBJECT_CLOAKED/UNCLOAKED). A
                // separate narrow range: 0x800D-0x8016 holds high-volume
                // events (value/selection changes) nothing here needs.
                let hook_cloak = SetWinEventHook(
                    EVENT_OBJECT_CLOAKED,
                    EVENT_OBJECT_UNCLOAKED,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );

                // Hook 4: the desktop itself is another (Windows' own
                // question for an administrator's rights, the lock, and
                // back): what was held when it went is not known when it
                // comes back.
                let _hook_desktop = SetWinEventHook(
                    EVENT_SYSTEM_DESKTOPSWITCH,
                    EVENT_SYSTEM_DESKTOPSWITCH,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );

                if hook_system.is_invalid() || hook_object.is_invalid() {
                    log::error!("Failed to set Windows event hooks");
                    return;
                }
                if hook_cloak.is_invalid() {
                    log::warn!(
                        "Cloak event hook unavailable; uncloaked windows are adopted only on show"
                    );
                }

                log::info!(
                    "Windows event hooks installed (system 0x0001-0x0017, object 0x8000-0x800C, cloak 0x8017-0x8018)"
                );

                // Message loop
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                let _ = UnhookWinEvent(hook_system);
                let _ = UnhookWinEvent(hook_object);
                if !hook_cloak.is_invalid() {
                    let _ = UnhookWinEvent(hook_cloak);
                }
            }
        });

        // No need for separate message hook - we'll use accessibility events instead

        Self {
            event_receiver: receiver,
            _event_sender: sender,
            display_receiver,
            _msg_hook_handle: None, // We can't easily store the handle from the thread
            last_reported_focus: std::cell::Cell::new(None),
            watch_foreground: true,
        }
    }

    /// A monitor without the hook thread, for runtime tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn inert() -> Self {
        let (sender, receiver) = channel();
        let (_display_sender, display_receiver) = channel();
        Self {
            event_receiver: receiver,
            _event_sender: sender,
            display_receiver,
            _msg_hook_handle: None,
            last_reported_focus: std::cell::Cell::new(None),
            watch_foreground: false,
        }
    }

    pub fn poll_events(&self) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_receiver.try_recv() {
            events.push(event);
        }
        let foreground = if self.watch_foreground {
            foreground_top_level()
        } else {
            None
        };
        let last = self.last_reported_focus.get();
        let (events, last) = with_foreground(events, last, foreground);
        self.last_reported_focus.set(last);
        events
    }

    /// True when the display topology, a work area, DPI, or power state
    /// changed since the last call. Bursts collapse into one `true`.
    pub fn poll_display_changes(&self) -> bool {
        self.display_receiver.try_iter().count() > 0
    }
}

impl Default for WindowMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// The top-level window that has the foreground now, unless it is one of
/// gevrek's own (borders, stackbars) or there is none at this moment.
fn foreground_top_level() -> Option<WindowHandle> {
    // SAFETY: plain queries; a window that went away meanwhile gives a null
    // or stale handle, which the engine treats like any unknown window.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() || is_own_window(hwnd) {
            return None;
        }
        Some(WindowHandle::from_hwnd(root_window(hwnd)))
    }
}

/// The events of one pass, made to end with the window that really has the
/// foreground.
///
/// Windows does not announce every change of the foreground window: a
/// window activated while a key is held, or one that takes the foreground
/// back without being asked, can change it with no `EVENT_SYSTEM_FOREGROUND`
/// at all. Everything that follows the focus (which monitor commands act
/// on, borders, the focus history) then stays with the window before. So
/// the foreground window is also read on every pass, and a `Focused` event
/// is added when it is not the one last reported.
///
/// Returns the events and the window of the last `Focused` among them.
fn with_foreground(
    mut events: Vec<WindowEvent>,
    mut last: Option<WindowHandle>,
    foreground: Option<WindowHandle>,
) -> (Vec<WindowEvent>, Option<WindowHandle>) {
    for event in &events {
        match event {
            WindowEvent::Focused(handle) => last = Some(*handle),
            // A handle value that comes back names another window.
            WindowEvent::Destroyed(handle) if last == Some(*handle) => last = None,
            _ => {}
        }
    }
    if let Some(foreground) = foreground {
        if last != Some(foreground) {
            events.push(WindowEvent::Focused(foreground));
            last = Some(foreground);
        }
    }
    (events, last)
}

/// Messages that mean the monitor model may be stale.
fn is_display_change(message: u32, wparam: WPARAM) -> bool {
    const SPI_SETWORKAREA: usize = 0x002F;
    const PBT_APMRESUMESUSPEND: usize = 0x0007;
    const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
    match message {
        WM_DISPLAYCHANGE | WM_DPICHANGED => true,
        // The taskbar (or an app bar) changed a monitor's work area.
        WM_SETTINGCHANGE => wparam.0 == SPI_SETWORKAREA,
        // Resume can come back with a different topology (docked meanwhile).
        WM_POWERBROADCAST => matches!(wparam.0, PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC),
        _ => false,
    }
}

unsafe extern "system" fn display_watch_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_display_change(message, wparam) {
        DISPLAY_SENDER.with(|sender| {
            if let Some(sender) = sender.borrow().as_ref() {
                let _ = sender.send(());
                gevrek_host::wake::now();
            }
        });
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

/// A hidden top-level window on the hook thread. These messages are
/// broadcast to top-level windows only, so a message-only window would
/// never see them.
unsafe fn create_display_watch_window() -> Option<HWND> {
    let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
    let class_name = windows::core::w!("Gevrek_DisplayWatch");
    let class = WNDCLASSW {
        lpfnWndProc: Some(display_watch_proc),
        hInstance: instance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    // Re-registration fails harmlessly if the class already exists.
    RegisterClassW(&class);
    CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class_name,
        windows::core::w!("Gevrek display watch"),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        None,
        None,
        instance,
        None,
    )
    .ok()
}

thread_local! {
    static HOOK_SENDER: std::cell::RefCell<Option<Sender<WindowEvent>>> = const { std::cell::RefCell::new(None) };
    static DISPLAY_SENDER: std::cell::RefCell<Option<Sender<()>>> = const { std::cell::RefCell::new(None) };
    static LAST_WINDOW_GEOMETRY: std::cell::RefCell<HashMap<WindowHandle, WindowGeometry>> =
        std::cell::RefCell::new(HashMap::new());
    static LAST_FOCUSED: std::cell::Cell<Option<WindowHandle>> = const { std::cell::Cell::new(None) };
}

/// Focus is tracked per top-level window: a focus event on a child control
/// is reported as its root window.
fn root_window(hwnd: HWND) -> HWND {
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.is_invalid() {
        hwnd
    } else {
        root
    }
}

/// One activation fires both EVENT_SYSTEM_FOREGROUND and EVENT_OBJECT_FOCUS
/// (and focus moving between controls of one window fires more). Forward a
/// `Focused` event only when the focused top-level window actually changes.
fn focus_transition(last: &mut Option<WindowHandle>, root: WindowHandle) -> Option<WindowEvent> {
    (last.replace(root) != Some(root)).then_some(WindowEvent::Focused(root))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowGeometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// True only for events about the window itself (OBJID_WINDOW/CHILDID_SELF).
/// WinEvent also fires for accessibility objects inside windows; those are not
/// window lifecycle events and must be dropped at the transport layer.
fn is_window_level_event(id_object: i32, id_child: i32) -> bool {
    const OBJID_WINDOW: i32 = 0;
    const CHILDID_SELF: i32 = 0;
    id_object == OBJID_WINDOW && id_child == CHILDID_SELF
}

fn read_window_geometry(hwnd: HWND) -> Option<WindowGeometry> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        Some(WindowGeometry {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        })
    }
}

fn classify_geometry_change(
    handle: WindowHandle,
    previous: Option<WindowGeometry>,
    current: WindowGeometry,
) -> Vec<WindowEvent> {
    // A left/top-edge resize changes both origin and dimensions. Classify it
    // as a resize so one physical LOCATIONCHANGE produces one semantic event
    // and one suppression token is sufficient.
    if previous
        .is_none_or(|previous| previous.width != current.width || previous.height != current.height)
    {
        vec![WindowEvent::Resized(handle, current.width, current.height)]
    } else if previous.is_none_or(|previous| previous.x != current.x || previous.y != current.y) {
        vec![WindowEvent::Moved(handle, current.x, current.y)]
    } else {
        Vec::new()
    }
}

/// Whether a window is one of gevrek's own implementation windows, by its
/// class. The names are matched from their start, never anywhere inside:
/// that would catch windows of other programs.
unsafe fn is_own_window(hwnd: HWND) -> bool {
    let mut class_name: [u16; 256] = [0; 256];
    let class_len = GetClassNameW(hwnd, &mut class_name);
    if class_len <= 0 {
        return false;
    }
    let class_str = String::from_utf16_lossy(&class_name[0..class_len as usize]);
    class_str.starts_with("Gevrek_Border")
        || class_str.starts_with("Gevrek_Stackbar")
        || class_str.starts_with("Gevrek_Window")
        || class_str.starts_with("Gevrek_Overlay")
        // The shell's surfaces and their popups (crates/shellkit).
        || class_str.starts_with("Gevrek_Shell")
        || class_str == "GevrekHotkeyWindow"
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    // (Told of no window: before any is asked for.)
    if event == EVENT_SYSTEM_DESKTOPSWITCH {
        log::debug!("the desktop is another: what is held is forgotten");
        crate::hotkey::forget_what_is_held();
        return;
    }
    if hwnd.is_invalid() {
        return;
    }

    // Only window-level events matter. Non-window accessibility objects
    // (OBJID_CLIENT children, custom UIA objects like explorer's InputSite)
    // flood the channel on Windows 11 and starve real window events: each
    // junk event used to trigger an expensive ignore-check with a 100ms
    // SendMessageTimeout, so the queue grew faster than the loop drained it.
    if !is_window_level_event(_id_object, _id_child) {
        return;
    }

    // Hardcoded transport filter for Gevrek's own implementation windows.
    if is_own_window(hwnd) {
        return;
    }

    let handle = WindowHandle::from_hwnd(hwnd);

    // Only a top-level window can be adopted. Child controls raise the same
    // window-level show events (e.g. Task Manager re-shows its list header
    // every second), and one that slipped past the ignore rules was tiled
    // as if it were an application window.
    if matches!(event, EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED) && root_window(hwnd) != hwnd {
        return;
    }

    let event_code = event;
    let window_events =
        match event {
            EVENT_OBJECT_SHOW => {
                let mut seen_before = false;
                if let Some(geometry) = read_window_geometry(hwnd) {
                    seen_before = LAST_WINDOW_GEOMETRY
                        .with(|rects| rects.borrow_mut().insert(handle, geometry).is_some());
                }
                // A window that opens on another display than the one
                // with the keyboard is not seen there (`veil`). Not one
                // gevrek itself shows again.
                if !seen_before && !super::hidden_journal::is_hidden(handle) {
                    super::veil::window_opens(hwnd);
                }
                vec![WindowEvent::Created(handle)]
            }
            // A window the app hides itself (tray apps). The runtime drops the
            // WM's own hides using the hidden-window journal.
            EVENT_OBJECT_HIDE => vec![WindowEvent::Unmapped(handle)],
            // Windows that start cloaked (UWP launch, other virtual desktop) are
            // rejected on show; becoming uncloaked is when they can be adopted.
            // For an already managed window this is a no-op `Created`.
            EVENT_OBJECT_UNCLOAKED => vec![WindowEvent::Created(handle)],
            EVENT_OBJECT_DESTROY => {
                LAST_WINDOW_GEOMETRY.with(|rects| {
                    rects.borrow_mut().remove(&handle);
                });
                // A reused handle must be able to report focus again.
                LAST_FOCUSED.with(|last| {
                    if last.get() == Some(handle) {
                        last.set(None);
                    }
                });
                // From here on this HWND value names a different window: the
                // events of a new window that reuses it carry a new id.
                super::hwnd_map::retire(super::hwnd_map::hwnd_of(handle.0));
                vec![WindowEvent::Destroyed(handle)]
            }
            // EVENT_SYSTEM_FOREGROUND is the authoritative activation signal
            // (Alt-Tab, clicks, programmatic SetForegroundWindow). Without it the
            // focus model only saw the rare OBJID_WINDOW focus events.
            EVENT_SYSTEM_FOREGROUND | EVENT_OBJECT_FOCUS => {
                // A menu or a flyout tells of the focus inside it, and is
                // a window that cannot have the keyboard: the window that
                // has it is the one it had (whose border would else go
                // dim for as long as its own menu is open).
                const WS_EX_NOACTIVATE: i32 = 0x0800_0000;
                if GetWindowLongW(root_window(hwnd), GWL_EXSTYLE) & WS_EX_NOACTIVATE != 0 {
                    return;
                }
                let root = WindowHandle::from_hwnd(root_window(hwnd));
                LAST_FOCUSED.with(|last| {
                    let mut value = last.get();
                    let event = focus_transition(&mut value, root);
                    log::trace!(
                    "focus event 0x{event_id:04x} for {hwnd:?} (top-level {root:?}), last {:?}: {}",
                    last.get(),
                    if event.is_some() { "forwarded" } else { "the same window, dropped" },
                    event_id = event_code,
                );
                    last.set(value);
                    event.into_iter().collect()
                })
            }
            EVENT_OBJECT_NAMECHANGE => vec![WindowEvent::MetadataChanged(handle)],
            EVENT_OBJECT_LOCATIONCHANGE => {
                let Some(current) = read_window_geometry(hwnd) else {
                    return;
                };
                let previous =
                    LAST_WINDOW_GEOMETRY.with(|rects| rects.borrow_mut().insert(handle, current));
                classify_geometry_change(handle, previous, current)
            }
            // EVENT_OBJECT_REORDER (0x8004) is z-order change, NOT resize.
            EVENT_OBJECT_REORDER => Vec::new(),
            0x000A => {
                // EVENT_SYSTEM_MOVESIZESTART - User started moving or resizing a window
                log::info!(
                    "✅ EVENT_SYSTEM_MOVESIZESTART detected for window {:?}",
                    hwnd
                );
                vec![WindowEvent::DragStarted(handle)]
            }
            0x000B => {
                // EVENT_SYSTEM_MOVESIZEEND - User finished moving or resizing a window
                log::info!("✅ EVENT_SYSTEM_MOVESIZEEND detected for window {:?}", hwnd);
                vec![WindowEvent::DragEnded(handle)]
            }
            0x0016 => {
                // EVENT_SYSTEM_MINIMIZESTART - Window is being minimized
                log::info!("🔽 Window minimized: {:?}", hwnd);
                vec![WindowEvent::Minimized(handle)]
            }
            0x0017 => {
                // EVENT_SYSTEM_MINIMIZEEND - Window is being restored from minimized state
                log::info!("🔼 Window restored: {:?}", hwnd);
                vec![WindowEvent::Restored(handle)]
            }
            _ => Vec::new(),
        };

    HOOK_SENDER.with(|sender| {
        if let Some(s) = sender.borrow().as_ref() {
            let any = !window_events.is_empty();
            for event in window_events {
                let _ = s.send(event);
            }
            if any {
                gevrek_host::wake::now();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_window_level_events_are_forwarded() {
        assert!(is_window_level_event(0, 0));
        // OBJID_CLIENT (-4) child objects (Chrome render widgets)
        assert!(!is_window_level_event(-4, 12));
        // Custom UIA object ids (explorer InputSite spam)
        assert!(!is_window_level_event(512292, 0));
        assert!(!is_window_level_event(0, 7));
    }

    #[test]
    fn a_foreground_change_windows_did_not_announce_is_still_reported() {
        let (a, b) = (WindowHandle(1), WindowHandle(2));
        // Nothing announced, the foreground is `a`: reported once.
        let (events, last) = with_foreground(Vec::new(), None, Some(a));
        assert_eq!(events, vec![WindowEvent::Focused(a)]);
        let (events, last) = with_foreground(Vec::new(), last, Some(a));
        assert!(events.is_empty());
        // Announced: not said a second time.
        let (events, last) = with_foreground(vec![WindowEvent::Focused(b)], last, Some(b));
        assert_eq!(events, vec![WindowEvent::Focused(b)]);
        // An announcement that is out of date by now is followed by the truth.
        let (events, last) = with_foreground(vec![WindowEvent::Focused(a)], last, Some(b));
        assert_eq!(
            events,
            vec![WindowEvent::Focused(a), WindowEvent::Focused(b)]
        );
        // No foreground window at this moment: nothing is made up.
        let (events, last) = with_foreground(Vec::new(), last, None);
        assert!(events.is_empty());
        assert_eq!(last, Some(b));
        // A window that went away and whose handle value comes back is new.
        let (_, last) = with_foreground(vec![WindowEvent::Destroyed(b)], last, None);
        let (events, _) = with_foreground(Vec::new(), last, Some(b));
        assert_eq!(events, vec![WindowEvent::Focused(b)]);
    }

    #[test]
    fn display_watch_reacts_only_to_topology_messages() {
        assert!(is_display_change(WM_DISPLAYCHANGE, WPARAM(32)));
        assert!(is_display_change(WM_DPICHANGED, WPARAM(0)));
        assert!(is_display_change(WM_SETTINGCHANGE, WPARAM(0x002F)));
        // Other setting changes (wallpaper, locale, theme) do not count.
        assert!(!is_display_change(WM_SETTINGCHANGE, WPARAM(0x0014)));
        assert!(is_display_change(WM_POWERBROADCAST, WPARAM(0x0012)));
        // Suspend notifications do not; only resume does.
        assert!(!is_display_change(WM_POWERBROADCAST, WPARAM(0x0004)));
        assert!(!is_display_change(WM_PAINT, WPARAM(0)));
    }

    #[test]
    fn focus_transition_forwards_only_changes() {
        let a = WindowHandle(1);
        let b = WindowHandle(2);
        let mut last = None;
        assert_eq!(
            focus_transition(&mut last, a),
            Some(WindowEvent::Focused(a))
        );
        // FOREGROUND + OBJECT_FOCUS for the same activation: one event.
        assert_eq!(focus_transition(&mut last, a), None);
        assert_eq!(
            focus_transition(&mut last, b),
            Some(WindowEvent::Focused(b))
        );
        assert_eq!(
            focus_transition(&mut last, a),
            Some(WindowEvent::Focused(a))
        );
    }

    #[test]
    fn geometry_classifier_emits_only_the_changed_dimensions() {
        let handle = WindowHandle(7);
        let initial = WindowGeometry {
            x: 10,
            y: 20,
            width: 800,
            height: 600,
        };
        let moved = WindowGeometry { x: 30, ..initial };
        let resized = WindowGeometry {
            width: 1024,
            height: 768,
            ..moved
        };

        assert_eq!(
            classify_geometry_change(handle, Some(initial), moved),
            vec![WindowEvent::Moved(handle, 30, 20)]
        );
        assert_eq!(
            classify_geometry_change(handle, Some(moved), resized),
            vec![WindowEvent::Resized(handle, 1024, 768)]
        );
        assert!(classify_geometry_change(handle, Some(resized), resized).is_empty());
        assert_eq!(
            classify_geometry_change(handle, None, initial),
            vec![WindowEvent::Resized(handle, 800, 600)]
        );
    }
}
