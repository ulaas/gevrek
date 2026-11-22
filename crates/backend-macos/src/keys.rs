//! The keys and mouse buttons gevrek's bindings are on, taken before any
//! program sees them (an event tap of the window server): keys, the mouse's
//! buttons (`LEFT`, `RIGHT`, `MIDDLE`, `X1`, `X2`) and the steps of its
//! wheel (`WHEEL_UP`, `WHEEL_DOWN`; the touchpad's scrolling is left to the
//! programs), long presses, a held button's repeat; and windows moved (the
//! left button) and resized (the right) with the drag modifier held, on a
//! window gevrek manages.
//!
//! A key that hits a binding is told to the host (`bindings_triggered`);
//! one whose binding says so (`consume`) goes no further, with its release
//! and its repeats. Every other key goes on to the program in front.
//!
//! The modifiers of a binding are held on a Mac as:
//!
//! | binding | held on a Mac                  |
//! |---------|--------------------------------|
//! | Win     | Option (⌥)                     |
//! | Alt     | Control, Option and Command    |
//! | Ctrl    | Control                        |
//! | Shift   | Shift                          |
//!
//! Command's shortcuts (copy, paste, quit, ...) stay every program's: only
//! Alt has Command, and only with Control and Option, which no program
//! holds together. (Option is what other tiling window managers for macOS
//! hold their keys with.) Alt is not Option with Control alone: that is
//! Win with Control, and the standard bindings have both (`Lock the
//! screen` and `Move window right`, `Move` and `Send window to workspace`).
//!
//! Keys are named through the same virtual-key codes as on the other
//! systems (`gevrek_model::keys`), so a binding is written the same
//! everywhere: a letter by the key that writes it in the layout in use
//! (`layout`), every other key by its place.

use crate::ffi;
use gevrek_engine::input::{Hotkey, HotkeyEvent};
use gevrek_model::settings::{BindingDevice, BindingTrigger};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The modifier keys held, as a Mac has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub command: bool,
}

impl Held {
    /// What the window server's flags of an event say is held.
    pub fn of_flags(flags: u64) -> Self {
        Self {
            shift: flags & 0x0002_0000 != 0,
            control: flags & 0x0004_0000 != 0,
            option: flags & 0x0008_0000 != 0,
            command: flags & 0x0010_0000 != 0,
        }
    }

    /// What a binding's modifiers are held as (see the module).
    pub fn of_binding(hotkey: &Hotkey) -> Self {
        Self {
            control: hotkey.ctrl || hotkey.alt,
            option: hotkey.win || hotkey.alt,
            shift: hotkey.shift,
            command: hotkey.alt,
        }
    }
}

/// The name a binding gives the key with this macOS key code by its place
/// on the keyboard (the US layout's), through its Windows virtual-key code.
pub fn key_name(code: u16) -> Option<String> {
    gevrek_model::keys::vk_to_key_name(place_key(code)?)
}

/// The virtual-key code of the key at a place on a Mac keyboard
/// (`kVK_*` of Carbon's Events.h), as the US layout has it.
pub fn place_key(code: u16) -> Option<u16> {
    Some(match code {
        0x00 => b'A' as u16,
        0x01 => b'S' as u16,
        0x02 => b'D' as u16,
        0x03 => b'F' as u16,
        0x04 => b'H' as u16,
        0x05 => b'G' as u16,
        0x06 => b'Z' as u16,
        0x07 => b'X' as u16,
        0x08 => b'C' as u16,
        0x09 => b'V' as u16,
        0x0B => b'B' as u16,
        0x0C => b'Q' as u16,
        0x0D => b'W' as u16,
        0x0E => b'E' as u16,
        0x0F => b'R' as u16,
        0x10 => b'Y' as u16,
        0x11 => b'T' as u16,
        0x12 => b'1' as u16,
        0x13 => b'2' as u16,
        0x14 => b'3' as u16,
        0x15 => b'4' as u16,
        0x16 => b'6' as u16,
        0x17 => b'5' as u16,
        0x18 => 0xBB, // =
        0x19 => b'9' as u16,
        0x1A => b'7' as u16,
        0x1B => 0xBD, // -
        0x1C => b'8' as u16,
        0x1D => b'0' as u16,
        0x1E => 0xDD, // ]
        0x1F => b'O' as u16,
        0x20 => b'U' as u16,
        0x21 => 0xDB, // [
        0x22 => b'I' as u16,
        0x23 => b'P' as u16,
        0x24 => 0x0D, // Return
        0x25 => b'L' as u16,
        0x26 => b'J' as u16,
        0x27 => 0xDE, // '
        0x28 => b'K' as u16,
        0x29 => 0xBA, // ;
        0x2A => 0xDC, // \
        0x2B => 0xBC, // ,
        0x2C => 0xBF, // /
        0x2D => b'N' as u16,
        0x2E => b'M' as u16,
        0x2F => 0xBE, // .
        0x30 => 0x09, // Tab
        0x31 => 0x20, // Space
        0x32 => 0xC0, // `
        0x33 => 0x08, // Delete (backspace)
        0x35 => 0x1B, // Escape
        0x40 => 0x80, // F17
        0x48 => 0xAF, // volume up
        0x49 => 0xAE, // volume down
        0x4A => 0xAD, // mute
        0x4F => 0x81, // F18
        0x50 => 0x82, // F19
        0x5A => 0x83, // F20
        0x60 => 0x74, // F5
        0x61 => 0x75, // F6
        0x62 => 0x76, // F7
        0x63 => 0x72, // F3
        0x64 => 0x77, // F8
        0x65 => 0x78, // F9
        0x67 => 0x7A, // F11
        0x69 => 0x7C, // F13
        0x6A => 0x7F, // F16
        0x6B => 0x7D, // F14
        0x6D => 0x79, // F10
        0x6F => 0x7B, // F12
        0x71 => 0x7E, // F15
        0x72 => 0x2D, // Help (Insert)
        0x73 => 0x24, // Home
        0x74 => 0x21, // Page Up
        0x75 => 0x2E, // forward Delete
        0x76 => 0x73, // F4
        0x77 => 0x23, // End
        0x78 => 0x71, // F2
        0x79 => 0x22, // Page Down
        0x7A => 0x70, // F1
        0x7B => 0x25, // Left
        0x7C => 0x27, // Right
        0x7D => 0x28, // Down
        0x7E => 0x26, // Up
        _ => return None,
    })
}

/// The bindings a key hits with what is held: their ids (of those with
/// this trigger), and whether one of them (of any trigger) keeps the key
/// from the program in front.
pub fn matching(
    bindings: &[(i32, Hotkey)],
    key: &str,
    held: Held,
    trigger: BindingTrigger,
) -> (Vec<i32>, bool) {
    matching_on(bindings, BindingDevice::Keyboard, key, held, trigger)
}

/// As `matching`, for a key or a mouse button (or wheel step).
pub fn matching_on(
    bindings: &[(i32, Hotkey)],
    device: BindingDevice,
    key: &str,
    held: Held,
    trigger: BindingTrigger,
) -> (Vec<i32>, bool) {
    let mut ids = Vec::new();
    let mut consume = false;
    for (id, hotkey) in bindings {
        let hit = hotkey.device == device
            && gevrek_model::keys::canonical_key_name(&hotkey.key) == key
            && Held::of_binding(hotkey) == held;
        if !hit {
            continue;
        }
        consume |= hotkey.consume;
        if hotkey.trigger == trigger {
            ids.push(*id);
        }
    }
    (ids, consume)
}

/// The name bindings give a mouse button (as on the other systems), by the
/// window server's number of it.
pub fn mouse_button_name(number: u32) -> Option<&'static str> {
    Some(match number {
        0 => "LEFT",
        1 => "RIGHT",
        2 => "MIDDLE",
        3 => "X1",
        4 => "X2",
        _ => return None,
    })
}

/// A held key or button starts repeating its binding after this long, then
/// at this rate (as on Linux). Keys repeat as macOS repeats them; this is
/// for the mouse's buttons, which macOS does not repeat.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(40);

/// A window is not dragged smaller than this.
pub const MIN_DRAG_SIZE: i32 = 80;

/// What a drag of a window with the mouse did, for the runtime to carry
/// out (the tap only notes it: it must answer at once).
#[derive(Debug, Clone, PartialEq)]
pub enum DragEvent {
    /// The button went down on a window gevrek manages, with the drag
    /// modifier held: the left button moves it, the right resizes it.
    Start {
        window: u32,
        resize: bool,
        at: (i32, i32),
    },
    /// The pointer is here now.
    To((i32, i32)),
    End,
}

/// A key or button that is down and that a long press (or, for a button, a
/// repeat) binding waits on.
#[derive(Debug, Clone)]
struct Down {
    /// The key code, or `BUTTON + number` for a mouse button.
    code: u32,
    device: BindingDevice,
    name: String,
    held: Held,
    since: Instant,
    last_repeat: Option<Instant>,
    /// Long-press bindings (by id) that ran already.
    long_fired: Vec<i32>,
}

/// Mouse buttons are kept apart from key codes by this.
const BUTTON: u32 = 0x1000;

/// What the tap shares with the runtime (both on the main thread).
#[derive(Default)]
pub struct Shared {
    /// The hotkeys that are live.
    pub bindings: Vec<(i32, Hotkey)>,
    /// Paused: no key hits a binding.
    pub paused: bool,
    /// The keys that hit bindings since the runtime last looked.
    pub hits: Vec<HotkeyEvent>,
    /// The windows gevrek manages and shows now, by window server id: a
    /// drag starts only on one of them.
    pub managed: HashSet<u32>,
    /// What, held alone, makes the left button move a window and the
    /// right one resize it; `None`: the mouse does not drag windows.
    pub drag_modifier: Option<Held>,
    /// What the drags did since the runtime last looked.
    pub drags: Vec<DragEvent>,
    /// Keys held whose press was kept from the programs: their repeats
    /// and their release are kept too.
    swallowed: HashSet<u16>,
    /// The same of mouse buttons.
    swallowed_buttons: HashSet<u32>,
    /// The button that holds a drag.
    dragging: Option<u32>,
    held: Vec<Down>,
    /// What the layout in use makes of the keys.
    layout: crate::layout::Layout,
}

impl Shared {
    /// What is held down that a long press or a button's repeat binding
    /// waits on, run when it is due. Called every pass; `now` is given so
    /// that tests can move time.
    pub fn tick(&mut self, now: Instant) {
        if self.paused {
            return;
        }
        for index in 0..self.held.len() {
            let down = self.held[index].clone();
            let down_for = now.saturating_duration_since(down.since);
            let hits = |hotkey: &Hotkey| {
                hotkey.device == down.device
                    && gevrek_model::keys::canonical_key_name(&hotkey.key) == down.name
                    && Held::of_binding(hotkey) == down.held
            };
            let long: Vec<i32> = self
                .bindings
                .iter()
                .filter(|(id, hotkey)| {
                    hits(hotkey)
                        && hotkey.trigger == BindingTrigger::LongPress
                        && down_for.as_millis() >= u128::from(hotkey.long_press_ms)
                        && !down.long_fired.contains(id)
                })
                .map(|(id, _)| *id)
                .collect();
            if !long.is_empty() {
                self.held[index].long_fired.extend(long.iter().copied());
                let event = hotkey_event(long, &down, BindingTrigger::LongPress, down_for);
                self.hits.push(event);
            }
            if down.device == BindingDevice::Mouse {
                let next = match down.last_repeat {
                    None => down.since + REPEAT_DELAY,
                    Some(last) => last + REPEAT_INTERVAL,
                };
                if now >= next {
                    let repeat: Vec<i32> = self
                        .bindings
                        .iter()
                        .filter(|(_, hotkey)| hits(hotkey) && hotkey.trigger == BindingTrigger::Repeat)
                        .map(|(id, _)| *id)
                        .collect();
                    if !repeat.is_empty() {
                        self.held[index].last_repeat = Some(now);
                        let event = hotkey_event(repeat, &down, BindingTrigger::Repeat, down_for);
                        self.hits.push(event);
                    }
                }
            }
        }
    }

    /// Remember a key or button that went down, where a long press (or a
    /// button's repeat) binding waits on it.
    fn hold(&mut self, code: u32, device: BindingDevice, name: &str, held: Held) {
        let waits = self.bindings.iter().any(|(_, hotkey)| {
            hotkey.device == device
                && gevrek_model::keys::canonical_key_name(&hotkey.key) == name
                && Held::of_binding(hotkey) == held
                && (hotkey.trigger == BindingTrigger::LongPress
                    || (device == BindingDevice::Mouse && hotkey.trigger == BindingTrigger::Repeat))
        });
        if waits && !self.held.iter().any(|down| down.code == code) {
            self.held.push(Down {
                code,
                device,
                name: name.to_string(),
                held,
                since: Instant::now(),
                last_repeat: None,
                long_fired: Vec::new(),
            });
        }
    }

    fn release(&mut self, code: u32) {
        self.held.retain(|down| down.code != code);
    }

    /// Tell the runtime of bindings a key or button hit.
    fn hit(&mut self, ids: Vec<i32>, device: BindingDevice, key: &str, held: Held, trigger: BindingTrigger) {
        if ids.is_empty() {
            return;
        }
        let down = Down {
            code: 0,
            device,
            name: key.to_string(),
            held,
            since: Instant::now(),
            last_repeat: None,
            long_fired: Vec::new(),
        };
        self.hits.push(hotkey_event(ids, &down, trigger, Duration::ZERO));
    }
}

/// A binding's event, its modifiers as the binding has them (Alt is
/// Control, Option and Command held together).
fn hotkey_event(ids: Vec<i32>, down: &Down, trigger: BindingTrigger, held_for: Duration) -> HotkeyEvent {
    let held = down.held;
    let alt = held.command && held.control && held.option;
    HotkeyEvent {
        registration_ids: ids,
        key: down.name.clone(),
        ctrl: held.control && !alt,
        alt,
        shift: held.shift,
        win: held.option && !alt,
        device: down.device,
        trigger,
        held_ms: held_for.as_millis() as u32,
    }
}

/// The event tap, while it is installed.
pub struct KeyTap {
    port: ffi::CFTypeRef,
    source: ffi::CFTypeRef,
    run_loop: ffi::CFTypeRef,
    /// Kept while the tap is: it holds a pointer to it.
    _shared: Rc<RefCell<Shared>>,
}

const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const LEFT_DRAGGED: u32 = 6;
const RIGHT_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const SCROLL_WHEEL: u32 = 22;
const OTHER_DOWN: u32 = 25;
const OTHER_UP: u32 = 26;
const BUTTON_NUMBER_FIELD: u32 = 3;
const SCROLL_DELTA_FIELD: u32 = 11;
const SCROLL_CONTINUOUS_FIELD: u32 = 88;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const KEYCODE_FIELD: u32 = 9;
const AUTOREPEAT_FIELD: u32 = 8;

impl KeyTap {
    /// Take the keyboard's keys on this thread's run loop. Fails where
    /// gevrek may not (Privacy & Security, Accessibility or Input
    /// Monitoring).
    pub fn install(shared: Rc<RefCell<Shared>>) -> anyhow::Result<Self> {
        let user_info = Rc::as_ptr(&shared) as *mut c_void;
        let mask = [
            KEY_DOWN,
            KEY_UP,
            LEFT_DOWN,
            LEFT_UP,
            RIGHT_DOWN,
            RIGHT_UP,
            LEFT_DRAGGED,
            RIGHT_DRAGGED,
            SCROLL_WHEEL,
            OTHER_DOWN,
            OTHER_UP,
        ]
        .iter()
        .fold(0u64, |mask, kind| mask | (1u64 << kind));
        // SAFETY: a session tap at the head, that may change events, with a
        // callback and a pointer to the boxed state it is kept with.
        let port = unsafe {
            ffi::CGEventTapCreate(1, 0, 0, mask, tap_callback, user_info)
        };
        if port.is_null() {
            anyhow::bail!(
                "the keyboard cannot be watched: allow gevrek in System Settings, \
                 Privacy & Security, Accessibility"
            );
        }
        // SAFETY: the port is valid; its source is owned (a Create call) and
        // added to this thread's run loop.
        let (source, run_loop) = unsafe {
            let source = ffi::CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
            let run_loop = ffi::CFRunLoopGetCurrent();
            ffi::CFRunLoopAddSource(run_loop, source, ffi::kCFRunLoopDefaultMode);
            ffi::CGEventTapEnable(port, true);
            (source, run_loop)
        };
        Ok(Self {
            port,
            source,
            run_loop,
            _shared: shared,
        })
    }
}

impl Drop for KeyTap {
    fn drop(&mut self) {
        // SAFETY: the tap and its source were made in `install` on this
        // thread; once removed and released no callback can come.
        unsafe {
            ffi::CGEventTapEnable(self.port, false);
            ffi::CFRunLoopRemoveSource(self.run_loop, self.source, ffi::kCFRunLoopDefaultMode);
            ffi::CFRelease(self.source);
            ffi::CFRelease(self.port);
        }
    }
}

extern "C" fn tap_callback(
    _proxy: ffi::CFTypeRef,
    kind: u32,
    event: ffi::CFTypeRef,
    user_info: *mut c_void,
) -> ffi::CFTypeRef {
    if user_info.is_null() {
        return event;
    }
    // SAFETY: user_info is the state the tap was made with, kept alive
    // while the tap is (see `Drop`); the callback runs on its thread.
    let shared = unsafe { &*(user_info as *const RefCell<Shared>) };
    if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER_INPUT {
        // The window server switches a tap off that answered too slowly:
        // it is switched on again. (The port is not at hand here; the
        // runtime does it, `KeyTap::keep_on`.)
        log::warn!("the keyboard tap was switched off by the system");
        return event;
    }
    let Ok(mut shared) = shared.try_borrow_mut() else {
        return event;
    };
    // SAFETY: a valid event, read only.
    let flags = unsafe { ffi::CGEventGetFlags(event) };
    let held = Held::of_flags(flags);
    let keep = match kind {
        KEY_DOWN | KEY_UP => key_event(&mut shared, event, kind == KEY_UP, held),
        LEFT_DOWN | RIGHT_DOWN | OTHER_DOWN | LEFT_UP | RIGHT_UP | OTHER_UP => {
            let number = match kind {
                LEFT_DOWN | LEFT_UP => 0,
                RIGHT_DOWN | RIGHT_UP => 1,
                // SAFETY: a valid mouse event, read only.
                _ => unsafe { ffi::CGEventGetIntegerValueField(event, BUTTON_NUMBER_FIELD) as u32 },
            };
            let up = matches!(kind, LEFT_UP | RIGHT_UP | OTHER_UP);
            button_event(&mut shared, event, number, up, held)
        }
        LEFT_DRAGGED | RIGHT_DRAGGED => {
            if shared.dragging.is_some() {
                let at = location(event);
                shared.drags.push(DragEvent::To(at));
                true
            } else {
                false
            }
        }
        SCROLL_WHEEL => wheel_event(&mut shared, event, held),
        _ => false,
    };
    if keep {
        std::ptr::null()
    } else {
        event
    }
}

/// Where the event happened, in points from the top-left of the main
/// display.
fn location(event: ffi::CFTypeRef) -> (i32, i32) {
    // SAFETY: a valid event, read only.
    let at = unsafe { ffi::CGEventGetLocation(event) };
    (at.x.round() as i32, at.y.round() as i32)
}

/// A key: its bindings. Returns whether it is kept from the programs.
fn key_event(shared: &mut Shared, event: ffi::CFTypeRef, up: bool, held: Held) -> bool {
    // SAFETY: a valid key event, read only.
    let (code, repeat) = unsafe {
        (
            ffi::CGEventGetIntegerValueField(event, KEYCODE_FIELD) as u16,
            ffi::CGEventGetIntegerValueField(event, AUTOREPEAT_FIELD) != 0,
        )
    };
    let trigger = match (up, repeat) {
        (true, _) => BindingTrigger::Release,
        (_, true) => BindingTrigger::Repeat,
        _ => BindingTrigger::Press,
    };
    // A key whose press was kept: its repeats and release are kept too.
    let was_swallowed = shared.swallowed.contains(&code);
    if up {
        shared.swallowed.remove(&code);
        shared.release(u32::from(code));
    }
    if trigger == BindingTrigger::Press {
        shared.layout.refresh();
    }
    let Some(key) = shared
        .layout
        .virtual_key(code)
        .and_then(gevrek_model::keys::vk_to_key_name)
    else {
        return was_swallowed;
    };
    if shared.paused {
        return was_swallowed;
    }
    let (ids, consume) = matching(&shared.bindings, &key, held, trigger);
    shared.hit(ids, BindingDevice::Keyboard, &key, held, trigger);
    if trigger == BindingTrigger::Press {
        shared.hold(u32::from(code), BindingDevice::Keyboard, &key, held);
        if consume {
            shared.swallowed.insert(code);
        }
    }
    consume || was_swallowed
}

/// A mouse button: a drag of a window, or its bindings. Returns whether it
/// is kept from the programs.
fn button_event(shared: &mut Shared, event: ffi::CFTypeRef, number: u32, up: bool, held: Held) -> bool {
    if up {
        shared.release(BUTTON + number);
        if shared.dragging == Some(number) {
            shared.dragging = None;
            shared.drags.push(DragEvent::End);
            return true;
        }
    } else if !shared.paused
        && shared.dragging.is_none()
        && number <= 1
        && shared.drag_modifier == Some(held)
    {
        let at = location(event);
        if let Some(window) = managed_window_at(&shared.managed, at) {
            shared.dragging = Some(number);
            shared.drags.push(DragEvent::Start {
                window,
                resize: number == 1,
                at,
            });
            return true;
        }
    }
    let was_swallowed = up && shared.swallowed_buttons.remove(&number);
    let Some(name) = mouse_button_name(number) else {
        return was_swallowed;
    };
    if shared.paused {
        return was_swallowed;
    }
    let trigger = if up { BindingTrigger::Release } else { BindingTrigger::Press };
    let (ids, consume) = matching_on(&shared.bindings, BindingDevice::Mouse, name, held, trigger);
    shared.hit(ids, BindingDevice::Mouse, name, held, trigger);
    if !up {
        shared.hold(BUTTON + number, BindingDevice::Mouse, name, held);
        if consume {
            shared.swallowed_buttons.insert(number);
        }
    }
    consume || was_swallowed
}

/// A step of a mouse's wheel: its bindings (`WHEEL_UP`, `WHEEL_DOWN`).
/// The touchpad's scrolling, which is continuous, is left to the programs:
/// a swipe would be many steps.
fn wheel_event(shared: &mut Shared, event: ffi::CFTypeRef, held: Held) -> bool {
    // SAFETY: a valid scroll event, read only.
    let (delta, continuous) = unsafe {
        (
            ffi::CGEventGetIntegerValueField(event, SCROLL_DELTA_FIELD),
            ffi::CGEventGetIntegerValueField(event, SCROLL_CONTINUOUS_FIELD) != 0,
        )
    };
    if continuous || delta == 0 || shared.paused {
        return false;
    }
    let name = if delta > 0 { "WHEEL_UP" } else { "WHEEL_DOWN" };
    let (ids, consume) =
        matching_on(&shared.bindings, BindingDevice::Mouse, name, held, BindingTrigger::Press);
    shared.hit(ids, BindingDevice::Mouse, name, held, BindingTrigger::Press);
    consume
}

/// The window gevrek manages that is in front at a place on the screen:
/// `None` where the window in front there is another (one of a program
/// gevrek leaves alone, a panel) or there is none.
fn managed_window_at(managed: &HashSet<u32>, (x, y): (i32, i32)) -> Option<u32> {
    let own = std::process::id() as i32;
    crate::window_list::server_windows(true)
        .into_iter()
        .filter(|window| window.layer == crate::window_list::NORMAL_LAYER && window.pid != own)
        .find(|window| {
            let b = window.bounds;
            x >= b.x && x < b.x + b.width && y >= b.y && y < b.y + b.height
        })
        .map(|window| window.id)
        .filter(|id| managed.contains(id))
}

impl KeyTap {
    /// Switch the tap on again if the system switched it off (it does to a
    /// tap that answered too slowly once).
    pub fn keep_on(&self) {
        // SAFETY: a valid tap.
        unsafe {
            if !ffi::CGEventTapIsEnabled(self.port) {
                log::warn!("the keyboard tap was off: switched on again");
                ffi::CGEventTapEnable(self.port, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkey(key: &str, win: bool, alt: bool, ctrl: bool, consume: bool) -> Hotkey {
        Hotkey {
            consume,
            ..Hotkey::new(key.to_string(), ctrl, alt, false, win)
        }
    }

    const OPTION: Held = Held {
        control: false,
        option: true,
        shift: false,
        command: false,
    };

    #[test]
    fn keys_are_named_as_bindings_name_them() {
        assert_eq!(key_name(0x00).as_deref(), Some("A"));
        assert_eq!(key_name(0x12).as_deref(), Some("1"));
        assert_eq!(key_name(0x7B).as_deref(), Some("LEFT"));
        assert_eq!(key_name(0x24).as_deref(), Some("ENTER"));
        assert_eq!(key_name(0x7A).as_deref(), Some("F1"));
        assert_eq!(key_name(0x69).as_deref(), Some("F13"));
        assert_eq!(key_name(0x21).as_deref(), Some("LEFTBRACKET"));
        assert_eq!(key_name(0x3A), None); // Option itself
    }

    #[test]
    fn win_is_option_and_alt_is_control_option_command() {
        let bindings = vec![
            (1, hotkey("H", true, false, false, true)),
            (2, hotkey("H", false, true, false, true)),
            (3, hotkey("H", true, false, true, true)),
        ];
        let option_control = Held {
            control: true,
            ..OPTION
        };
        let all_three = Held {
            command: true,
            ..option_control
        };
        assert_eq!(matching(&bindings, "H", OPTION, BindingTrigger::Press), (vec![1], true));
        // Win with Control is not Alt.
        assert_eq!(
            matching(&bindings, "H", option_control, BindingTrigger::Press),
            (vec![3], true)
        );
        assert_eq!(matching(&bindings, "H", all_three, BindingTrigger::Press), (vec![2], true));
        assert_eq!(
            matching(&bindings, "H", Held::default(), BindingTrigger::Press),
            (vec![], false)
        );
    }

    #[test]
    fn commands_shortcuts_stay_the_programs() {
        let bindings = vec![
            (1, hotkey("C", true, false, false, true)),
            (2, hotkey("V", false, false, false, true)),
            (3, hotkey("Q", false, true, false, true)),
        ];
        let command = Held {
            command: true,
            ..Held::default()
        };
        let option_command = Held {
            command: true,
            ..OPTION
        };
        for key in ["C", "V", "Q"] {
            assert_eq!(matching(&bindings, key, command, BindingTrigger::Press), (vec![], false));
            assert_eq!(
                matching(&bindings, key, option_command, BindingTrigger::Press),
                (vec![], false)
            );
        }
    }

    #[test]
    fn a_consuming_binding_keeps_its_key_whatever_its_trigger() {
        let mut release = hotkey("Q", true, false, false, true);
        release.trigger = BindingTrigger::Release;
        let bindings = vec![(5, release)];
        // The press hits nothing, and is kept: its release is the binding's.
        assert_eq!(matching(&bindings, "Q", OPTION, BindingTrigger::Press), (vec![], true));
        assert_eq!(matching(&bindings, "Q", OPTION, BindingTrigger::Release), (vec![5], true));
    }

    /// Alt is held as Option with Control, which is also Win with Control:
    /// two standard bindings on the same keys held so would both fire.
    #[test]
    fn no_two_standard_bindings_are_on_the_same_keys_on_a_mac() {
        let config = gevrek_model::config::Config::new();
        let live: Vec<_> = config
            .live_bindings()
            .into_iter()
            .filter(|binding| binding.enabled && binding.device == BindingDevice::Keyboard)
            .collect();
        let mut clashes = Vec::new();
        for (index, one) in live.iter().enumerate() {
            for other in &live[index + 1..] {
                let held = |binding: &gevrek_model::settings::BindingProfile| {
                    Held::of_binding(&Hotkey::new(
                        binding.key.clone(),
                        binding.ctrl,
                        binding.alt,
                        binding.shift,
                        binding.win,
                    ))
                };
                let same_keys = gevrek_model::keys::canonical_key_name(&one.key)
                    == gevrek_model::keys::canonical_key_name(&other.key)
                    && held(one) == held(other)
                    && one.trigger == other.trigger
                    && one.normalized_submap() == other.normalized_submap();
                let same_on_a_pc = (one.ctrl, one.alt, one.shift, one.win)
                    == (other.ctrl, other.alt, other.shift, other.win);
                if same_keys && !same_on_a_pc {
                    clashes.push(format!("'{}' and '{}' ({})", one.name, other.name, one.key));
                }
            }
        }
        assert!(clashes.is_empty(), "on the same keys on a Mac: {clashes:#?}");
    }

    fn mouse(key: &str, trigger: BindingTrigger, long_press_ms: u32) -> Hotkey {
        Hotkey {
            device: BindingDevice::Mouse,
            trigger,
            long_press_ms,
            ..Hotkey::new(key.to_string(), false, false, false, true)
        }
    }

    #[test]
    fn a_mouse_binding_is_not_a_keys_and_the_other_way_round() {
        let bindings = vec![
            (1, mouse("WHEEL_UP", BindingTrigger::Press, 0)),
            (2, hotkey("LEFT", true, false, false, false)),
        ];
        assert_eq!(
            matching_on(&bindings, BindingDevice::Mouse, "WHEEL_UP", OPTION, BindingTrigger::Press),
            (vec![1], true)
        );
        // `LEFT` is an arrow key on the keyboard and a button of the mouse.
        assert_eq!(
            matching_on(&bindings, BindingDevice::Mouse, "LEFT", OPTION, BindingTrigger::Press),
            (vec![], false)
        );
        assert_eq!(matching(&bindings, "LEFT", OPTION, BindingTrigger::Press), (vec![2], false));
        assert_eq!(mouse_button_name(0), Some("LEFT"));
        assert_eq!(mouse_button_name(2), Some("MIDDLE"));
        assert_eq!(mouse_button_name(9), None);
    }

    #[test]
    fn a_long_press_runs_once_when_it_is_long_enough() {
        let mut shared = Shared {
            bindings: vec![(7, {
                let mut hotkey = hotkey("J", true, false, false, false);
                hotkey.trigger = BindingTrigger::LongPress;
                hotkey.long_press_ms = 500;
                hotkey
            })],
            ..Shared::default()
        };
        shared.hold(0x26, BindingDevice::Keyboard, "J", OPTION);
        let since = shared.held[0].since;
        shared.tick(since + Duration::from_millis(300));
        assert!(shared.hits.is_empty());
        shared.tick(since + Duration::from_millis(520));
        shared.tick(since + Duration::from_millis(900));
        assert_eq!(shared.hits.len(), 1, "once");
        assert_eq!(shared.hits[0].registration_ids, vec![7]);
        assert_eq!(shared.hits[0].trigger, BindingTrigger::LongPress);
        assert!(shared.hits[0].win && !shared.hits[0].alt);
        // Released, it is not timed any more.
        shared.release(0x26);
        assert!(shared.held.is_empty());
    }

    #[test]
    fn a_held_button_repeats_after_a_while_then_steadily() {
        let mut shared = Shared {
            bindings: vec![(3, mouse("X1", BindingTrigger::Repeat, 0))],
            ..Shared::default()
        };
        shared.hold(BUTTON + 3, BindingDevice::Mouse, "X1", OPTION);
        let since = shared.held[0].since;
        shared.tick(since + Duration::from_millis(300));
        assert!(shared.hits.is_empty(), "not before the delay");
        shared.tick(since + Duration::from_millis(410));
        shared.tick(since + Duration::from_millis(430));
        shared.tick(since + Duration::from_millis(455));
        assert_eq!(shared.hits.len(), 2, "at the delay, then 40 ms after");
        // A key repeats as macOS repeats it: it is not timed here.
        let mut keys = Shared {
            bindings: vec![(4, {
                let mut hotkey = hotkey("K", true, false, false, false);
                hotkey.trigger = BindingTrigger::Repeat;
                hotkey
            })],
            ..Shared::default()
        };
        keys.hold(0x28, BindingDevice::Keyboard, "K", OPTION);
        assert!(keys.held.is_empty());
    }

    #[test]
    fn a_binding_written_with_another_name_of_the_key_hits() {
        let bindings = vec![(1, hotkey("Return", true, false, false, false))];
        assert_eq!(matching(&bindings, "ENTER", OPTION, BindingTrigger::Press), (vec![1], false));
    }
}
