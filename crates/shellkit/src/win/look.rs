//! The desktop shown soft behind parts of a surface, on Windows.
//!
//! Windows gives a window a material behind it (the "system backdrop",
//! since Windows 11 of 2022) and round corners, both for a whole window
//! and for nothing less. A surface that covers a display and wants the
//! material behind a few rectangles of it cannot have it of its own
//! window. So each such rectangle is a window of its own that draws
//! nothing: it lies directly under the surface's window, at the
//! rectangle's place, has the material of a passing window (the one
//! menus and flyouts have, which blurs what is behind it) and the
//! system's round corners. The surface, seen through where it draws a
//! see-through ground, shows it.
//!
//! What this takes from Windows is only what is written down for
//! everyone: `DWMWA_SYSTEMBACKDROP_TYPE`, `DWMWA_WINDOW_CORNER_PREFERENCE`
//! and `DwmExtendFrameIntoClientArea`. What follows from that:
//!
//! - the corners are the system's, eight pixels or four at a scale of
//!   one, whatever radius was asked for ([`corner_of`] picks the nearer);
//! - where Windows has no such material (before Windows 11 22H2) nothing
//!   is shown behind the surface and [`supported`] says so: the drawing
//!   then makes its grounds more solid by itself;
//! - where the person has switched "transparency effects" off, Windows
//!   shows the material as one plain colour.
//!
//! **Nobody has seen this on a screen.** It is put together from the
//! documentation; `examples/look.rs` is there to look at it.

use super::place::Area;
use crate::look::{rounded, BlurRegion};
use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_AUTO, DWMSBT_TRANSIENTWINDOW,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
    DWMWCP_ROUNDSMALL, DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GetStockObject, BLACK_BRUSH, HBRUSH, HDC, PAINTSTRUCT,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, RegisterClassExW,
    SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HTTRANSPARENT, MA_NOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOWPOS, WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCACTIVATE,
    WM_NCHITTEST, WM_PAINT, WM_WINDOWPOSCHANGING, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

/// The corners Windows gives a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    Square,
    /// Four pixels at a scale of one.
    Small,
    /// Eight pixels at a scale of one.
    Round,
}

/// The corners of the system that are nearest a radius asked for, in
/// points. Windows has two radii and no others; a radius between them
/// goes to the nearer one, and one below a point and a half is square.
pub fn corner_of(radius: f32) -> Corner {
    if !radius.is_finite() || radius < 1.5 {
        Corner::Square
    } else if radius < 6.0 {
        Corner::Small
    } else {
        Corner::Round
    }
}

/// The parts of a surface as the windows that lie under it: each a
/// rectangle of the screen in pixels, for a surface at `origin` on a
/// display of this scale, with its corners. Parts of no size are left
/// out.
pub fn windows_for(regions: &[BlurRegion], origin: (i32, i32), scale: f32) -> Vec<(Area, Corner)> {
    regions
        .iter()
        .filter_map(|region| {
            let shape = rounded(region, scale)?;
            Some((
                Area::new(
                    origin.0 + shape.x,
                    origin.1 + shape.y,
                    shape.width,
                    shape.height,
                ),
                corner_of(region.radius),
            ))
        })
        .collect()
}

thread_local! {
    /// The window each backdrop lies under, for its procedure.
    static UNDER: RefCell<HashMap<isize, isize>> = RefCell::new(HashMap::new());
    /// The window classes registered so far.
    static CLASSES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Whether this Windows has the material: asked by setting "as the
/// system likes" on a window of ours that is never shown, which an older
/// Windows refuses.
pub fn supported(probe: HWND) -> bool {
    let kind = DWMSBT_AUTO;
    // SAFETY: a window of this thread, and a value of the size named.
    unsafe {
        DwmSetWindowAttribute(
            probe,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&kind as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
    .is_ok()
}

/// Leave a window out of pictures taken of the screen, or no longer.
pub fn exclude_from_capture(window: HWND, exclude: bool) -> bool {
    // SAFETY: a window of this thread.
    unsafe {
        SetWindowDisplayAffinity(
            window,
            if exclude {
                WDA_EXCLUDEFROMCAPTURE
            } else {
                WDA_NONE
            },
        )
    }
    .is_ok()
}

/// The procedure of a backdrop's window: it draws nothing (black, which
/// the desktop's compositor takes for "seen through" in a window whose
/// frame reaches over all of it), takes no press, is never the active
/// window, and stays directly under its surface whatever moves it.
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // (Through to what is under the pointer on this thread, which is
        // the surface above; and no press ever comes here.)
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => {
            let mut all = RECT::default();
            let _ = GetClientRect(window, &mut all);
            FillRect(
                HDC(wparam.0 as *mut _),
                &all,
                HBRUSH(GetStockObject(BLACK_BRUSH).0),
            );
            LRESULT(1)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let context = BeginPaint(window, &mut paint);
            FillRect(context, &paint.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
            let _ = EndPaint(window, &paint);
            LRESULT(0)
        }
        // Drawn as a window that is in use, always: Windows shows the
        // material of a window that is not as a plain colour.
        WM_NCACTIVATE => DefWindowProcW(window, message, WPARAM(1), lparam),
        WM_WINDOWPOSCHANGING => {
            let above = UNDER.with(|under| {
                under
                    .try_borrow()
                    .ok()
                    .and_then(|under| under.get(&(window.0 as isize)).copied())
            });
            if let Some(above) = above {
                let position = &mut *(lparam.0 as *mut WINDOWPOS);
                position.hwndInsertAfter = HWND(above as *mut _);
                position.flags &= !SWP_NOZORDER;
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

/// A surface's window changed its place among the windows: its backdrops
/// go under it again. (Called from the surface's own procedure.)
pub fn restack_under(surface: HWND) {
    let mine: Vec<isize> = UNDER.with(|under| {
        under
            .try_borrow()
            .map(|under| {
                under
                    .iter()
                    .filter(|(_, above)| **above == surface.0 as isize)
                    .map(|(backdrop, _)| *backdrop)
                    .collect()
            })
            .unwrap_or_default()
    });
    for backdrop in mine {
        // SAFETY: windows of this thread; no pointers.
        unsafe {
            let _ = SetWindowPos(
                HWND(backdrop as *mut _),
                surface,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
    }
}

/// One window under a surface that shows the desktop soft.
struct Backdrop {
    window: HWND,
    area: Area,
    corner: Option<Corner>,
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        UNDER.with(|under| {
            if let Ok(mut under) = under.try_borrow_mut() {
                under.remove(&(self.window.0 as isize));
            }
        });
        // SAFETY: a window of this thread, made here.
        unsafe {
            let _ = DestroyWindow(self.window);
        }
    }
}

/// The windows under one surface; gone with this.
#[derive(Default)]
pub struct Backdrops {
    windows: Vec<Backdrop>,
    /// Left out of pictures of the screen, as their surface is.
    hidden_from_capture: bool,
}

fn class(module: HINSTANCE, name: &str) -> Option<Vec<u16>> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let known = CLASSES.with(|classes| classes.borrow().iter().any(|known| known == name));
    if !known {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: module,
            lpszClassName: PCWSTR(wide.as_ptr()),
            ..Default::default()
        };
        // SAFETY: the name lives until the call returns.
        if unsafe { RegisterClassExW(&class) } == 0 {
            log::warn!(
                "the window class {name}: {}",
                windows::core::Error::from_win32()
            );
            return None;
        }
        CLASSES.with(|classes| classes.borrow_mut().push(name.to_string()));
    }
    Some(wide)
}

impl Backdrops {
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Make the windows under `surface` these and no others: kept where
    /// they are the same, moved where they are not, made and destroyed
    /// as their number changes.
    pub fn set(&mut self, module: HINSTANCE, surface: HWND, name: &str, wanted: &[(Area, Corner)]) {
        self.windows.truncate(wanted.len());
        while self.windows.len() < wanted.len() {
            let Some(backdrop) = make(module, surface, name) else {
                break;
            };
            if self.hidden_from_capture {
                exclude_from_capture(backdrop.window, true);
            }
            self.windows.push(backdrop);
        }
        for (backdrop, (area, corner)) in self.windows.iter_mut().zip(wanted) {
            if backdrop.corner != Some(*corner) {
                let preference = match corner {
                    Corner::Square => DWMWCP_DONOTROUND,
                    Corner::Small => DWMWCP_ROUNDSMALL,
                    Corner::Round => DWMWCP_ROUND,
                };
                // SAFETY: a window of this thread, a value of the size named.
                unsafe {
                    let _ = DwmSetWindowAttribute(
                        backdrop.window,
                        DWMWA_WINDOW_CORNER_PREFERENCE,
                        (&preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                        std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
                    );
                }
                backdrop.corner = Some(*corner);
            }
            let moved = backdrop.area != *area;
            backdrop.area = *area;
            // SAFETY: windows of this thread; no pointers.
            unsafe {
                // (Its procedure puts it under the surface, whatever is
                // named here.)
                let _ = SetWindowPos(
                    backdrop.window,
                    surface,
                    area.x,
                    area.y,
                    area.width,
                    area.height,
                    if moved {
                        SWP_NOACTIVATE
                    } else {
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE
                    },
                );
                let _ = ShowWindow(backdrop.window, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Leave them out of pictures of the screen with their surface, or
    /// no longer.
    pub fn exclude_from_capture(&mut self, exclude: bool) {
        self.hidden_from_capture = exclude;
        for backdrop in &self.windows {
            exclude_from_capture(backdrop.window, exclude);
        }
    }

    /// Out of sight until they are set again (their surface is gone from
    /// where they were).
    pub fn hide(&mut self) {
        for backdrop in &self.windows {
            // SAFETY: a window of this thread.
            unsafe {
                let _ = ShowWindow(backdrop.window, SW_HIDE);
            }
        }
    }
}

/// A window that draws nothing and has the material behind it.
fn make(module: HINSTANCE, surface: HWND, name: &str) -> Option<Backdrop> {
    let class = class(module, &super::place::backdrop_class_of(name))?;
    // SAFETY: the class's name lives until the call returns.
    let window = unsafe {
        CreateWindowExW(
            // Not in the taskbar or the window switcher, never active.
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class.as_ptr()),
            PCWSTR(class.as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            module,
            None,
        )
    }
    .map_err(|error| log::warn!("a backdrop's window: {error}"))
    .ok()?;
    UNDER.with(|under| {
        if let Ok(mut under) = under.try_borrow_mut() {
            under.insert(window.0 as isize, surface.0 as isize);
        }
    });
    let backdrop = Backdrop {
        window,
        area: Area::default(),
        corner: None,
    };
    // The frame over the whole window: what the window does not draw
    // (all of it) shows the material.
    let all = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let kind = DWMSBT_TRANSIENTWINDOW;
    // SAFETY: a window of this thread; values of the sizes named.
    let set = unsafe {
        let _ = DwmExtendFrameIntoClientArea(window, &all);
        DwmSetWindowAttribute(
            window,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&kind as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    };
    if let Err(error) = set {
        log::warn!("Windows gave a backdrop no material: {error}");
        return None;
    }
    // SAFETY: as above. (Told once that it is in use: see the procedure.)
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
            window,
            WM_NCACTIVATE,
            WPARAM(BOOL::from(true).0 as usize),
            LPARAM(0),
        );
    }
    Some(backdrop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: f32, y: f32, width: f32, height: f32, radius: f32) -> BlurRegion {
        BlurRegion::new(
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height)),
            radius,
        )
    }

    #[test]
    fn a_radius_gets_the_nearer_of_the_systems_two() {
        let table = [
            (0.0, Corner::Square),
            (1.0, Corner::Square),
            (2.0, Corner::Small),
            (4.0, Corner::Small),
            (5.9, Corner::Small),
            (6.0, Corner::Round),
            (8.0, Corner::Round),
            (24.0, Corner::Round),
            (-3.0, Corner::Square),
            (f32::NAN, Corner::Square),
        ];
        for (radius, corner) in table {
            assert_eq!(corner_of(radius), corner, "{radius}");
        }
    }

    #[test]
    fn a_part_is_a_window_at_its_place_on_the_screen_in_pixels() {
        // A surface on a second display to the left, at one and a half.
        let parts = [
            region(20.0, 40.0, 300.0, 200.0, 12.0),
            region(0.0, 0.0, 0.0, 50.0, 4.0),
            region(400.0, 40.0, 100.0, 60.0, 0.0),
        ];
        assert_eq!(
            windows_for(&parts, (-2880, 120), 1.5),
            [
                (Area::new(-2850, 180, 450, 300), Corner::Round),
                (Area::new(-2280, 180, 150, 90), Corner::Square),
            ]
        );
        assert!(windows_for(&[], (0, 0), 1.0).is_empty());
    }
}
