//! Every key for one program, for a moment: what the settings program
//! needs while the keys of a binding are recorded.
//!
//! The window manager takes its own bindings before a window sees them, and
//! Windows keeps the Windows key to itself: pressed while recording, Win+2
//! would show the second workspace and never reach the settings program. A
//! program may listen to the keyboard before both (a low-level hook; the
//! one set last is asked first), and that is what this does, only while
//! one of the program's own windows has the keyboard and only for as long
//! as [`EveryKey`] is kept. A key that is heard goes no further.
//!
//! Windows only; on Linux the compositor is asked instead (the settings
//! program does that itself, on its toolkit's connection).
#![cfg_attr(not(windows), allow(dead_code))]

/// A key that was pressed, with the modifiers held as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    /// The key, as Windows numbers it (a virtual-key code).
    pub key: u16,
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Which of the modifiers a key is, if it is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Win,
    Ctrl,
    Alt,
    Shift,
}

fn modifier_of(key: u16) -> Option<Modifier> {
    match key {
        0x5B | 0x5C => Some(Modifier::Win),
        0x11 | 0xA2 | 0xA3 => Some(Modifier::Ctrl),
        0x12 | 0xA4 | 0xA5 => Some(Modifier::Alt),
        0x10 | 0xA0 | 0xA1 => Some(Modifier::Shift),
        _ => None,
    }
}

/// The modifiers held, followed from key to key (a key that goes no
/// further is not among those the system says are held).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Held {
    win: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl Held {
    /// A key went down or up: the first key that is no modifier, as it
    /// goes down, is what was pressed.
    fn take(&mut self, key: u16, down: bool) -> Option<Heard> {
        match modifier_of(key) {
            Some(Modifier::Win) => self.win = down,
            Some(Modifier::Ctrl) => self.ctrl = down,
            Some(Modifier::Alt) => self.alt = down,
            Some(Modifier::Shift) => self.shift = down,
            None if down => {
                return Some(Heard {
                    key,
                    win: self.win,
                    ctrl: self.ctrl,
                    alt: self.alt,
                    shift: self.shift,
                })
            }
            None => {}
        }
        None
    }
}

#[cfg(windows)]
pub use self::windows::{
    modifier_held, press, press_chord, say_to_the_terminal_it_was_started_from, shift_held,
    EveryKey, OWN_MARK,
};

#[cfg(windows)]
mod windows {
    use super::{Heard, Held};
    use std::sync::Mutex;
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
        UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
    };

    struct Listening {
        held: Held,
        heard: Option<Heard>,
        /// The keys whose press went no further: their release does not
        /// either. (The release of a key that was pressed before, for
        /// another program, goes on: the system would take the key for
        /// held otherwise.)
        kept: Vec<u16>,
        wake: Box<dyn Fn() + Send>,
    }

    /// One at a time: a program records one binding at a time.
    static LISTENING: Mutex<Option<Listening>> = Mutex::new(None);

    /// Every key is this program's while this is kept and one of its
    /// windows has the keyboard.
    pub struct EveryKey {
        hook: isize,
    }

    impl EveryKey {
        /// Begin to listen. `wake` is called (on this thread) when a key
        /// was heard, so that who waits for it looks. The thread must be
        /// one that reads its messages, as a thread with windows does.
        /// `None` when Windows refuses, or when this is already kept.
        pub fn take(wake: impl Fn() + Send + 'static) -> Option<Self> {
            let down = |key: i32| unsafe { GetAsyncKeyState(key) } < 0;
            {
                let mut listening = LISTENING.lock().unwrap_or_else(|error| error.into_inner());
                if listening.is_some() {
                    return None;
                }
                *listening = Some(Listening {
                    // What is held already counts.
                    held: Held {
                        win: down(0x5B) || down(0x5C),
                        ctrl: down(0x11),
                        alt: down(0x12),
                        shift: down(0x10),
                    },
                    heard: None,
                    kept: Vec::new(),
                    wake: Box::new(wake),
                });
            }
            match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_heard), HINSTANCE::default(), 0) }
            {
                Ok(hook) => Some(Self {
                    hook: hook.0 as isize,
                }),
                Err(error) => {
                    log::warn!("the keyboard cannot be listened to: {error}");
                    *LISTENING.lock().unwrap_or_else(|error| error.into_inner()) = None;
                    None
                }
            }
        }

        /// The key that was pressed since this was last asked, if one was.
        pub fn heard(&self) -> Option<Heard> {
            LISTENING
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_mut()
                .and_then(|listening| listening.heard.take())
        }
    }

    impl Drop for EveryKey {
        fn drop(&mut self) {
            unsafe {
                let _ = UnhookWindowsHookEx(HHOOK(self.hook as *mut _));
            }
            *LISTENING.lock().unwrap_or_else(|error| error.into_inner()) = None;
        }
    }

    /// Whether Alt, Ctrl or the Windows key is held now, as the system has
    /// it: what a window switcher goes by, whose window is not told when
    /// a key is let go that was pressed before it came.
    pub fn modifier_held() -> bool {
        [0x12, 0x11, 0x5B, 0x5C]
            .into_iter()
            .any(|key| unsafe { GetAsyncKeyState(key) } < 0)
    }

    /// What marks a key as pressed by gevrek itself, for the system and
    /// its programs: the window manager, which has these keys bound (a
    /// key for "play" asks the shell, which presses the key for "play"),
    /// lets such a key through and does not answer it again. The same
    /// number is in `gevrek_win32::hotkey`.
    pub const OWN_MARK: usize = 0x6765_7672;

    /// Press and let go a key, as the keyboard does (for the keys that
    /// are meant for every program: play and pause, next, previous).
    pub fn press(key: u16) {
        press_chord(&[key]);
    }

    /// Press keys together, the first first, and let them go again.
    pub fn press_chord(keys: &[u16]) {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
            KEYEVENTF_KEYUP, VIRTUAL_KEY,
        };
        let event = |key: u16, flags: KEYBD_EVENT_FLAGS| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(key),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: OWN_MARK,
                },
            },
        };
        let mut events: Vec<INPUT> = keys
            .iter()
            .map(|key| event(*key, KEYBD_EVENT_FLAGS(0)))
            .collect();
        events.extend(keys.iter().rev().map(|key| event(*key, KEYEVENTF_KEYUP)));
        // SAFETY: plain key events.
        unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
    }

    /// A program with a window has no terminal of its own. Where it was
    /// started from one and its output goes nowhere else, it takes that
    /// terminal for what it says. (Nothing happens where there is no
    /// terminal, or the output is already sent somewhere.)
    pub fn say_to_the_terminal_it_was_started_from() {
        use windows::Win32::System::Console::{
            AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
            STD_OUTPUT_HANDLE,
        };
        // SAFETY: plain calls about this process's own handles.
        unsafe {
            let sent_somewhere = |which| {
                GetStdHandle(which).is_ok_and(|handle| !handle.is_invalid() && !handle.0.is_null())
            };
            if sent_somewhere(STD_OUTPUT_HANDLE) || sent_somewhere(STD_ERROR_HANDLE) {
                return;
            }
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

    /// Whether Shift is held now.
    pub fn shift_held() -> bool {
        let state = unsafe { GetAsyncKeyState(0x10) };
        state < 0
    }

    /// One of this program's windows has the keyboard.
    fn ours_in_front() -> bool {
        let mut process = 0u32;
        unsafe {
            GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process));
            process == GetCurrentProcessId()
        }
    }

    unsafe extern "system" fn key_heard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && ours_in_front() {
            let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if let Ok(mut listening) = LISTENING.try_lock() {
                if let Some(listening) = listening.as_mut() {
                    let key = event.vkCode as u16;
                    let ours = if down {
                        if !listening.kept.contains(&key) {
                            listening.kept.push(key);
                        }
                        true
                    } else {
                        let before = listening.kept.len();
                        listening.kept.retain(|kept| *kept != key);
                        listening.kept.len() != before
                    };
                    if let Some(heard) = listening.held.take(key, down) {
                        // The first one counts; who asked looks, and lets
                        // go of the keyboard.
                        if listening.heard.is_none() {
                            listening.heard = Some(heard);
                            (listening.wake)();
                        }
                    }
                    // The key is this program's: neither the window
                    // manager nor Windows (its Start menu) hears of it.
                    if ours {
                        return LRESULT(1);
                    }
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_key_that_is_no_modifier_is_heard_with_what_is_held() {
        let mut held = Held::default();
        // Win, Shift, then 2: one chord.
        assert_eq!(held.take(0x5B, true), None);
        assert_eq!(held.take(0xA0, true), None);
        assert_eq!(
            held.take(0x32, true),
            Some(Heard {
                key: 0x32,
                win: true,
                ctrl: false,
                alt: false,
                shift: true
            })
        );
        // A key let go is nothing pressed.
        assert_eq!(held.take(0x32, false), None);
        // Win let go before the key: no part of the chord.
        assert_eq!(held.take(0x5B, false), None);
        assert_eq!(held.take(0xA0, false), None);
        assert_eq!(
            held.take(0x4B, true),
            Some(Heard {
                key: 0x4B,
                win: false,
                ctrl: false,
                alt: false,
                shift: false
            })
        );
        // The right-hand keys are the same modifiers.
        assert_eq!(held.take(0xA3, true), None);
        assert_eq!(held.take(0xA5, true), None);
        let heard = held.take(0x0D, true).expect("a key");
        assert!(heard.ctrl && heard.alt && !heard.win && !heard.shift);
    }
}
