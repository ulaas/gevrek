//! The Windows key is gevrek's while gevrek runs
//! (`desktop.platform.windows.own_windows_key`), and windows are moved and
//! resized with it and the mouse (`desktop.platform.windows.drag_with_windows_key`).
//!
//! **The key.** Windows has shortcuts of its own on the Windows key (the
//! Start menu on the key alone, Win+E, Win+Tab, Win+arrows and many more),
//! and a desktop whose keys are gevrek's cannot have two programs answering
//! one key. So while the key is held, every other key is gevrek's: one that
//! is bound does what it is bound to, one that is not does nothing, and
//! neither reaches Windows. The key alone opens nothing: as it is let go,
//! Windows is told of a key that means nothing, which is how a program says
//! "the Windows key was used with something". (As it is let go, not as it
//! is pressed: what a hook sends reaches Windows before the key the hook
//! is looking at, so sent on the press it would come before the Windows
//! key and mean nothing; sent on the release it comes between the two.)
//!
//! The Windows key itself is not kept from Windows: only so does Windows
//! know when it is let go, whatever happens in between (a locked session, a
//! program with an administrator's rights in front). What cannot be taken
//! from Windows at all: Win+L (the lock) and Ctrl+Alt+Del.
//!
//! **The mouse.** With the Windows key held and no other modifier, the left
//! button on a window moves it and the right button resizes it from the
//! corner nearest the pointer, wherever on the window the pointer is. The
//! window manager hears of it as of any drag (a tiled window can be dropped
//! on another, or into a zone).
//!
//! Both are called from the input hooks (`hotkey.rs`), on their thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static OWN_KEY: AtomicBool = AtomicBool::new(false);
static DRAG_WITH_KEY: AtomicBool = AtomicBool::new(false);
/// A drag is going on (looked at with every move of the pointer).
static DRAGGING: AtomicBool = AtomicBool::new(false);

/// Take the two settings.
pub fn configure(own_key: bool, drag: bool) {
    OWN_KEY.store(own_key, Ordering::Relaxed);
    DRAG_WITH_KEY.store(drag, Ordering::Relaxed);
    // (The helper for elevated windows goes by the same two, where the
    // window manager has one: `elevated_helper`. In the helper itself
    // nobody is told.)
    crate::elevated_helper::client::set_windows_key(own_key, drag);
}

/// Whether every key pressed with the Windows key is gevrek's.
pub fn owns_key() -> bool {
    OWN_KEY.load(Ordering::Relaxed)
}

/// What is told to the window manager's loop about a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragEvent {
    Started(isize),
    Ended(isize),
}

static EVENTS: Mutex<Vec<DragEvent>> = Mutex::new(Vec::new());

/// Where and when a mouse button last went down: what tells which display
/// a press on the desktop itself was meant for (`pressed_just_now`).
static LAST_PRESS: Mutex<Option<(std::time::Instant, (i32, i32))>> = Mutex::new(None);

/// A mouse button went down at this point of the desktop.
pub fn note_press(at: (i32, i32)) {
    if let Ok(mut last) = LAST_PRESS.try_lock() {
        *last = Some((std::time::Instant::now(), at));
    }
    // (The window manager's loop looks at it: `desktop_pressed`.)
    gevrek_host::wake::now();
}

/// The press that `desktop_pressed` last told of.
static TOLD: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// The system's desktop, when a mouse button has just gone down on it and
/// that was not told yet: the window manager then takes the desktop as
/// having got the keyboard anew, on the display that was pressed on. (The
/// desktop keeps the foreground from one press to the next, so Windows
/// tells nothing of the second.)
pub fn desktop_pressed() -> Option<isize> {
    let (when, at) = (*LAST_PRESS.lock().unwrap_or_else(|error| error.into_inner()))?;
    let mut told = TOLD.lock().unwrap_or_else(|error| error.into_inner());
    if *told == Some(when) || when.elapsed() > std::time::Duration::from_millis(700) {
        return None;
    }
    *told = Some(when);
    system::desktop_at(at)
}

/// Where a mouse button went down within the last moment, if one did.
pub fn pressed_just_now() -> Option<(i32, i32)> {
    let last = *LAST_PRESS.lock().unwrap_or_else(|error| error.into_inner());
    last.filter(|(when, _)| when.elapsed() < std::time::Duration::from_millis(700))
        .map(|(_, at)| at)
}

/// The drags that began or ended since this was last asked.
/// Whether a window is being dragged with the Windows key by this
/// program's own hooks.
pub fn is_dragging() -> bool {
    DRAGGING.load(Ordering::Relaxed)
}

/// A drag that was made elsewhere (by the helper for elevated windows) is
/// told of as one made here.
pub fn push_event(event: DragEvent) {
    EVENTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(event);
    gevrek_host::wake::now();
}

pub fn take_events() -> Vec<DragEvent> {
    std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|error| error.into_inner()))
}

/// Which way a drag changes its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    Move,
    /// Which edges follow the pointer: -1 the left (or upper) one, 1 the
    /// right (or lower) one, for across and for down.
    Resize(i32, i32),
}

/// A rectangle of the desktop: left, top, right, bottom.
type Edges = (i32, i32, i32, i32);

/// The smallest a window is made by a drag.
const LEAST: (i32, i32) = (160, 100);

/// The edges that follow the pointer when a window is resized from where
/// it was pressed: those of the corner nearest to it.
fn corner(at: (i32, i32), window: Edges) -> (i32, i32) {
    let across = if at.0 * 2 < window.0 + window.2 { -1 } else { 1 };
    let down = if at.1 * 2 < window.1 + window.3 { -1 } else { 1 };
    (across, down)
}

/// Where a window is after the pointer went `by` from where the drag
/// began. A resize never makes it smaller than `LEAST`: the edge stops.
fn dragged(start: Edges, way: Way, by: (i32, i32)) -> Edges {
    let (mut left, mut top, mut right, mut bottom) = start;
    match way {
        Way::Move => {
            left += by.0;
            right += by.0;
            top += by.1;
            bottom += by.1;
        }
        Way::Resize(across, down) => {
            if across < 0 {
                left = (left + by.0).min(right - LEAST.0);
            } else {
                right = (right + by.0).max(left + LEAST.0);
            }
            if down < 0 {
                top = (top + by.1).min(bottom - LEAST.1);
            } else {
                bottom = (bottom + by.1).max(top + LEAST.1);
            }
        }
    }
    (left, top, right, bottom)
}

/// Whether a window of this class is one a drag may take: not the desktop,
/// the taskbar, or anything of gevrek's own.
fn may_be_dragged(class: &str) -> bool {
    !(class.starts_with("Gevrek")
        || matches!(
            class,
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "#32769"
        ))
}

pub use system::{cancel_drag, key_pressed, key_released, pointer_moved, pressed, released};

mod system {
    use super::*;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetWindowRect, IsIconic, IsWindowVisible, IsZoomed,
        SetWindowPos, WindowFromPoint, GA_ROOT, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER,
    };

    /// A drag that is going on.
    struct Drag {
        window: isize,
        /// The button that began it: true the left one.
        left_button: bool,
        way: Way,
        from: (i32, i32),
        start: Edges,
    }

    static DRAG: Mutex<Option<Drag>> = Mutex::new(None);

    /// The key that means nothing (no program gives it a meaning): pressed
    /// and let go while the Windows key is down, it tells Windows that the
    /// key was used with something, so letting it go opens no Start menu.
    const NOTHING: u16 = 0xE8;

    fn answer() {
        let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(NOTHING),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let both = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
        // SAFETY: two plain key events.
        unsafe { SendInput(&both, std::mem::size_of::<INPUT>() as i32) };
    }

    /// The system's desktop, if that is what lies at a point.
    pub fn desktop_at(at: (i32, i32)) -> Option<isize> {
        // SAFETY: plain questions about the window at the point.
        unsafe {
            let window = GetAncestor(WindowFromPoint(POINT { x: at.0, y: at.1 }), GA_ROOT);
            let mut name = [0u16; 32];
            let length = GetClassNameW(window, &mut name) as usize;
            let class = String::from_utf16_lossy(&name[..length.min(name.len())]);
            if !matches!(class.as_str(), "Progman" | "WorkerW") {
                return None;
            }
            let desktop = windows::Win32::UI::WindowsAndMessaging::GetShellWindow();
            (!desktop.is_invalid()).then_some(desktop.0 as isize)
        }
    }

    /// The Windows key went down (also told while it is held down).
    pub fn key_pressed() {}

    /// The Windows key is being let go: Windows has not heard of it yet.
    pub fn key_released() {
        if owns_key() {
            answer();
        }
    }

    /// A button went down at a point of the desktop while the Windows key
    /// and nothing else was held. `true`: a drag began, and the press is
    /// gevrek's (it goes no further).
    pub fn pressed(left_button: bool, at: (i32, i32)) -> bool {
        if !DRAG_WITH_KEY.load(Ordering::Relaxed) {
            return false;
        }
        // A button goes down while a drag is still held: the button that
        // held it was let go where no hook heard of it (behind Windows'
        // question for an administrator's rights, behind the lock, over
        // a window of a program with more rights). That drag is over.
        if DRAGGING.load(Ordering::Relaxed) {
            cancel_drag();
        }
        // SAFETY: plain questions about the window under the pointer.
        unsafe {
            let under = WindowFromPoint(POINT { x: at.0, y: at.1 });
            let window = GetAncestor(under, GA_ROOT);
            if window.is_invalid()
                || !IsWindowVisible(window).as_bool()
                || IsIconic(window).as_bool()
                || IsZoomed(window).as_bool()
            {
                return false;
            }
            let mut name = [0u16; 96];
            let length = GetClassNameW(window, &mut name) as usize;
            if !may_be_dragged(&String::from_utf16_lossy(&name[..length.min(name.len())])) {
                return false;
            }
            let mut rect = RECT::default();
            if GetWindowRect(window, &mut rect).is_err() {
                return false;
            }
            let start = (rect.left, rect.top, rect.right, rect.bottom);
            let way = if left_button {
                Way::Move
            } else {
                let (across, down) = corner(at, start);
                Way::Resize(across, down)
            };
            *DRAG.lock().unwrap_or_else(|error| error.into_inner()) = Some(Drag {
                window: window.0 as isize,
                left_button,
                way,
                from: at,
                start,
            });
            DRAGGING.store(true, Ordering::Relaxed);
            EVENTS
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(DragEvent::Started(window.0 as isize));
            gevrek_host::wake::now();
            // (The Windows key was used with something, whatever the
            // setting for the key says: it is down, so this comes after it.)
            answer();
            true
        }
    }

    /// The pointer moved: a window that is dragged follows.
    pub fn pointer_moved(at: (i32, i32)) {
        if !DRAGGING.load(Ordering::Relaxed) {
            return;
        }
        let Ok(drag) = DRAG.try_lock() else {
            return;
        };
        let Some(drag) = drag.as_ref() else {
            return;
        };
        let edges = dragged(drag.start, drag.way, (at.0 - drag.from.0, at.1 - drag.from.1));
        want(Wanted {
            window: drag.window,
            edges,
            only_moved: drag.way == Way::Move,
        });
    }

    /// Where a dragged window is to be next.
    #[derive(Clone, Copy)]
    struct Wanted {
        window: isize,
        edges: Edges,
        only_moved: bool,
    }

    /// The newest place asked for, until the thread that carries it out
    /// takes it. An older one that was not taken yet is dropped.
    static WANTED: (Mutex<Option<Wanted>>, std::sync::Condvar) =
        (Mutex::new(None), std::sync::Condvar::new());
    static CARRIER: std::sync::Once = std::sync::Once::new();

    /// How long a program is given to carry out one place before the next
    /// is sent all the same (it is asked twice: `settled`).
    const PATIENCE_MS: u32 = 150;

    /// Ask for a dragged window's next place.
    ///
    /// The pointer tells of itself up to a thousand times a second, and a
    /// window was sent a place for each: a program that takes a while over
    /// every new size (Unreal Editor lays out and draws all of itself anew)
    /// got them faster than it could carry them out, and went on working
    /// through places of long ago, far behind the pointer and for seconds
    /// after the button was let go. Now a window is sent one place at a
    /// time, the newest there is when it has done with the last.
    fn want(wanted: Wanted) {
        CARRIER.call_once(|| {
            let started = std::thread::Builder::new()
                .name("gevrek-drag".into())
                .spawn(carry);
            if let Err(error) = started {
                log::warn!("no thread for dragging windows with the Windows key: {error}");
            }
        });
        *WANTED.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(wanted);
        WANTED.1.notify_one();
    }

    /// Carry out the places asked for, one after the other, for good.
    fn carry() {
        loop {
            let wanted = {
                let mut next = WANTED.0.lock().unwrap_or_else(|error| error.into_inner());
                loop {
                    match next.take() {
                        Some(wanted) => break wanted,
                        None => {
                            next = WANTED
                                .1
                                .wait(next)
                                .unwrap_or_else(|error| error.into_inner());
                        }
                    }
                }
            };
            let (left, top, right, bottom) = wanted.edges;
            let mut flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
            if wanted.only_moved {
                flags |= SWP_NOSIZE;
            }
            let window = HWND(wanted.window as *mut _);
            // SAFETY: a window that is gone is no window; a program that
            // hangs is not waited for (the asking is not).
            unsafe {
                let _ = SetWindowPos(window, None, left, top, right - left, bottom - top, flags);
            }
            settled(window);
        }
    }

    /// Wait until a window's program has carried out what was posted to
    /// it, or for as long as there is patience. A message sent the plain
    /// way is answered when the program next looks for messages, before
    /// the posted ones that wait: the first answer says that it looks, and
    /// the second that it has been through what was posted meanwhile.
    fn settled(window: HWND) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_NULL,
        };
        for _ in 0..2 {
            // SAFETY: a message that means nothing, to a window that may
            // be gone; not waited for beyond the time given.
            let answered = unsafe {
                SendMessageTimeoutW(
                    window,
                    WM_NULL,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_ABORTIFHUNG,
                    PATIENCE_MS,
                    None,
                )
            };
            if answered.0 == 0 {
                return;
            }
        }
    }

    /// End a drag whose button was let go unheard (see `pressed`), or
    /// whose desktop went away under it. The window stays where it is,
    /// and the drag is told of as ended.
    pub fn cancel_drag() {
        if !DRAGGING.swap(false, Ordering::Relaxed) {
            return;
        }
        let held = DRAG
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(held) = held {
            log::debug!("a drag with the Windows key ends without its button's release");
            EVENTS
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(DragEvent::Ended(held.window));
            gevrek_host::wake::now();
        }
    }

    /// A button was let go. `true`: it was the one that held a drag, which
    /// ends here; the release is gevrek's too.
    pub fn released(left_button: bool) -> bool {
        if !DRAGGING.load(Ordering::Relaxed) {
            return false;
        }
        let mut drag = DRAG.lock().unwrap_or_else(|error| error.into_inner());
        match drag.as_ref() {
            Some(held) if held.left_button == left_button => {
                let window = held.window;
                *drag = None;
                DRAGGING.store(false, Ordering::Relaxed);
                EVENTS
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(DragEvent::Ended(window));
                gevrek_host::wake::now();
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Edges = (100, 100, 900, 700);

    #[test]
    fn a_move_takes_the_whole_window_along() {
        assert_eq!(dragged(WINDOW, Way::Move, (30, -20)), (130, 80, 930, 680));
        assert_eq!(dragged(WINDOW, Way::Move, (0, 0)), WINDOW);
    }

    #[test]
    fn a_resize_moves_the_edges_of_the_corner_that_was_nearest() {
        // Pressed in the lower right quarter: the right and the lower edge.
        assert_eq!(corner((800, 650), WINDOW), (1, 1));
        assert_eq!(
            dragged(WINDOW, Way::Resize(1, 1), (50, 40)),
            (100, 100, 950, 740)
        );
        // In the upper left quarter: the left and the upper edge, and the
        // others stay where they are.
        assert_eq!(corner((150, 120), WINDOW), (-1, -1));
        assert_eq!(
            dragged(WINDOW, Way::Resize(-1, -1), (50, 40)),
            (150, 140, 900, 700)
        );
        // Upper right.
        assert_eq!(corner((850, 120), WINDOW), (1, -1));
    }

    #[test]
    fn a_window_is_never_made_smaller_than_the_least() {
        let small = dragged(WINDOW, Way::Resize(1, 1), (-5000, -5000));
        assert_eq!(small, (100, 100, 100 + LEAST.0, 100 + LEAST.1));
        let from_the_left = dragged(WINDOW, Way::Resize(-1, -1), (5000, 5000));
        assert_eq!(from_the_left, (900 - LEAST.0, 700 - LEAST.1, 900, 700));
    }

    #[test]
    fn the_desktop_and_gevreks_own_windows_are_not_dragged() {
        assert!(may_be_dragged("CASCADIA_HOSTING_WINDOW_CLASS"));
        assert!(may_be_dragged("Chrome_WidgetWin_1"));
        for class in ["Progman", "WorkerW", "Shell_TrayWnd", "Gevrek_Shell_gevrek-panel", "Gevrek_Border"] {
            assert!(!may_be_dragged(class), "{class}");
        }
    }
}
