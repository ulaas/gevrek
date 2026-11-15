//! A window picked by a press on it (`DispatchCommand::PickWindow`).
//!
//! While it is on, the next press of the left mouse button is kept from
//! the window under the pointer (and its release too), and the window
//! manager's loop is told which window that was; Escape ends it with
//! nothing picked. Called from the input hooks (`hotkey.rs`), on their
//! thread; the loop takes what happened with `take`.
//!
//! (The pointer keeps its picture: Windows has one pointer picture per
//! window, and none that a program may set for all of them without
//! changing the person's own.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
/// The release of the button whose press picked is nobody's.
static RELEASE_KEPT: AtomicBool = AtomicBool::new(false);
/// The release of the Escape that ended it is nobody's.
static ESCAPE_KEPT: AtomicBool = AtomicBool::new(false);
/// What happened, for the loop: a window and its class, or nothing picked.
static DONE: Mutex<Option<Option<(isize, String)>>> = Mutex::new(None);

fn done(window: Option<(isize, String)>) {
    *DONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(window);
}

/// Begin picking; asked for again while it is on, it ends with nothing
/// picked.
pub fn toggle() {
    if ON.swap(false, Ordering::SeqCst) {
        done(None);
    } else {
        ON.store(true, Ordering::SeqCst);
    }
}

/// The left button went down at this place. `true`: the press picked a
/// window and is not passed on.
pub fn pressed(at: (i32, i32)) -> bool {
    if !ON.swap(false, Ordering::SeqCst) {
        return false;
    }
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, WindowFromPoint, GA_ROOT,
    };
    // SAFETY: these only read which window is where and what its class
    // is called, into a buffer of our own; a null window is answered
    // for none.
    let (window, class) = unsafe {
        let window = GetAncestor(WindowFromPoint(POINT { x: at.0, y: at.1 }), GA_ROOT);
        let mut name = [0u16; 256];
        let length = (GetClassNameW(window, &mut name).max(0) as usize).min(name.len());
        (window, String::from_utf16_lossy(&name[..length]))
    };
    done((!window.0.is_null()).then_some((window.0 as isize, class)));
    RELEASE_KEPT.store(true, Ordering::SeqCst);
    true
}

/// The left button went up. `true`: it is the release of the press that
/// picked, and is not passed on.
pub fn released() -> bool {
    RELEASE_KEPT.swap(false, Ordering::SeqCst)
}

/// Escape went down or up. `true`: it ended the picking (or is the
/// release of the one that did) and is not passed on.
pub fn escape(down: bool) -> bool {
    if !down {
        return ESCAPE_KEPT.swap(false, Ordering::SeqCst);
    }
    if !ON.swap(false, Ordering::SeqCst) {
        return false;
    }
    done(None);
    ESCAPE_KEPT.store(true, Ordering::SeqCst);
    true
}

/// What happened since it was last asked: `Some(Some((window, class)))`
/// picked, `Some(None)` ended with nothing.
pub fn take() -> Option<Option<(isize, String)>> {
    DONE.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}
