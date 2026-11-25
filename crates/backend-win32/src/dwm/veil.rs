//! A window that opens on another display than the one with the keyboard
//! is not seen there.
//!
//! Windows opens a program's window where the program last had one, on
//! whichever display that was, and gevrek brings a window that is to be
//! tiled to the display with the keyboard (`adopt_new_window`). Between
//! the two the window was seen where it opened, for as long as the window
//! manager took to hear of it and to place it: a window that flashed on
//! one display and then stood on the other.
//!
//! Such a window is cloaked the moment Windows tells that it is shown
//! (nothing of a cloaked window is drawn), and uncloaked at the end of
//! the pass in which the window manager was told of it, when it stands
//! where it belongs. One that is left where it opened (a dialog, a window
//! that floats, one no rule takes) is uncloaked in that same pass, a few
//! thousandths of a second after it opened.
//!
//! Whatever happens to the pass, no window stays cloaked: each is
//! uncloaked when `LONGEST` is over, and all of them when the window
//! manager ends.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowLongW, IsIconic, IsWindow, GWL_EXSTYLE, GWL_STYLE, GW_OWNER,
};

/// The longest a window stays cloaked, whatever becomes of the pass that
/// was to uncloak it.
const LONGEST: Duration = Duration::from_millis(400);

const DWMWA_CLOAK: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(13);
const DWMWA_CLOAKED: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(14);

/// The display with the keyboard, as Windows names it (an `HMONITOR`);
/// 0 while it is not known.
static KEYBOARD_DISPLAY: AtomicIsize = AtomicIsize::new(0);

/// A window that is cloaked here.
struct Veiled {
    window: isize,
    since: Instant,
    /// The window manager's loop has taken what Windows told of it: it
    /// is uncloaked when that pass ends.
    taken: bool,
}

static VEILED: Mutex<Vec<Veiled>> = Mutex::new(Vec::new());

fn veiled() -> std::sync::MutexGuard<'static, Vec<Veiled>> {
    VEILED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The display with the keyboard is this one now.
pub fn set_keyboard_display(display: isize) {
    KEYBOARD_DISPLAY.store(display, Ordering::Relaxed);
}

/// What is known of a window that is shown for the first time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opening {
    pub style: u32,
    pub extended: u32,
    pub owned: bool,
    pub minimized: bool,
    pub cloaked: bool,
    /// The display it is on, and the one with the keyboard (0: not known).
    pub on: isize,
    pub keyboard: isize,
    /// The primary mouse button is held: a window that opens then is
    /// one the person pulls out of another (a tab), and is seen.
    pub button_down: bool,
}

const WS_CAPTION: u32 = 0x00C0_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// Whether a window that opens so is cloaked until it is placed: a
/// program's own window with a title bar (what is tiled; a menu, a hint,
/// a dialog of another window are not), on another display than the one
/// with the keyboard.
pub fn is_veiled_as_it_opens(opening: Opening) -> bool {
    opening.keyboard != 0
        && opening.on != 0
        && opening.on != opening.keyboard
        && opening.style & WS_CAPTION == WS_CAPTION
        && opening.style & WS_CHILD == 0
        && opening.extended & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
        && !opening.owned
        && !opening.minimized
        && !opening.cloaked
        && !opening.button_down
}

/// A window is shown that was not seen before: cloak it where
/// `is_veiled_as_it_opens` says so.
///
/// # Safety
/// Plain questions about a window that may be gone, and one request to
/// the compositor about it.
pub unsafe fn window_opens(hwnd: HWND) {
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut _,
        std::mem::size_of::<u32>() as u32,
    );
    let opening = Opening {
        style: GetWindowLongW(hwnd, GWL_STYLE) as u32,
        extended: GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
        owned: GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()),
        minimized: IsIconic(hwnd).as_bool(),
        cloaked: cloaked != 0,
        on: MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as isize,
        keyboard: KEYBOARD_DISPLAY.load(Ordering::Relaxed),
        button_down: GetAsyncKeyState(i32::from(VK_LBUTTON.0)) < 0,
    };
    if !is_veiled_as_it_opens(opening) {
        return;
    }
    // (Refused for a window of a program with more rights: it is seen
    // where it opens, as before.)
    if cloak(hwnd, true) {
        log::debug!(
            "{hwnd:?} opens on another display than the keyboard's: not seen until it is placed"
        );
        veiled().push(Veiled {
            window: hwnd.0 as isize,
            since: Instant::now(),
            taken: false,
        });
    }
}

unsafe fn cloak(hwnd: HWND, cloaked: bool) -> bool {
    let value: i32 = i32::from(cloaked);
    super::reach::dwm_set_window_attribute(
        hwnd,
        DWMWA_CLOAK,
        &value as *const i32 as *const _,
        std::mem::size_of::<i32>() as u32,
    )
    .is_ok()
}

/// Whether a window is cloaked here: to the window manager it is a
/// window like any other (one that another cloaked is none of its).
pub fn holds(hwnd: HWND) -> bool {
    veiled()
        .iter()
        .any(|veiled| veiled.window == hwnd.0 as isize)
}

/// The window manager's loop takes what Windows told so far: the windows
/// cloaked until now are uncloaked when this pass ends (`pass_ended`).
pub fn events_taken() {
    for veiled in veiled().iter_mut() {
        veiled.taken = true;
    }
}

/// A pass of the window manager's loop is over: the windows it was told
/// of stand where they belong, and are seen. So is one that has been
/// cloaked for `LONGEST`.
pub fn pass_ended() {
    let lifted: Vec<isize> = {
        let mut all = veiled();
        let (lifted, kept): (Vec<Veiled>, Vec<Veiled>) = std::mem::take(&mut *all)
            .into_iter()
            .partition(|veiled| veiled.taken || veiled.since.elapsed() >= LONGEST);
        *all = kept;
        lifted.into_iter().map(|veiled| veiled.window).collect()
    };
    lift(&lifted);
}

/// Every window that is cloaked here is seen: the window manager ends.
pub fn lift_all() {
    let all: Vec<isize> = std::mem::take(&mut *veiled())
        .into_iter()
        .map(|veiled| veiled.window)
        .collect();
    lift(&all);
}

fn lift(windows: &[isize]) {
    for window in windows {
        let hwnd = HWND(*window as *mut _);
        // SAFETY: a request about a window that may be gone.
        unsafe {
            if IsWindow(hwnd).as_bool() {
                cloak(hwnd, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: Opening = Opening {
        style: WS_CAPTION | 0x0004_0000,
        extended: 0,
        owned: false,
        minimized: false,
        cloaked: false,
        on: 2,
        keyboard: 1,
        button_down: false,
    };

    #[test]
    fn a_programs_window_that_opens_on_another_display_is_not_seen_there() {
        assert!(is_veiled_as_it_opens(APP));
    }

    #[test]
    fn what_stays_where_it_opens_is_seen_at_once() {
        // On the display with the keyboard; or that display is not known.
        assert!(!is_veiled_as_it_opens(Opening { on: 1, ..APP }));
        assert!(!is_veiled_as_it_opens(Opening { keyboard: 0, ..APP }));
        // A menu or a hint, a dialog of another window, a child.
        assert!(!is_veiled_as_it_opens(Opening {
            style: 0x8000_0000,
            ..APP
        }));
        assert!(!is_veiled_as_it_opens(Opening {
            extended: WS_EX_TOOLWINDOW,
            ..APP
        }));
        assert!(!is_veiled_as_it_opens(Opening {
            extended: WS_EX_NOACTIVATE,
            ..APP
        }));
        assert!(!is_veiled_as_it_opens(Opening { owned: true, ..APP }));
        assert!(!is_veiled_as_it_opens(Opening {
            style: APP.style | WS_CHILD,
            ..APP
        }));
        // Minimized, or cloaked by another (a virtual desktop).
        assert!(!is_veiled_as_it_opens(Opening {
            minimized: true,
            ..APP
        }));
        assert!(!is_veiled_as_it_opens(Opening {
            cloaked: true,
            ..APP
        }));
        // A tab pulled out of its window.
        assert!(!is_veiled_as_it_opens(Opening {
            button_down: true,
            ..APP
        }));
    }

    #[test]
    fn a_window_is_seen_when_the_pass_that_was_told_of_it_ends() {
        // (Windows that are none: nothing is asked of the system.)
        let before = veiled().len();
        veiled().push(Veiled {
            window: -7,
            since: Instant::now(),
            taken: false,
        });
        assert!(holds(HWND(-7isize as *mut _)));
        // The pass that ends had not taken what was told of it yet.
        pass_ended();
        assert!(holds(HWND(-7isize as *mut _)));
        events_taken();
        pass_ended();
        assert!(!holds(HWND(-7isize as *mut _)));
        // One that was never taken is seen when the longest time is over.
        veiled().push(Veiled {
            window: -8,
            since: Instant::now() - LONGEST,
            taken: false,
        });
        pass_ended();
        assert!(!holds(HWND(-8isize as *mut _)));
        assert_eq!(veiled().len(), before);
    }
}
