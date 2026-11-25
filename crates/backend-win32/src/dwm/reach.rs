//! The few things done to another program's window, done by whoever can
//! do them: this program, or, for a window of a program that runs as an
//! administrator, the helper that runs so too (`elevated_helper`).
//!
//! Each of these is the Windows call of its name. A window within this
//! program's reach, and every window while no helper answers, is called
//! as before.

use crate::elevated_helper::{client, After, Answer, Ask, Placed};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SetWindowPos, ShowWindow, ShowWindowAsync, SET_WINDOW_POS_FLAGS, SHOW_WINDOW_CMD,
    WM_CLOSE,
};

/// Whether what is done to this window is asked of the helper: the
/// window is beyond this program's reach, and a helper answers.
pub fn by_helper(hwnd: HWND) -> bool {
    client::connected() && crate::elevated::window_is_beyond_reach(hwnd.0 as isize)
}

/// What the helper said, as the answer of the call it stands for.
fn answered(answer: Option<Answer>) -> windows::core::Result<()> {
    match answer {
        Some(Answer::Done) => Ok(()),
        Some(other) => {
            log::debug!("the helper for elevated windows: {other:?}");
            Err(windows::core::Error::from(
                windows::Win32::Foundation::E_ACCESSDENIED,
            ))
        }
        None => Err(windows::core::Error::from(
            windows::Win32::Foundation::E_ACCESSDENIED,
        )),
    }
}

/// `SetWindowPos`.
///
/// # Safety
/// As the call itself: `hwnd` and `after` are handles of windows, or
/// were (a window that is gone takes no harm).
pub unsafe fn set_window_pos(
    hwnd: HWND,
    after: Option<HWND>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    flags: SET_WINDOW_POS_FLAGS,
) -> windows::core::Result<()> {
    if by_helper(hwnd) {
        return answered(client::ask(&Ask::Place {
            window: hwnd.0 as isize,
            after: After::of(after),
            x,
            y,
            width,
            height,
            flags: flags.0,
        }));
    }
    SetWindowPos(hwnd, after.unwrap_or_default(), x, y, width, height, flags)
}

/// `SetWindowPos` for several windows of elevated programs at once,
/// none of them moved among the others: one question to the helper.
/// (Only for windows that are the helper's: [`by_helper`].)
pub fn place_many_by_helper(
    places: &[(HWND, i32, i32, i32, i32)],
    flags: SET_WINDOW_POS_FLAGS,
) -> windows::core::Result<()> {
    if places.is_empty() {
        return Ok(());
    }
    answered(client::ask(&Ask::PlaceMany {
        places: places
            .iter()
            .map(|(hwnd, x, y, width, height)| Placed {
                window: hwnd.0 as isize,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            })
            .collect(),
        flags: flags.0,
    }))
}

fn show(hwnd: HWND, command: SHOW_WINDOW_CMD, posted: bool) {
    if by_helper(hwnd) {
        let _ = answered(client::ask(&Ask::Show {
            window: hwnd.0 as isize,
            command: command.0,
            posted,
        }));
        return;
    }
    // SAFETY: plain calls on a window that may be gone meanwhile.
    unsafe {
        if posted {
            let _ = ShowWindowAsync(hwnd, command);
        } else {
            let _ = ShowWindow(hwnd, command);
        }
    }
}

/// `ShowWindowAsync`.
pub fn show_window_async(hwnd: HWND, command: SHOW_WINDOW_CMD) {
    show(hwnd, command, true);
}

/// `ShowWindow`.
pub fn show_window(hwnd: HWND, command: SHOW_WINDOW_CMD) {
    show(hwnd, command, false);
}

/// Give a window the keyboard where that is the helper's to do. `false`:
/// it is this program's own to do (`take_foreground`).
pub fn foreground_by_helper(hwnd: HWND) -> bool {
    if !by_helper(hwnd) {
        return false;
    }
    let _ = answered(client::ask(&Ask::Foreground {
        window: hwnd.0 as isize,
    }));
    true
}

/// Ask a window to close (`WM_CLOSE`, posted).
pub fn post_close(hwnd: HWND) -> windows::core::Result<()> {
    if by_helper(hwnd) {
        return answered(client::ask(&Ask::Close {
            window: hwnd.0 as isize,
        }));
    }
    // SAFETY: posted to a window that may be gone meanwhile.
    unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) }
}

/// `SetWindowLongW` for a window's style or its extended style. (The
/// helper changes of them only what gevrek changes of any window: the
/// other bits of an elevated window stay as they are.)
///
/// # Safety
/// As the call itself.
pub unsafe fn set_window_long(
    hwnd: HWND,
    index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
    value: i32,
) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongW, GWL_EXSTYLE, GWL_STYLE};
    if (index == GWL_STYLE || index == GWL_EXSTYLE) && by_helper(hwnd) {
        let _ = answered(client::ask(&Ask::Style {
            window: hwnd.0 as isize,
            extended: index == GWL_EXSTYLE,
            value,
        }));
        return;
    }
    SetWindowLongW(hwnd, index, value);
}

/// How solid a window is (`SetLayeredWindowAttributes` with `LWA_ALPHA`).
///
/// # Safety
/// As the call itself.
pub unsafe fn set_layered_alpha(hwnd: HWND, alpha: u8) -> windows::core::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA};
    if by_helper(hwnd) {
        return answered(client::ask(&Ask::Alpha {
            window: hwnd.0 as isize,
            alpha,
        }));
    }
    SetLayeredWindowAttributes(
        hwnd,
        windows::Win32::Foundation::COLORREF(0),
        alpha,
        LWA_ALPHA,
    )
}

/// `DwmSetWindowAttribute`. (The helper passes on the attributes gevrek
/// sets, each of which is one number of four bytes.)
///
/// # Safety
/// As the call itself: `value` points at `size` bytes.
pub unsafe fn dwm_set_window_attribute(
    hwnd: HWND,
    attribute: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE,
    value: *const core::ffi::c_void,
    size: u32,
) -> windows::core::Result<()> {
    if size == 4 && by_helper(hwnd) {
        return answered(client::ask(&Ask::Dwm {
            window: hwnd.0 as isize,
            attribute: attribute.0 as u32,
            value: std::ptr::read_unaligned(value as *const u32),
        }));
    }
    windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, attribute, value, size)
}
