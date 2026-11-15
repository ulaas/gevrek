//! The picture a program gives its own window (**xdg-toplevel-icon**): a
//! name out of the icon theme, or the pixels themselves. The shell shows
//! it on the window's button in the taskbar, in place of the picture it
//! would look for by the program's name.
//!
//! gevrek speaks the protocol itself. smithay's module for it ends a
//! program's connection when the program lets go of a picture's memory
//! after the icon made from it, which the protocol allows and Chrome
//! does (2026-11-13: Chrome did not start). Here the pixels are copied
//! out at the moment they are handed over, so nothing of the program's
//! is needed afterwards, in whatever order it lets go of things; and
//! what a program does out of turn with an icon is passed over, not
//! answered with the end of its connection.
//!
//! An icon of pixels is written as a PNG file into a folder of this
//! compositor's own in the session's runtime folder, and the window's
//! state names that file; an icon by name is passed on as the name.

use crate::state::State;
use crate::surface::WindowSurface;
use gevrek_engine::WindowHandle;
use smithay::reexports::wayland_protocols::xdg::toplevel_icon::v1::server::{
    xdg_toplevel_icon_manager_v1::{self, XdgToplevelIconManagerV1},
    xdg_toplevel_icon_v1::{self, XdgToplevelIconV1},
};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use std::path::PathBuf;
use std::sync::Mutex;

/// The sizes programs are told to bring, in pixels.
const SIZES: [i32; 2] = [32, 64];
/// A picture larger than this on a side is not taken.
const LARGEST: i32 = 1024;

/// One picture of an icon: its side in pixels and its pixels, four bytes
/// each, red first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    pub side: u32,
    pub rgba: Vec<u8>,
}

/// An icon as a program describes it, piece by piece.
#[derive(Debug, Default)]
pub struct Icon {
    name: String,
    pictures: Vec<Pixels>,
}

/// The picture of an icon to keep for a taskbar: the smallest that is at
/// least `wanted` on a side, else the largest there is.
pub fn chosen(pictures: &[Pixels], wanted: u32) -> Option<&Pixels> {
    pictures
        .iter()
        .filter(|picture| picture.side >= wanted)
        .min_by_key(|picture| picture.side)
        .or_else(|| pictures.iter().max_by_key(|picture| picture.side))
}

/// Pixels as a program hands them over (blue first, the colours already
/// multiplied by how solid they are when `solid_known`) as plain ones,
/// red first. `None` when the bytes are fewer than the size says.
pub fn plain(side: u32, stride: usize, bytes: &[u8], solid_known: bool) -> Option<Vec<u8>> {
    let side = side as usize;
    if stride < side * 4 || bytes.len() < stride * side.saturating_sub(1) + side * 4 {
        return None;
    }
    let mut rgba = Vec::with_capacity(side * side * 4);
    for row in 0..side {
        let line = &bytes[row * stride..row * stride + side * 4];
        for pixel in line.as_chunks::<4>().0 {
            let solid = if solid_known { pixel[3] } else { 255 };
            let back = |colour: u8| {
                if solid == 0 || solid == 255 {
                    colour
                } else {
                    ((u32::from(colour) * 255 + u32::from(solid) / 2) / u32::from(solid)).min(255)
                        as u8
                }
            };
            rgba.extend_from_slice(&[back(pixel[2]), back(pixel[1]), back(pixel[0]), solid]);
        }
    }
    Some(rgba)
}

pub fn init(display: &DisplayHandle) {
    display.create_global::<State, XdgToplevelIconManagerV1, ()>(1, ());
}

/// Where this compositor keeps the icons programs gave their windows.
fn folder() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    runtime.join(format!("gevrek-window-icons-{}", std::process::id()))
}

/// The file of a window's icon.
fn file_of(window: WindowHandle, serial: u32) -> PathBuf {
    folder().join(format!("{}-{serial}.png", window.0 as u64))
}

impl State {
    /// A program gave a window an icon, or took it back (`None`).
    fn window_icon(&mut self, id: WindowHandle, icon: Option<(String, Option<Pixels>)>) {
        let Some(window) = self.desktop.windows.get_mut(&id) else {
            return;
        };
        // The file of the icon it had goes.
        if window.icon.starts_with('/') {
            let _ = std::fs::remove_file(&window.icon);
        }
        window.icon = match icon {
            // Pixels: a file of them, which is sure to be there for the
            // shell (a name may be one the icon theme here has not).
            Some((_, Some(picture))) => {
                self.icon_serial = self.icon_serial.wrapping_add(1);
                let file = file_of(id, self.icon_serial);
                let written = std::fs::create_dir_all(folder()).is_ok()
                    && image::save_buffer(
                        &file,
                        &picture.rgba,
                        picture.side,
                        picture.side,
                        image::ColorType::Rgba8,
                    )
                    .is_ok();
                if written {
                    file.to_string_lossy().into_owned()
                } else {
                    log::warn!("a window's icon was not kept: {}", file.display());
                    String::new()
                }
            }
            Some((name, None)) => name,
            None => String::new(),
        };
        let surface = window.surface.clone();
        self.metadata_changed(&surface);
    }

    /// A window is gone: so is the file of its icon.
    pub(crate) fn forget_window_icon(&mut self, id: WindowHandle) {
        if let Some(window) = self.desktop.windows.get(&id) {
            if window.icon.starts_with('/') {
                let _ = std::fs::remove_file(&window.icon);
            }
        }
    }

    /// The compositor ends: the folder of the icons goes.
    pub fn forget_window_icons(&self) {
        let _ = std::fs::remove_dir_all(folder());
    }
}

impl GlobalDispatch<XdgToplevelIconManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<XdgToplevelIconManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        for size in SIZES {
            manager.icon_size(size);
        }
        manager.done();
    }
}

impl Dispatch<XdgToplevelIconManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &XdgToplevelIconManagerV1,
        request: xdg_toplevel_icon_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_toplevel_icon_manager_v1::Request::CreateIcon { id } => {
                init.init(id, Mutex::new(Icon::default()));
            }
            xdg_toplevel_icon_manager_v1::Request::SetIcon { toplevel, icon } => {
                let window = state
                    .desktop
                    .windows
                    .iter()
                    .find(|(_, window)| {
                        matches!(&window.surface, WindowSurface::Xdg(surface) if *surface.xdg_toplevel() == toplevel)
                    })
                    .map(|(id, _)| *id);
                let Some(window) = window else {
                    return;
                };
                let given = icon.and_then(|icon| {
                    let described = icon.data::<Mutex<Icon>>()?;
                    let described = described
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let picture = chosen(&described.pictures, SIZES[1] as u32).cloned();
                    // (An icon with neither a name nor a picture is none.)
                    (picture.is_some() || !described.name.is_empty())
                        .then(|| (described.name.clone(), picture))
                });
                state.window_icon(window, given);
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgToplevelIconV1, Mutex<Icon>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        icon: &XdgToplevelIconV1,
        request: xdg_toplevel_icon_v1::Request,
        described: &Mutex<Icon>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let mut described = described
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match request {
            xdg_toplevel_icon_v1::Request::SetName { icon_name } => described.name = icon_name,
            xdg_toplevel_icon_v1::Request::AddBuffer { buffer, scale } => {
                // The pixels are copied now: what the program does with
                // their memory afterwards is its own affair.
                let copied =
                    smithay::wayland::shm::with_buffer_contents(&buffer, |bytes, length, data| {
                        let solid_known = match data.format {
                            wl_shm::Format::Argb8888 => true,
                            wl_shm::Format::Xrgb8888 => false,
                            _ => return None,
                        };
                        if data.width != data.height || data.width <= 0 || data.width > LARGEST {
                            return None;
                        }
                        let from = data.offset.max(0) as usize;
                        let needed = data.stride.max(0) as usize * data.height as usize;
                        if from + needed > length {
                            return None;
                        }
                        // SAFETY: smithay hands over the pool's memory
                        // with its length, and holds it for this call;
                        // the range was checked against that length.
                        let pixels = unsafe { std::slice::from_raw_parts(bytes.add(from), needed) };
                        plain(data.width as u32, data.stride as usize, pixels, solid_known).map(
                            |rgba| Pixels {
                                side: data.width as u32,
                                rgba,
                            },
                        )
                    });
                match copied {
                    Ok(Some(picture)) => {
                        // (Its scale says how sharp it is: only the
                        // number of pixels matters for choosing one.)
                        let _ = scale;
                        described.pictures.push(picture);
                    }
                    // What the protocol calls a mistake of the program:
                    // memory that is not shared memory, or not square.
                    _ => icon.post_error(
                        xdg_toplevel_icon_v1::Error::InvalidBuffer,
                        "an icon's picture is square, of shared memory, with four bytes a pixel",
                    ),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_kept_is_the_smallest_that_is_enough() {
        let of = |side: u32| Pixels {
            side,
            rgba: Vec::new(),
        };
        let all = [of(16), of(128), of(64), of(32)];
        assert_eq!(chosen(&all, 64).map(|picture| picture.side), Some(64));
        assert_eq!(chosen(&all, 48).map(|picture| picture.side), Some(64));
        // None large enough: the largest.
        assert_eq!(chosen(&all[..1], 64).map(|picture| picture.side), Some(16));
        assert_eq!(chosen(&[], 64), None);
    }

    #[test]
    fn a_programs_pixels_become_plain_ones() {
        // Two by two, blue first, rows of twelve bytes (four to spare).
        let row = |pixels: [[u8; 4]; 2]| {
            let mut bytes: Vec<u8> = pixels.concat();
            bytes.extend_from_slice(&[9, 9, 9, 9]);
            bytes
        };
        let mut bytes = row([[10, 20, 30, 255], [0, 0, 128, 128]]);
        bytes.extend(row([[0, 0, 0, 0], [50, 60, 70, 255]]));
        let plain_ones = plain(2, 12, &bytes, true).expect("enough bytes");
        // Red first; half solid red comes back as full red, half solid.
        assert_eq!(&plain_ones[..4], &[30, 20, 10, 255]);
        assert_eq!(&plain_ones[4..8], &[255, 0, 0, 128]);
        assert_eq!(&plain_ones[8..12], &[0, 0, 0, 0]);
        assert_eq!(&plain_ones[12..], &[70, 60, 50, 255]);
        // Without solidness (the spare byte is anything): all solid.
        assert_eq!(plain(2, 12, &bytes, false).expect("enough")[7], 255);
        // Fewer bytes than the size says.
        assert_eq!(plain(2, 12, &bytes[..10], true), None);
        assert_eq!(plain(2, 4, &bytes, true), None);
    }
}
