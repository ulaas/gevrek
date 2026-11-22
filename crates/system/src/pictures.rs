//! A small picture of a window as it looks: what the window switcher
//! shows of the window the choice is on.
//!
//! On Windows the window is asked to draw itself into memory
//! (`PrintWindow`, with what the desktop's compositor has of it), so a
//! window that is behind others is seen whole. A window that is minimised
//! or hidden has no picture. On Linux the shell asks the compositor itself.

/// A picture: red, green, blue and how solid, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A picture as Windows gives it (blue, green, red and a byte to spare,
/// row by row from the top) made no larger than `side` on its longer side,
/// by taking every so-manieth pixel. `None` when the bytes are fewer than
/// the size says.
pub fn shrink(width: u32, height: u32, pixels: &[u8], side: u32) -> Option<Picture> {
    let (width, height) = (width as usize, height as usize);
    if width == 0 || height == 0 || pixels.len() < width * height * 4 {
        return None;
    }
    let by = (width.max(height) as f32 / side.max(1) as f32).max(1.0);
    let (out_width, out_height) = (
        ((width as f32 / by) as usize).max(1),
        ((height as f32 / by) as usize).max(1),
    );
    let mut rgba = Vec::with_capacity(out_width * out_height * 4);
    for row in 0..out_height {
        let from_row = ((row as f32 * by) as usize).min(height - 1);
        for column in 0..out_width {
            let from = (from_row * width + ((column as f32 * by) as usize).min(width - 1)) * 4;
            rgba.extend_from_slice(&[pixels[from + 2], pixels[from + 1], pixels[from], 255]);
        }
    }
    Some(Picture {
        width: out_width as u32,
        height: out_height as u32,
        rgba,
    })
}

/// The picture of a window (by its handle), no larger than `side` on its
/// longer side. `None` for a window that is gone, minimised, or gives none.
#[cfg(target_os = "macos")]
pub use self::mac::of_window;

/// ScreenCaptureKit: the picture of one window, alone, as it looks (a
/// window behind others whole). macOS shows other programs' windows only
/// to a program that may record the screen: the first picture asked for
/// asks for that permission (the system's own question, once), and until
/// it is given there are no pictures.
#[cfg(target_os = "macos")]
mod mac {
    use super::Picture;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_core_graphics::{CGDataProvider, CGImage};
    use objc2_foundation::NSError;
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
    };
    use std::sync::mpsc::channel;
    use std::sync::{Mutex, Once};
    use std::time::{Duration, Instant};

    /// How long an answer of ScreenCaptureKit is waited for.
    const WAIT: Duration = Duration::from_secs(1);
    /// How long the list of windows is used before it is asked again (a
    /// switcher asks for many pictures at once).
    const LIST_KEPT: Duration = Duration::from_secs(2);

    /// An object of ScreenCaptureKit carried from the queue it answers on.
    struct Carried<T>(Retained<T>);
    // SAFETY: ScreenCaptureKit's objects are immutable answers, used from
    // any thread by its own examples.
    unsafe impl<T> Send for Carried<T> {}

    static CONTENT: Mutex<Option<(Instant, Carried<SCShareableContent>)>> = Mutex::new(None);

    fn shareable(again: bool) -> Option<Retained<SCShareableContent>> {
        let mut kept = CONTENT.lock().ok()?;
        if let Some((at, content)) = kept.as_ref() {
            if !again && at.elapsed() < LIST_KEPT {
                return Some(content.0.clone());
            }
        }
        let (sender, answer) = channel();
        let block = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: the answer's objects, retained to be carried.
            let content = unsafe { Retained::retain(content) };
            if content.is_none() && !error.is_null() {
                let error = unsafe { &*error };
                log::debug!("no windows to picture: {}", error.localizedDescription());
            }
            let _ = sender.send(content.map(Carried));
        });
        // SAFETY: the block is kept by ScreenCaptureKit until it answers.
        unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
        let content = answer.recv_timeout(WAIT).ok().flatten()?.0;
        *kept = Some((Instant::now(), Carried(content.clone())));
        Some(content)
    }

    fn window(handle: isize) -> Option<Retained<SCWindow>> {
        let find = |content: &SCShareableContent| {
            // SAFETY: plain properties of the answer.
            unsafe { content.windows() }
                .iter()
                .find(|window| unsafe { window.windowID() } as isize == handle)
        };
        // (A window newer than the list kept: the list is asked again.)
        let kept = shareable(false)?;
        find(&kept).or_else(|| shareable(true).and_then(|content| find(&content)))
    }

    /// Whether pictures may be taken, asking (once in the program's life)
    /// where they may not yet.
    fn allowed() -> bool {
        static ASKED: Once = Once::new();
        // (The window server's connection, which the capture needs, is
        // opened by any question to it: in a program without windows of
        // its own, here.)
        objc2_core_graphics::CGMainDisplayID();
        if objc2_core_graphics::CGPreflightScreenCaptureAccess() {
            return true;
        }
        ASKED.call_once(|| {
            log::info!("pictures of windows need the Screen Recording permission: asked for");
            objc2_core_graphics::CGRequestScreenCaptureAccess();
        });
        false
    }

    pub fn of_window(handle: isize, side: u32) -> Option<Picture> {
        if !allowed() {
            return None;
        }
        let window = window(handle)?;
        // SAFETY: a filter of the one window, a configuration of the size
        // wanted, and the capture's answer carried back.
        let image = unsafe {
            let frame = window.frame();
            let (width, height) = (frame.size.width.max(1.0), frame.size.height.max(1.0));
            // (At most twice the points: a display's pixels.)
            let by = (f64::from(side.max(1)) / width.max(height)).min(2.0);
            let filter = SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window);
            let configuration = SCStreamConfiguration::new();
            configuration.setWidth((width * by).round().max(1.0) as usize);
            configuration.setHeight((height * by).round().max(1.0) as usize);
            configuration.setShowsCursor(false);
            configuration.setIgnoreShadowsSingleWindow(true);
            let (sender, answer) = channel();
            let block = RcBlock::new(move |image: *mut CGImage, _error: *mut NSError| {
                let image = (!image.is_null()).then(|| {
                    objc2_core_foundation::CFRetained::retain(std::ptr::NonNull::new_unchecked(image))
                });
                let _ = sender.send(image.map(CarriedImage));
            });
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                &filter,
                &configuration,
                Some(&block),
            );
            answer.recv_timeout(WAIT).ok().flatten()?.0
        };
        picture_of(&image, side)
    }

    struct CarriedImage(objc2_core_foundation::CFRetained<CGImage>);
    // SAFETY: a CGImage is immutable, and may be read on any thread.
    unsafe impl Send for CarriedImage {}

    /// The image's pixels (blue first, as ScreenCaptureKit gives them),
    /// made a picture no larger than `side`.
    fn picture_of(image: &CGImage, side: u32) -> Option<Picture> {
        let (width, height) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
        let row = CGImage::bytes_per_row(Some(image));
        if CGImage::bits_per_pixel(Some(image)) != 32 || width == 0 || height == 0 {
            return None;
        }
        let data = CGDataProvider::data(CGImage::data_provider(Some(image)).as_deref())?;
        let bytes = data.to_vec();
        // Rows may be longer than their pixels: packed tightly first.
        let mut packed = Vec::with_capacity(width * height * 4);
        for line in bytes.chunks(row).take(height) {
            packed.extend_from_slice(line.get(..width * 4)?);
        }
        super::shrink(width as u32, height as u32, &packed, side)
    }
}

#[cfg(windows)]
pub fn of_window(handle: isize, side: u32) -> Option<Picture> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsIconic, IsWindow, IsWindowVisible,
    };

    let window = HWND(handle as *mut _);
    // SAFETY: every handle made here is given back before the function
    // returns; the pixels are read while the bitmap that owns them lives.
    unsafe {
        if !IsWindow(window).as_bool()
            || !IsWindowVisible(window).as_bool()
            || IsIconic(window).as_bool()
        {
            return None;
        }
        let mut frame = RECT::default();
        GetWindowRect(window, &mut frame).ok()?;
        let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
        // (A window as large as this is no window to take a picture of.)
        if width <= 0 || height <= 0 || width > 16_384 || height > 16_384 {
            return None;
        }
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(screen);
        ReleaseDC(None, screen);
        if memory.is_invalid() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // From the top down.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let picture = match CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => {
                let before = SelectObject(memory, bitmap);
                // (The second flag: what the compositor has of it, which
                // is all there is of most windows today.)
                let drawn = PrintWindow(window, memory, PRINT_WINDOW_FLAGS(2)).as_bool();
                let _ = GdiFlush();
                let found = drawn
                    .then(|| {
                        let pixels = std::slice::from_raw_parts(
                            bits as *const u8,
                            width as usize * height as usize * 4,
                        );
                        shrink(width as u32, height as u32, pixels, side)
                    })
                    .flatten();
                SelectObject(memory, before);
                let _ = DeleteObject(bitmap);
                found
            }
            Ok(bitmap) => {
                let _ = DeleteObject(bitmap);
                None
            }
            Err(_) => None,
        };
        let _ = DeleteDC(memory);
        picture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_made_small_and_its_colours_turned_round() {
        // Four by two, each pixel blue, green, red, spare; the first is
        // all blue.
        let mut pixels = Vec::new();
        for index in 0..8u8 {
            pixels.extend_from_slice(&[255 - index, index, index * 2, 0]);
        }
        let small = shrink(4, 2, &pixels, 2).expect("a picture");
        assert_eq!((small.width, small.height), (2, 1));
        // The first pixel: red 0, green 0, blue 255, solid.
        assert_eq!(&small.rgba[..4], &[0, 0, 255, 255]);
        // The second is the third of the first row.
        assert_eq!(&small.rgba[4..], &[4, 2, 253, 255]);
        // One that is small enough stays as large as it is.
        let same = shrink(4, 2, &pixels, 640).expect("a picture");
        assert_eq!((same.width, same.height, same.rgba.len()), (4, 2, 32));
        // Fewer bytes than the size says, or no size.
        assert!(shrink(4, 3, &pixels, 2).is_none());
        assert!(shrink(0, 2, &pixels, 2).is_none());
    }
}
