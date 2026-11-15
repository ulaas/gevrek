//! What kind of window a window is, by what Windows says of it: a
//! program's own window, something that goes with one, or something that
//! only passes by.
//!
//! gevrek draws a border round the windows it manages. Round a program's
//! window that is right; round the small things programs and Windows put
//! on the screen besides it is not:
//!
//! - **What goes with a window** (`Kind::Beside`): a dialog (a window that
//!   belongs to another, or is of the dialogs' own class, or can be
//!   neither resized nor minimised: "Save as", "About", a question to
//!   answer). It is still gevrek's window (it is shown and hidden with its
//!   workspace) and has no border.
//! - **What passes by** (`Kind::Passing`): a window that stays above all
//!   others and has neither a title bar nor a frame to resize it by: a
//!   notification a program draws itself, a volume or brightness display, a
//!   tooltip-like popup. It is no window to gevrek at all: not laid out,
//!   not given the keyboard's border, not moved.
//!
//! Everything else is an ordinary window (`Kind::Ordinary`), including a
//! program without a title bar of the system's that can still be resized
//! (most programs that draw their own).

/// What a window is to gevrek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ordinary,
    Beside,
    Passing,
}

const WS_CAPTION: u32 = 0x00C0_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;

/// The kind of a window with these styles, this class, and an owner or
/// none.
pub fn kind_of(style: u32, extended: u32, class: &str, owned: bool) -> Kind {
    let resizable = style & WS_THICKFRAME != 0;
    // (A title bar is both of two bits; one alone is a thin edge.)
    let titled = style & WS_CAPTION == WS_CAPTION;
    if extended & WS_EX_TOPMOST != 0 && !resizable && !titled && !owned {
        return Kind::Passing;
    }
    if owned || class == "#32770" || (!resizable && style & WS_MINIMIZEBOX == 0) {
        return Kind::Beside;
    }
    Kind::Ordinary
}

/// The kind of a window on the screen.
pub fn of_window(window: isize) -> Kind {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindow, GetWindowLongW, GWL_EXSTYLE, GWL_STYLE, GW_OWNER,
    };
    let hwnd = HWND(window as *mut _);
    // SAFETY: plain questions about a window; one that is gone has no
    // styles, no class and no owner, and is an ordinary window to nobody.
    unsafe {
        let mut name = [0u16; 64];
        let length = GetClassNameW(hwnd, &mut name) as usize;
        let class = String::from_utf16_lossy(&name[..length.min(name.len())]);
        let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
        kind_of(
            GetWindowLongW(hwnd, GWL_STYLE) as u32,
            GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
            &class,
            owned,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS_POPUP: u32 = 0x8000_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    /// A program's window as Windows makes it: title bar, frame, both boxes.
    const OVERLAPPED: u32 = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

    #[test]
    fn a_programs_own_window_is_an_ordinary_one() {
        assert_eq!(kind_of(OVERLAPPED, 0, "Notepad", false), Kind::Ordinary);
        // One that draws its own title bar and can still be resized.
        assert_eq!(
            kind_of(WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX, 0, "Chrome_WidgetWin_1", false),
            Kind::Ordinary
        );
        // One of a fixed size that can be minimised (an old game, a small tool).
        assert_eq!(
            kind_of(WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 0, "Calc", false),
            Kind::Ordinary
        );
        // Kept above the others by the person: still its program's window.
        assert_eq!(kind_of(OVERLAPPED, WS_EX_TOPMOST, "Notepad", false), Kind::Ordinary);
    }

    #[test]
    fn what_goes_with_a_window_has_no_border() {
        // It belongs to another window.
        assert_eq!(kind_of(OVERLAPPED, 0, "Settings", true), Kind::Beside);
        // The dialogs' own class.
        assert_eq!(kind_of(WS_CAPTION | WS_SYSMENU, 0, "#32770", false), Kind::Beside);
        // It can be neither resized nor minimised.
        assert_eq!(kind_of(WS_CAPTION | WS_SYSMENU, 0, "AboutBox", false), Kind::Beside);
        assert_eq!(kind_of(WS_POPUP, 0, "Splash", false), Kind::Beside);
    }

    #[test]
    fn what_passes_by_is_no_window_to_gevrek() {
        // Above everything, with no title bar and no frame: a notification.
        assert_eq!(kind_of(WS_POPUP, WS_EX_TOPMOST, "Toast", false), Kind::Passing);
        // The same, belonging to a window: it goes with that window.
        assert_eq!(kind_of(WS_POPUP, WS_EX_TOPMOST, "Menu", true), Kind::Beside);
        // Above everything, with a title bar: a dialog that insists.
        assert_eq!(
            kind_of(WS_CAPTION | WS_SYSMENU, WS_EX_TOPMOST, "Ask", false),
            Kind::Beside
        );
    }
}
