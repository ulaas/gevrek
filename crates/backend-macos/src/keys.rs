//! The keys gevrek's bindings are on, taken from the keyboard before any
//! program sees them (an event tap of the window server).
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
//! Keys are named by where they are on the keyboard (the US layout's
//! names), and through the same virtual-key codes as on the other systems
//! (`gevrek_model::keys`), so a binding is written the same everywhere.

use crate::ffi;
use gevrek_engine::input::{Hotkey, HotkeyEvent};
use gevrek_model::settings::{BindingDevice, BindingTrigger};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;
use std::rc::Rc;

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

/// The name a binding gives the key with this macOS key code (its place on
/// the keyboard), through its Windows virtual-key code.
pub fn key_name(code: u16) -> Option<String> {
    gevrek_model::keys::vk_to_key_name(virtual_key(code)?)
}

/// The virtual-key code of the key at a place on a Mac keyboard
/// (`kVK_*` of Carbon's Events.h).
fn virtual_key(code: u16) -> Option<u16> {
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
    let mut ids = Vec::new();
    let mut consume = false;
    for (id, hotkey) in bindings {
        let hit = hotkey.device == BindingDevice::Keyboard
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

/// What the tap shares with the runtime (both on the main thread).
#[derive(Default)]
pub struct Shared {
    /// The hotkeys that are live.
    pub bindings: Vec<(i32, Hotkey)>,
    /// Paused: no key hits a binding.
    pub paused: bool,
    /// The keys that hit bindings since the runtime last looked.
    pub hits: Vec<HotkeyEvent>,
    /// Keys held whose press was kept from the programs: their repeats
    /// and their release are kept too.
    swallowed: HashSet<u16>,
}

/// The event tap, while it is installed.
pub struct KeyTap {
    port: ffi::CFTypeRef,
    source: ffi::CFTypeRef,
    run_loop: ffi::CFTypeRef,
    /// Kept while the tap is: it holds a pointer to it.
    _shared: Rc<RefCell<Shared>>,
}

const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
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
        let mask = (1u64 << KEY_DOWN) | (1u64 << KEY_UP);
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
    if kind != KEY_DOWN && kind != KEY_UP {
        return event;
    }
    // SAFETY: a valid key event, read only.
    let (code, flags, repeat) = unsafe {
        (
            ffi::CGEventGetIntegerValueField(event, KEYCODE_FIELD) as u16,
            ffi::CGEventGetFlags(event),
            ffi::CGEventGetIntegerValueField(event, AUTOREPEAT_FIELD) != 0,
        )
    };
    let Ok(mut shared) = shared.try_borrow_mut() else {
        return event;
    };
    let trigger = match (kind, repeat) {
        (KEY_UP, _) => BindingTrigger::Release,
        (_, true) => BindingTrigger::Repeat,
        _ => BindingTrigger::Press,
    };
    // A key whose press was kept: its repeats and release are kept too.
    let was_swallowed = shared.swallowed.contains(&code);
    if kind == KEY_UP {
        shared.swallowed.remove(&code);
    }
    let Some(key) = key_name(code) else {
        return if was_swallowed { std::ptr::null() } else { event };
    };
    if shared.paused {
        return if was_swallowed { std::ptr::null() } else { event };
    }
    let held = Held::of_flags(flags);
    let (ids, consume) = matching(&shared.bindings, &key, held, trigger);
    if !ids.is_empty() {
        // (As the binding has them: Alt is the three held together.)
        let alt = held.command && held.control && held.option;
        shared.hits.push(HotkeyEvent {
            registration_ids: ids,
            key,
            ctrl: held.control && !alt,
            alt,
            shift: held.shift,
            win: held.option && !alt,
            device: BindingDevice::Keyboard,
            trigger,
            held_ms: 0,
        });
    }
    if consume && trigger == BindingTrigger::Press {
        shared.swallowed.insert(code);
    }
    if consume || was_swallowed {
        std::ptr::null()
    } else {
        event
    }
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

    #[test]
    fn a_binding_written_with_another_name_of_the_key_hits() {
        let bindings = vec![(1, hotkey("Return", true, false, false, false))];
        assert_eq!(matching(&bindings, "ENTER", OPTION, BindingTrigger::Press), (vec![1], false));
    }
}
