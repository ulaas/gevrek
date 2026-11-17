//! A window that comes in is seen coming in as a picture of itself.
//!
//! On Windows a window can only be moved and sized for real
//! (`SetWindowPos`): a window that grows into its place is sized anew at
//! every frame, and its program has to lay it out and draw it again at
//! every size. A program that has only just started cannot keep up, and
//! one that draws with DirectComposition (Chromium and Electron, the
//! newer Windows programs) shows nothing at all at a size it has not drawn
//! yet: the window flickered in and out all the way in.
//!
//! So the window is put in its place at once, at its size, unseen (alpha
//! 0), and a stand-in shows it coming: a window of gevrek's that holds a
//! DWM thumbnail of it, a live picture the compositor scales itself. Only
//! the picture moves; the program draws its window once, where it stays.
//! When the movement is over the window is seen again and the stand-in
//! goes, a frame later, so that the two are never both missing.
//!
//! The stand-in spans every display and is not seen anywhere but where
//! the picture is (it has no surface of its own), nor felt: clicks go
//! through it. Its class begins with `Gevrek_Border`, as all of gevrek's
//! own windows that are not to be managed.

use crate::dwm::container::Rect;
use crate::dwm::WindowHandle;
use std::collections::HashMap;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: windows::core::PCWSTR = windows::core::w!("Gevrek_BorderStandIn");

struct StandIn {
    hwnd: HWND,
    thumbnail: isize,
    /// Where the stand-in's own top left corner is on the screen.
    origin: (i32, i32),
    /// Where the picture was last put (what is seen of the window).
    rect: Rect,
    /// How solid the window is to be when it is seen again.
    alpha: u8,
}

impl StandIn {
    fn destroy(&self) {
        // SAFETY: the thumbnail and window are this program's own.
        unsafe {
            let _ = DwmUnregisterThumbnail(self.thumbnail);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

#[derive(Default)]
pub struct StandIns {
    class_registered: bool,
    live: HashMap<WindowHandle, StandIn>,
    /// Stand-ins whose windows are seen again, to be destroyed at the
    /// next frame: by then the window has been drawn where it is.
    retired: Vec<StandIn>,
}

/// A copy of the window manager's state does not own the windows made
/// here (as with `aura::Auras`): it has no stand-ins.
impl Clone for StandIns {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl StandIns {
    fn register_class(&mut self) {
        if self.class_registered {
            return;
        }
        unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // SAFETY: a plain class with a plain procedure.
        unsafe {
            let _ = RegisterClassW(&class);
        }
        self.class_registered = true;
    }

    /// Show a picture of `source` at `rect` (what is seen of a window, on
    /// the screen), right above `source`. `part` is what is seen of the
    /// window in its own coordinates (the window without its invisible
    /// frame). `false` where no picture can be had: the window is then
    /// left to move itself.
    pub fn open(
        &mut self,
        handle: WindowHandle,
        source: HWND,
        alpha: u8,
        rect: Rect,
        part: RECT,
    ) -> bool {
        self.close_at_once(handle);
        self.register_class();
        // SAFETY: plain calls; `source` may be gone, which they survive.
        unsafe {
            let source_ex = GetWindowLongW(source, GWL_EXSTYLE);
            let topmost = source_ex & WS_EX_TOPMOST.0 as i32 != 0;
            let mut ex = WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE;
            if topmost {
                ex |= WS_EX_TOPMOST;
            }
            let origin = (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            );
            let Ok(hwnd) = CreateWindowExW(
                ex,
                CLASS,
                windows::core::w!(""),
                WS_POPUP,
                origin.0,
                origin.1,
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            ) else {
                return false;
            };
            let thumbnail = match DwmRegisterThumbnail(hwnd, source) {
                Ok(thumbnail) => thumbnail,
                Err(error) => {
                    log::debug!(
                        "No picture of window {:?} to stand in for it: {error}",
                        handle
                    );
                    let _ = DestroyWindow(hwnd);
                    return false;
                }
            };
            let stand_in = StandIn {
                hwnd,
                thumbnail,
                origin,
                rect,
                alpha,
            };
            if !show_picture(&stand_in, rect, part) {
                stand_in.destroy();
                return false;
            }
            // Right above the window: what is above it stays above it.
            // (The window right above it may be the lowest of the windows
            // that stay on top, when it is the highest of the others: the
            // stand-in then is the highest of the others.)
            let above = GetWindow(source, GW_HWNDPREV).unwrap_or_default();
            let above_is_topmost = !above.is_invalid()
                && GetWindowLongW(above, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as i32 != 0;
            let after = if above.is_invalid() || (above_is_topmost && !topmost) {
                if topmost {
                    HWND_TOPMOST
                } else {
                    HWND_TOP
                }
            } else {
                above
            };
            let _ = SetWindowPos(
                hwnd,
                after,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            self.live.insert(handle, stand_in);
        }
        true
    }

    pub fn has(&self, handle: WindowHandle) -> bool {
        self.live.contains_key(&handle)
    }

    /// Where the picture of `handle` is.
    pub fn rect(&self, handle: WindowHandle) -> Option<Rect> {
        self.live.get(&handle).map(|stand_in| stand_in.rect)
    }

    pub fn handles(&self) -> Vec<WindowHandle> {
        self.live.keys().copied().collect()
    }

    /// Stand-ins that are shown or still to be destroyed: the loop goes
    /// on at the pace of a frame while there are any.
    pub fn count(&self) -> usize {
        self.live.len() + self.retired.len()
    }

    /// The picture of `handle` over every window that does not stay on
    /// top (over what an overview has put behind the pictures).
    pub fn raise(&self, handle: WindowHandle) {
        if let Some(stand_in) = self.live.get(&handle) {
            // SAFETY: the window is this program's own.
            unsafe {
                let _ = SetWindowPos(
                    stand_in.hwnd,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// Put the picture of `handle` at `rect`.
    pub fn place(&mut self, handle: WindowHandle, rect: Rect, part: RECT) {
        if let Some(stand_in) = self.live.get_mut(&handle) {
            stand_in.rect = rect;
            show_picture(stand_in, rect, part);
        }
    }

    /// The strength `handle` is to have once it is seen again; the picture
    /// is as strong meanwhile. `false` when it has no stand-in.
    pub fn set_alpha(&mut self, handle: WindowHandle, alpha: u8, part: RECT) -> bool {
        let Some(stand_in) = self.live.get_mut(&handle) else {
            return false;
        };
        stand_in.alpha = alpha;
        let rect = stand_in.rect;
        show_picture(stand_in, rect, part);
        true
    }

    /// The window stands for itself again: its stand-in is put away
    /// (destroyed at the next `sweep`). The strength it is to have.
    pub fn take(&mut self, handle: WindowHandle) -> Option<u8> {
        let stand_in = self.live.remove(&handle)?;
        let alpha = stand_in.alpha;
        self.retired.push(stand_in);
        Some(alpha)
    }

    /// Destroy the stand-ins put away before.
    pub fn sweep(&mut self) {
        for stand_in in self.retired.drain(..) {
            stand_in.destroy();
        }
    }

    fn close_at_once(&mut self, handle: WindowHandle) {
        if let Some(stand_in) = self.live.remove(&handle) {
            stand_in.destroy();
        }
    }
}

impl Drop for StandIns {
    fn drop(&mut self) {
        self.sweep();
        for (_, stand_in) in self.live.drain() {
            stand_in.destroy();
        }
    }
}

/// Show `part` of the window at `rect` on the screen.
fn show_picture(stand_in: &StandIn, rect: Rect, part: RECT) -> bool {
    let properties = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION
            | DWM_TNP_RECTSOURCE
            | DWM_TNP_OPACITY
            | DWM_TNP_VISIBLE
            | DWM_TNP_SOURCECLIENTAREAONLY,
        rcDestination: RECT {
            left: rect.x - stand_in.origin.0,
            top: rect.y - stand_in.origin.1,
            right: rect.x - stand_in.origin.0 + rect.width.max(1),
            bottom: rect.y - stand_in.origin.1 + rect.height.max(1),
        },
        rcSource: part,
        opacity: stand_in.alpha,
        fVisible: true.into(),
        fSourceClientAreaOnly: false.into(),
    };
    // SAFETY: the thumbnail is this program's own.
    unsafe { DwmUpdateThumbnailProperties(stand_in.thumbnail, &properties) }.is_ok()
}
