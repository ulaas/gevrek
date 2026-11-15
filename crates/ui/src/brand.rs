//! gevrek's own look: the mark, the program's icon and the brand's colours
//! (`assets/brand`, with its `USAGE.md`), for every program that draws
//! with this crate.
//!
//! The pictures are part of the program (read from `assets/brand` when it
//! is built), so nothing has to be installed for them.
//!
//! The brand's rules that are kept here: the mark of five panes from 48
//! pixels up and the one of three panes below; the mark for a dark ground
//! on a dark ground and the one for a light ground on a light one; a build
//! that is no release has the "nightly" icon.

use egui::{Color32, TextureHandle};

/// The brand's colours, by their names in `assets/brand/tokens`.
pub const MOLASSES: Color32 = Color32::from_rgb(0x21, 0x11, 0x0A);
pub const CRUST: Color32 = Color32::from_rgb(0xD9, 0x70, 0x1F);
pub const CRUST_DEEP: Color32 = Color32::from_rgb(0xC7, 0x5F, 0x14);
pub const EMBER: Color32 = Color32::from_rgb(0xE5, 0x8A, 0x3C);
pub const SESAME: Color32 = Color32::from_rgb(0xF4, 0xE2, 0xBC);
pub const SURFACE: Color32 = Color32::from_rgb(0x2E, 0x1A, 0x10);
pub const MUTED: Color32 = Color32::from_rgb(0xC4, 0xA4, 0x82);
pub const MUTED_ON_LIGHT: Color32 = Color32::from_rgb(0x5E, 0x3D, 0x26);

macro_rules! brand_file {
    ($path:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/brand/",
            $path
        ))
    };
}

/// The mark of three panes (for 32 pixels and under), drawn at twice that
/// for displays that are scaled.
const SMALL_ON_DARK: &[u8] = brand_file!("logo/mark/png/gevrek-mark-small-on-dark-64.png");
const SMALL_ON_LIGHT: &[u8] = brand_file!("logo/mark/png/gevrek-mark-small-on-light-64.png");
/// The mark of five panes.
const MARK_ON_DARK: &[u8] = brand_file!("logo/mark/png/gevrek-mark-on-dark-256.png");
const MARK_ON_LIGHT: &[u8] = brand_file!("logo/mark/png/gevrek-mark-on-light-256.png");

/// The program's icon as each system shapes it (a soft tile on Windows, a
/// circle on Linux, a rounded square on macOS), and its "nightly" twin.
#[cfg(windows)]
const ICON: (&[u8], &[u8]) = (
    brand_file!("app-icon/windows/png/gevrek-256.png"),
    brand_file!("app-icon/windows/png/gevrek-nightly-256.png"),
);
#[cfg(target_os = "macos")]
const ICON: (&[u8], &[u8]) = (
    brand_file!("app-icon/macos/png/gevrek-256.png"),
    brand_file!("app-icon/macos/png/gevrek-nightly-256.png"),
);
#[cfg(not(any(windows, target_os = "macos")))]
const ICON: (&[u8], &[u8]) = (
    brand_file!("app-icon/linux/hicolor/256x256/apps/gevrek.png"),
    brand_file!("app-icon/linux/hicolor/256x256/apps/gevrek-nightly.png"),
);

/// A picture read from a PNG: its pixels (red, green, blue and how solid,
/// not multiplied), its width and its height.
fn read(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let picture = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = picture.dimensions();
    Some((picture.into_raw(), width, height))
}

/// The icon a window of the program has (its title bar, the taskbar, the
/// window switcher): pixels, width, height. The "nightly" one in a build
/// that is no release.
pub fn window_icon() -> Option<(Vec<u8>, u32, u32)> {
    read(if cfg!(debug_assertions) {
        ICON.1
    } else {
        ICON.0
    })
}

/// The mark as something to draw: for a ground that is dark or light, at
/// `size` points (which chooses between the two marks).
pub fn mark(context: &egui::Context, on_dark: bool, size: f32) -> Option<TextureHandle> {
    let small = size < 48.0;
    let id = egui::Id::unique(("gevrek brand mark", on_dark, small));
    if let Some(known) = context.data(|data| data.get_temp::<TextureHandle>(id)) {
        return Some(known);
    }
    let (pixels, width, height) = read(match (small, on_dark) {
        (true, true) => SMALL_ON_DARK,
        (true, false) => SMALL_ON_LIGHT,
        (false, true) => MARK_ON_DARK,
        (false, false) => MARK_ON_LIGHT,
    })?;
    let texture = context.load_texture(
        format!("gevrek-mark-{on_dark}-{small}"),
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &pixels),
        egui::TextureOptions::LINEAR,
    );
    context.data_mut(|data| data.insert_temp(id, texture.clone()));
    Some(texture)
}

/// Whether a colour is a dark ground (the mark for a dark ground goes on
/// it).
pub fn is_dark(ground: Color32) -> bool {
    crate::theme::luminance(ground) < 0.35
}

/// Draw the mark, `size` points wide and high, on a ground of this colour.
/// Where the picture cannot be read nothing is drawn, and the room is kept.
pub fn show_mark(ui: &mut egui::Ui, size: f32, ground: Color32) -> egui::Response {
    match mark(ui.ctx(), is_dark(ground), size) {
        Some(texture) => ui.add(egui::Image::new((texture.id(), egui::vec2(size, size)))),
        None => ui.allocate_response(egui::vec2(size, size), egui::Sense::hover()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_of_the_brand_are_read() {
        // The marks: square, and see-through around the ring.
        for (png, side) in [
            (SMALL_ON_DARK, 64),
            (SMALL_ON_LIGHT, 64),
            (MARK_ON_DARK, 256),
            (MARK_ON_LIGHT, 256),
        ] {
            let (pixels, width, height) = read(png).expect("a picture");
            assert_eq!((width, height), (side, side));
            assert_eq!(pixels[3], 0, "the corner is see-through");
            assert!(pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] == 255));
        }
        // The icons: the release's and the nightly one are not the same.
        let (release, width, height) = read(ICON.0).expect("the icon");
        let (nightly, ..) = read(ICON.1).expect("the nightly icon");
        assert_eq!((width, height), (256, 256));
        assert_ne!(release, nightly);
        assert!(window_icon().is_some());
    }

    #[test]
    fn the_mark_for_a_dark_ground_goes_on_a_dark_ground() {
        assert!(is_dark(MOLASSES) && is_dark(SURFACE) && is_dark(Color32::BLACK));
        assert!(!is_dark(SESAME) && !is_dark(Color32::WHITE));
    }
}
