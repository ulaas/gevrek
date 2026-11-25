//! Windows' taskbar put away (`desktop.platform.windows.hide_taskbar`, on
//! unless switched off), for a desktop whose panel is gevrek's own: it is
//! hidden only while that panel is on the screen (or about to come, just
//! after the start), so no desktop is left with neither.
//!
//! The taskbar's windows (one on each display) are hidden, not ended, and
//! none of Windows' settings is changed: nothing is written that a later
//! start of Windows would find. Windows still keeps the taskbar's strip
//! clear for its own purposes (a maximised window that gevrek does not
//! manage stops short of it); the window manager lays its windows out over
//! the strip (`work_with_strips`). Windows shows its taskbar again now and
//! then (the Start menu does); it is looked at every two seconds and put
//! away again.
//!
//! The taskbar is shown again when the setting is switched off, when the
//! window manager stops, and when it is dropped without stopping. For the
//! case that it is ended from outside, a note is left with the instance's
//! state while the taskbar is hidden: the next start that finds it shows
//! the taskbar first.

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, IsWindowVisible, ShowWindowAsync, SW_HIDE,
    SW_SHOWNOACTIVATE,
};

/// The classes of the taskbar's windows: the main display's, and one for
/// each other display.
const CLASSES: [&str; 2] = ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
    let found = &mut *(found.0 as *mut Vec<isize>);
    let mut name = [0u16; 64];
    let length = GetClassNameW(window, &mut name) as usize;
    let class = String::from_utf16_lossy(&name[..length.min(name.len())]);
    // (Not a window that only bears the name: gevrek's shell has one of
    // no size, to hear what programs tell the taskbar of their icons.)
    let mut rect = RECT::default();
    let real = GetWindowRect(window, &mut rect).is_ok()
        && rect.right > rect.left
        && rect.bottom > rect.top;
    if real && CLASSES.contains(&class.as_str()) {
        found.push(window.0 as isize);
    }
    true.into()
}

/// The taskbar's windows.
fn windows() -> Vec<isize> {
    let mut found: Vec<isize> = Vec::new();
    // SAFETY: the list outlives the call that fills it.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut Vec<isize> as isize));
    }
    found
}

/// Where the taskbar's windows are (also while they are hidden): x, y,
/// width, height.
pub fn strips() -> Vec<(i32, i32, i32, i32)> {
    windows()
        .into_iter()
        .filter_map(|window| {
            let mut rect = RECT::default();
            // SAFETY: the rectangle is ours.
            unsafe { GetWindowRect(HWND(window as *mut _), &mut rect) }.ok()?;
            Some((
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ))
        })
        .collect()
}

/// A display's room for windows with the strips of a hidden taskbar given
/// to it: the smallest rectangle that holds the room and what of each
/// strip is on the display. (A taskbar lies along a whole edge, so that is
/// the room made longer; a strip that is not on the display changes
/// nothing.)
pub fn work_with_strips(
    display: (i32, i32, i32, i32),
    work: (i32, i32, i32, i32),
    strips: &[(i32, i32, i32, i32)],
) -> (i32, i32, i32, i32) {
    let (mut left, mut top) = (work.0, work.1);
    let (mut right, mut bottom) = (work.0 + work.2, work.1 + work.3);
    for strip in strips {
        // What of the strip is on this display.
        let on = (
            strip.0.max(display.0),
            strip.1.max(display.1),
            (strip.0 + strip.2).min(display.0 + display.2),
            (strip.1 + strip.3).min(display.1 + display.3),
        );
        if on.2 <= on.0 || on.3 <= on.1 {
            continue;
        }
        left = left.min(on.0);
        top = top.min(on.1);
        right = right.max(on.2);
        bottom = bottom.max(on.3);
    }
    (left, top, right - left, bottom - top)
}

/// The note that says the taskbar is hidden by this instance.
fn note() -> std::path::PathBuf {
    gevrek_model::paths::state_dir().join("taskbar-hidden")
}

fn show(window: isize, shown: bool) {
    // SAFETY: a window that is gone is no window; nothing happens.
    unsafe {
        // (Posted: the taskbar is another program's window, and a
        // program that hangs is not waited for.)
        let _ = ShowWindowAsync(
            HWND(window as *mut _),
            if shown { SW_SHOWNOACTIVATE } else { SW_HIDE },
        );
    }
}

fn visible(window: isize) -> bool {
    unsafe { IsWindowVisible(HWND(window as *mut _)) }.as_bool()
}

/// The taskbar as the window manager has it.
#[derive(Default)]
pub struct Taskbar {
    hidden: bool,
}

impl Taskbar {
    /// A window manager that was ended from outside while the taskbar was
    /// hidden left a note: the taskbar is shown first.
    pub fn show_what_was_left_hidden() {
        if note().exists() {
            log::info!(
                "The taskbar was left hidden by a window manager that was ended: shown again"
            );
            for window in windows() {
                show(window, true);
            }
            let _ = std::fs::remove_file(note());
        }
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Hide the taskbar or show it, as the configuration asks. `true` when
    /// the room for windows may be another now (hidden, or shown again).
    pub fn follow(&mut self, hide: bool) -> bool {
        if hide {
            // Again each time: Windows shows it now and then.
            let mut put_away = 0;
            for window in windows() {
                if visible(window) {
                    show(window, false);
                    put_away += 1;
                }
            }
            if !self.hidden {
                self.hidden = true;
                if let Some(folder) = note().parent() {
                    let _ = std::fs::create_dir_all(folder);
                }
                let _ = std::fs::write(note(), "");
                log::info!("The taskbar is hidden ({put_away} windows)");
                return true;
            }
            false
        } else if self.hidden {
            self.restore();
            log::info!("The taskbar is shown again");
            true
        } else {
            false
        }
    }

    /// Show the taskbar again, if this hid it.
    pub fn restore(&mut self) {
        if !self.hidden {
            return;
        }
        for window in windows() {
            show(window, true);
        }
        self.hidden = false;
        let _ = std::fs::remove_file(note());
    }
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_taskbars_strip_is_room_for_windows() {
        let display = (0, 0, 2560, 1440);
        // Along the bottom: the room reaches the display's lower edge.
        let work = (0, 0, 2560, 1392);
        assert_eq!(
            work_with_strips(display, work, &[(0, 1392, 2560, 48)]),
            (0, 0, 2560, 1440)
        );
        // Along the left side.
        assert_eq!(
            work_with_strips(display, (60, 0, 2500, 1440), &[(0, 0, 60, 1440)]),
            (0, 0, 2560, 1440)
        );
        // The other display's taskbar changes nothing here; nor does none.
        let other = [(2560, 1392, 2560, 48)];
        assert_eq!(work_with_strips(display, work, &other), work);
        assert_eq!(work_with_strips(display, work, &[]), work);
        // A display that begins elsewhere, with its own strip among two.
        let second = (2560, 0, 2560, 1440);
        assert_eq!(
            work_with_strips(
                second,
                (2560, 0, 2560, 1392),
                &[(0, 1392, 2560, 48), (2560, 1392, 2560, 48)]
            ),
            (2560, 0, 2560, 1440)
        );
    }
}
