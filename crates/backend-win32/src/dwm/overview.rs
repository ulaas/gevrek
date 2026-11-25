//! The overview of a workspace's windows: what is behind the pictures.
//!
//! The windows of a workspace are shown small, side by side, to choose
//! one of. On Windows a window cannot be shown smaller than it is, only
//! made smaller for real, which its program has to follow (the overview
//! used to do that: every window was sized into a cell of a grid, laid
//! itself out anew, and was sized back). So the windows stay as they are,
//! unseen, and what is seen are live pictures of them (`stand_in`), which
//! the compositor makes as small as it is asked to.
//!
//! This is the window behind those pictures: it darkens the workspace,
//! writes each window's title under its picture, marks the one that is
//! pointed at, and hears the mouse and the keyboard while the overview is
//! shown. What it hears it only notes (`Said`); the window manager reads
//! the notes at its next pass and does what they ask
//! (`WindowManager::tick_window_overview`).
//!
//! Its class begins with `Gevrek_Border`, as all of gevrek's own windows
//! that are not to be managed, and that no event is told of.

use crate::dwm::container::Rect;
use std::sync::Mutex;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: windows::core::PCWSTR = windows::core::w!("Gevrek_BorderOverview");

/// How dark the workspace is behind the pictures, of 255.
pub const DARK: u8 = 205;
/// How far the mark of the picture that is pointed at stands out around
/// it, in pixels.
const MARK: i32 = 5;

/// What the person did at the overview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Said {
    /// The pointer is here (on the screen).
    Pointed(i32, i32),
    /// The mouse was pressed and let go here.
    Pressed(i32, i32),
    /// A key went down (its virtual key code).
    Key(u32),
    /// The keyboard went to another window.
    Left,
}

/// A picture as the window behind it knows it: where it is on the
/// screen, and what is written under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub rect: Rect,
    pub title: String,
}

struct Shared {
    said: Vec<Said>,
    shown: Vec<Shown>,
    marked: Option<usize>,
    /// Where the window's own top left corner is on the screen.
    origin: (i32, i32),
    /// The colour of the mark.
    accent: (u8, u8, u8),
    /// It is where it belongs among the windows: it is not raised over
    /// the pictures when it is pressed.
    placed: bool,
}

/// One overview at a time: what its window and the window manager both
/// know.
static SHARED: Mutex<Shared> = Mutex::new(Shared {
    said: Vec::new(),
    shown: Vec::new(),
    marked: None,
    origin: (0, 0),
    accent: (90, 150, 255),
    placed: false,
});

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn note(said: Said) {
    let mut shared = shared();
    // (Of many places the pointer was at, the last is the one it is at.)
    if let (Said::Pointed(..), Some(Said::Pointed(..))) = (said, shared.said.last()) {
        shared.said.pop();
    }
    shared.said.push(said);
    drop(shared);
    gevrek_host::wake::now();
}

fn colour((red, green, blue): (u8, u8, u8)) -> COLORREF {
    COLORREF((blue as u32) << 16 | (green as u32) << 8 | (red as u32))
}

/// The point of a mouse message, on the screen.
fn point_of(lparam: LPARAM, origin: (i32, i32)) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x + origin.0, y + origin.1)
}

unsafe fn paint(hwnd: HWND) {
    let mut painted = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut painted);
    let mut all = RECT::default();
    let _ = GetClientRect(hwnd, &mut all);
    FillRect(dc, &all, HBRUSH(GetStockObject(BLACK_BRUSH).0));
    let shared = shared();
    let within = |rect: &Rect| RECT {
        left: rect.x - shared.origin.0,
        top: rect.y - shared.origin.1,
        right: rect.x - shared.origin.0 + rect.width,
        bottom: rect.y - shared.origin.1 + rect.height,
    };
    // The mark of the one that is pointed at: it stands out around its
    // picture, which lies over the rest of it.
    if let Some(marked) = shared.marked.and_then(|at| shared.shown.get(at)) {
        let mut around = within(&marked.rect);
        around.left -= MARK;
        around.top -= MARK;
        around.right += MARK;
        around.bottom += MARK;
        let brush = CreateSolidBrush(colour(shared.accent));
        FillRect(dc, &around, brush);
        let _ = DeleteObject(brush);
    }
    // The titles, each under its picture.
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let font = CreateFontW(
        -(15 * dpi / 96),
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
        windows::core::w!("Segoe UI"),
    );
    let before = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT);
    for (at, one) in shared.shown.iter().enumerate() {
        let picture = within(&one.rect);
        // (Wider than a narrow picture, so that a title can be read.)
        let wider = ((160 - one.rect.width) / 2).max(0);
        let mut line = RECT {
            left: picture.left - wider,
            top: picture.bottom + MARK + 2,
            right: picture.right + wider,
            bottom: picture.bottom + gevrek_engine::expose::PICTURE_TITLE,
        };
        SetTextColor(
            dc,
            if shared.marked == Some(at) {
                colour((255, 255, 255))
            } else {
                colour((200, 200, 200))
            },
        );
        let mut text: Vec<u16> = one.title.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut text,
            &mut line,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
    SelectObject(dc, before);
    let _ = DeleteObject(font);
    drop(shared);
    let _ = EndPaint(hwnd, &painted);
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let origin = shared().origin;
            let (x, y) = point_of(lparam, origin);
            note(Said::Pointed(x, y));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let origin = shared().origin;
            let (x, y) = point_of(lparam, origin);
            note(Said::Pressed(x, y));
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            note(Said::Key(wparam.0 as u32));
            LRESULT(0)
        }
        // The keyboard went elsewhere: the overview is over.
        WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            note(Said::Left);
            LRESULT(0)
        }
        // A press does not raise it: the pictures lie over it.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_WINDOWPOSCHANGING => {
            if shared().placed && lparam.0 != 0 {
                // SAFETY: Windows gives a valid WINDOWPOS with this message.
                let position = &mut *(lparam.0 as *mut WINDOWPOS);
                position.flags |= SWP_NOZORDER;
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// The window behind the pictures of an overview.
pub struct Backdrop {
    hwnd: HWND,
    dark: u8,
}

impl Backdrop {
    /// Over `area`, with these pictures to write under and to mark.
    /// It takes the keyboard where Windows lets it. `None` where no
    /// window could be made.
    pub fn open(area: Rect, shown: Vec<Shown>, accent: (u8, u8, u8)) -> Option<Self> {
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(|| {
            // SAFETY: a plain class with this module's procedure.
            unsafe {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    lpszClassName: CLASS,
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                let _ = RegisterClassW(&class);
            }
        });
        {
            let mut shared = shared();
            shared.said.clear();
            shared.shown = shown;
            shared.marked = None;
            shared.origin = (area.x, area.y);
            shared.accent = accent;
            shared.placed = false;
        }
        // SAFETY: plain calls on a window of this program's own.
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                CLASS,
                windows::core::w!(""),
                WS_POPUP,
                area.x,
                area.y,
                area.width.max(1),
                area.height.max(1),
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            )
            .ok()?;
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            // The keyboard, for Escape, the arrows and Enter. Where
            // Windows does not give it, the mouse still chooses.
            // (Not in a test: it would take the keyboard of whoever runs it.)
            #[cfg(not(test))]
            let _ = SetForegroundWindow(hwnd);
            shared().placed = true;
            Some(Self { hwnd, dark: 0 })
        }
    }

    /// How dark it is, of 255.
    pub fn darken(&mut self, dark: u8) {
        if dark != self.dark {
            self.dark = dark;
            // SAFETY: the window is this program's own.
            unsafe {
                let _ = SetLayeredWindowAttributes(self.hwnd, COLORREF(0), dark, LWA_ALPHA);
            }
        }
    }

    /// The picture that is pointed at (by its place among them).
    pub fn mark(&self, marked: Option<usize>) {
        let mut shared = shared();
        if shared.marked != marked {
            shared.marked = marked;
            drop(shared);
            // SAFETY: the window is this program's own.
            unsafe {
                let _ = InvalidateRect(self.hwnd, None, false);
            }
        }
    }

    /// What was done at it since the last time.
    pub fn said(&self) -> Vec<Said> {
        std::mem::take(&mut shared().said)
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        {
            let mut shared = shared();
            shared.placed = false;
            shared.shown.clear();
            shared.said.clear();
        }
        // SAFETY: the window is this program's own.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
