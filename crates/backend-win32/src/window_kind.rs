//! What kind of window a window is, by what Windows says of it: a
//! program's own window, something that goes with one, or something that
//! only passes by.
//!
//! Everything gevrek decides about a window from its structure (not from
//! a rule of the person's) is decided here, in one place, from the same
//! facts: whether it is gevrek's window at all, whether it gets a border,
//! and whether it floats when no rule says so.
//!
//! - **A program's own window** (`Kind::Ordinary`): laid out with the
//!   others and given a border. It floats instead where it cannot sensibly
//!   be given a tile's size (`floats`): a window with a title bar and no
//!   frame to resize it by (an installer, a small tool of a fixed size),
//!   and one its program keeps above all others (a picture-in-picture
//!   player).
//! - **A window of its own that is no program's main window**
//!   (`Kind::Beside`): one of the dialogs' own class, or one that can be
//!   neither resized nor minimised ("About", a splash screen, a question
//!   to answer, with no window it belongs to). It is gevrek's window (it
//!   is shown and hidden with its workspace), it floats, and it has no
//!   border.
//! - **What belongs to another window** (`Kind::Owned`): a dialog a window
//!   opened ("Save as", "Find", a question), a palette, a menu or a bubble
//!   of it. It is no window to gevrek: not laid out, not moved, not given a
//!   border. It goes where its window goes: Windows keeps it above that
//!   window and minimises it with it, and gevrek hides and shows it with
//!   it (`WindowManager::hide_window`).
//! - **What passes by** (`Kind::Passing`): a window that stays above all
//!   others and has neither a title bar nor a frame to resize it by (a
//!   notification a program draws itself, a volume or brightness display,
//!   a tooltip-like popup), and a window inside another (a control). No
//!   window to gevrek at all.

/// What a window is to gevrek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ordinary,
    Beside,
    Owned,
    Passing,
}

impl Kind {
    /// Whether gevrek takes the window as one of its own (in a workspace,
    /// tiled or floating).
    pub fn is_managed(self) -> bool {
        matches!(self, Kind::Ordinary | Kind::Beside)
    }
}

const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;

/// The class of the dialogs Windows makes itself.
const DIALOG_CLASS: &str = "#32770";

/// What Windows says of a window, as far as its kind depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts<'a> {
    pub style: u32,
    pub extended: u32,
    pub class: &'a str,
    /// It belongs to another window that is on the screen (or that gevrek
    /// has put away with its workspace): see `owner_on_screen`.
    pub owned: bool,
}

impl Facts<'_> {
    fn resizable(&self) -> bool {
        self.style & WS_THICKFRAME != 0
    }

    /// (A title bar is both of two bits; one alone is a thin edge.)
    fn titled(&self) -> bool {
        self.style & WS_CAPTION == WS_CAPTION
    }

    fn topmost(&self) -> bool {
        self.extended & WS_EX_TOPMOST != 0
    }

    /// A window that belongs to another and still asks for a place of its
    /// own in the taskbar is a window of its own (a program whose every
    /// window belongs to its first one).
    fn belongs_to_another(&self) -> bool {
        self.owned && self.extended & WS_EX_APPWINDOW == 0
    }
}

/// The kind of a window.
pub fn kind(facts: &Facts) -> Kind {
    if facts.style & WS_CHILD != 0 {
        return Kind::Passing;
    }
    if facts.belongs_to_another() {
        return Kind::Owned;
    }
    if facts.topmost() && !facts.resizable() && !facts.titled() {
        return Kind::Passing;
    }
    if facts.class == DIALOG_CLASS
        || (!facts.resizable() && facts.style & WS_MINIMIZEBOX == 0)
    {
        return Kind::Beside;
    }
    Kind::Ordinary
}

/// Whether a window floats when no rule of the person's says whether it
/// does: what is no program's main window, a window with a title bar that
/// cannot be resized (to give it a tile's size would stretch what its
/// program laid out for one size: an installer's pages), and a window its
/// program keeps above all others (it would cover its neighbours' tiles,
/// or be a tile above every floating window).
///
/// A window without a title bar and without a frame that can still be
/// minimised is tiled: programs that draw all of their window themselves
/// are often made so, and resize by their own edges.
pub fn floats(facts: &Facts) -> bool {
    match kind(facts) {
        Kind::Beside => true,
        Kind::Ordinary => facts.topmost() || (facts.titled() && !facts.resizable()),
        Kind::Owned | Kind::Passing => false,
    }
}

/// The kind of a window with these styles, this class, and an owner or
/// none.
pub fn kind_of(style: u32, extended: u32, class: &str, owned: bool) -> Kind {
    kind(&Facts {
        style,
        extended,
        class,
        owned,
    })
}

/// What Windows says of a window on the screen, and its class.
struct Read {
    style: u32,
    extended: u32,
    class: String,
    owned: bool,
}

impl Read {
    fn facts(&self) -> Facts<'_> {
        Facts {
            style: self.style,
            extended: self.extended,
            class: &self.class,
            owned: self.owned,
        }
    }
}

/// `window` is the window's handle as Windows has it (an `HWND`'s value,
/// not gevrek's own number for the window: `WindowHandle::to_hwnd`).
fn read(window: isize) -> Read {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowLongW, GWL_EXSTYLE, GWL_STYLE,
    };
    let hwnd = HWND(window as *mut _);
    // SAFETY: plain questions about a window; one that is gone has no
    // styles, no class and no owner.
    unsafe {
        let mut name = [0u16; 64];
        let length = GetClassNameW(hwnd, &mut name).max(0) as usize;
        Read {
            style: GetWindowLongW(hwnd, GWL_STYLE) as u32,
            extended: GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
            class: String::from_utf16_lossy(&name[..length.min(name.len())]),
            owned: owner_on_screen(window).is_some(),
        }
    }
}

/// The window this one belongs to, where that is a window a person can
/// see: one that is shown and has a size, or one gevrek itself has put
/// away with its workspace. `None` for a window of its own, and for one
/// that belongs to a window nobody sees (many programs make a hidden
/// window first and let every other belong to it: those are the program's
/// own windows).
pub fn owner_on_screen(window: isize) -> Option<isize> {
    use crate::dwm::HwndExt;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowRect, IsWindowVisible, GW_OWNER,
    };
    let hwnd = HWND(window as *mut _);
    // SAFETY: plain questions; a window that is gone has no owner.
    unsafe {
        let owner = GetWindow(hwnd, GW_OWNER).ok().filter(|owner| !owner.is_invalid())?;
        let seen = if IsWindowVisible(owner).as_bool() {
            let mut rect = RECT::default();
            GetWindowRect(owner, &mut rect).is_ok()
                && rect.right > rect.left
                && rect.bottom > rect.top
        } else {
            crate::dwm::hidden_journal::is_hidden(crate::dwm::WindowHandle::from_hwnd(owner))
        };
        seen.then_some(owner.0 as isize)
    }
}

/// The kind of a window on the screen, by its handle as Windows has it.
pub fn of_window(window: isize) -> Kind {
    kind(&read(window).facts())
}

/// Whether a window on the screen floats when no rule says (`floats`), by
/// its handle as Windows has it.
pub fn window_floats(window: isize) -> bool {
    floats(&read(window).facts())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS_POPUP: u32 = 0x8000_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    /// A program's window as Windows makes it: title bar, frame, both boxes.
    const OVERLAPPED: u32 = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

    fn floats_with(style: u32, extended: u32, class: &str, owned: bool) -> bool {
        floats(&Facts {
            style,
            extended,
            class,
            owned,
        })
    }

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
    fn a_window_of_its_own_that_is_no_main_window_has_no_border() {
        // The dialogs' own class.
        assert_eq!(kind_of(WS_CAPTION | WS_SYSMENU, 0, "#32770", false), Kind::Beside);
        // It can be neither resized nor minimised.
        assert_eq!(kind_of(WS_CAPTION | WS_SYSMENU, 0, "AboutBox", false), Kind::Beside);
        assert_eq!(kind_of(WS_POPUP, 0, "Splash", false), Kind::Beside);
        // Above everything, with a title bar: a dialog that insists.
        assert_eq!(
            kind_of(WS_CAPTION | WS_SYSMENU, WS_EX_TOPMOST, "Ask", false),
            Kind::Beside
        );
        assert!(Kind::Beside.is_managed());
    }

    #[test]
    fn what_belongs_to_another_window_is_not_gevreks() {
        // A dialog a window opened, of the dialogs' class or of any other.
        assert_eq!(kind_of(WS_POPUP | WS_CAPTION | WS_SYSMENU, 0, "#32770", true), Kind::Owned);
        assert_eq!(kind_of(OVERLAPPED, 0, "Settings", true), Kind::Owned);
        // A menu, a list that drops down, a bubble: with no frame at all,
        // above everything or not.
        assert_eq!(kind_of(WS_POPUP, WS_EX_TOPMOST, "Menu", true), Kind::Owned);
        assert_eq!(kind_of(WS_POPUP, 0, "Chrome_WidgetWin_1", true), Kind::Owned);
        assert!(!Kind::Owned.is_managed());
        // One that asks for its own place in the taskbar is a window of
        // its own, whatever it belongs to.
        assert_eq!(kind_of(OVERLAPPED, WS_EX_APPWINDOW, "Document", true), Kind::Ordinary);
    }

    #[test]
    fn what_passes_by_is_no_window_to_gevrek() {
        // Above everything, with no title bar and no frame: a notification.
        assert_eq!(kind_of(WS_POPUP, WS_EX_TOPMOST, "Toast", false), Kind::Passing);
        // A window inside another.
        assert_eq!(kind_of(WS_CHILD, 0, "Button", false), Kind::Passing);
        assert_eq!(kind_of(WS_CHILD | WS_CAPTION | WS_THICKFRAME, 0, "MDIChild", true), Kind::Passing);
        assert!(!Kind::Passing.is_managed());
    }

    #[test]
    fn a_window_that_cannot_take_a_tiles_size_floats() {
        // A program's window as Windows makes it is tiled.
        assert!(!floats_with(OVERLAPPED, 0, "Notepad", false));
        // An installer: a title bar, a box to minimise it by, one size.
        assert!(floats_with(WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 0, "TWizardForm", false));
        // A dialog with no window it belongs to, a splash screen.
        assert!(floats_with(WS_CAPTION | WS_SYSMENU, 0, "#32770", false));
        assert!(floats_with(OVERLAPPED, 0, "#32770", false));
        assert!(floats_with(WS_POPUP, 0, "Splash", false));
        // A player kept above everything, which can be resized.
        assert!(floats_with(
            WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX,
            WS_EX_TOPMOST,
            "Chrome_WidgetWin_1",
            false
        ));
        // A program that draws all of its window itself and has no frame
        // of the system's is tiled as before.
        assert!(!floats_with(WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU, 0, "Qt5QWindowIcon", false));
        assert!(!floats_with(WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX, 0, "Chrome_WidgetWin_1", false));
        // What is not gevrek's is neither.
        assert!(!floats_with(WS_POPUP | WS_CAPTION, 0, "#32770", true));
        assert!(!floats_with(WS_POPUP, WS_EX_TOPMOST, "Toast", false));
    }
}
