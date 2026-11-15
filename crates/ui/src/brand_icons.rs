//! The brand's own icons (`assets/brand/icons`, the symbolic set): the
//! signs of the panel's modules and of a window's buttons.
//!
//! They are drawings (SVG), one colour each, drawn once for every size
//! they are asked for at and coloured where they are shown. What the
//! pack has no sign for stays a sign of the icon font (`theme::icons`).

use egui::{Color32, TextureHandle};
use std::collections::HashMap;

macro_rules! pack {
    ($($name:ident => $file:literal),* $(,)?) => {
        /// One sign of the pack.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum BrandIcon {
            $($name),*
        }

        impl BrandIcon {
            /// Every sign of the pack.
            pub const ALL: &'static [BrandIcon] = &[$(BrandIcon::$name),*];

            /// The drawing, as the pack has it.
            fn drawing(self) -> &'static str {
                match self {
                    $(BrandIcon::$name => include_str!(concat!(
                        "../../../assets/brand/icons/symbolic/gevrek-",
                        $file,
                        "-symbolic.svg"
                    ))),*
                }
            }

            /// Its name in the pack.
            pub fn name(self) -> &'static str {
                match self {
                    $(BrandIcon::$name => $file),*
                }
            }
        }
    };
}

pack! {
    Close => "close",
    Minimize => "minimize",
    Maximize => "maximize",
    Restore => "restore",
    Fullscreen => "fullscreen",
    Float => "float",
    TileLeft => "tile-left",
    TileRight => "tile-right",
    MasterStack => "master-stack",
    Stacked => "stacked",
    Grid => "grid",
    Workspaces => "workspaces",
    Launcher => "launcher",
    Search => "search",
    Settings => "settings",
    Terminal => "terminal",
    Files => "files",
    Clock => "clock",
    Battery => "battery",
    Network => "network",
    Volume => "volume",
    Notifications => "notifications",
    Power => "power",
    Lock => "lock",
}

/// The sign drawn white on nothing, `side` pixels a side: four bytes a
/// pixel, the colours already multiplied by how solid they are. `None`
/// where the drawing cannot be read (which a test rules out).
pub fn drawn(icon: BrandIcon, side: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let side = side.clamp(1, 512);
    // (The pack draws in "the colour of the text around": white here,
    // coloured when it is shown.)
    let drawing = icon.drawing().replace("currentColor", "#ffffff");
    let tree = usvg::Tree::from_str(&drawing, &usvg::Options::default()).ok()?;
    let mut picture = tiny_skia::Pixmap::new(side, side)?;
    let from = tree.size();
    let scale = side as f32 / from.width().max(from.height()).max(1.0);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut picture.as_mut(),
    );
    Some(picture.take())
}

/// The pictures made so far, by sign and side in pixels.
#[derive(Clone, Default)]
struct Made(HashMap<(BrandIcon, u32), TextureHandle>);

/// The sign as a picture `size` points a side, sharp at the display's
/// scale. `None` only where it cannot be drawn.
pub fn texture(context: &egui::Context, icon: BrandIcon, size: f32) -> Option<TextureHandle> {
    let side = (size * context.pixels_per_point()).round().max(1.0) as u32;
    let key = (icon, side);
    let id = egui::Id::unique("gevrek-brand-icons");
    if let Some(made) = context.data(|data| data.get_temp::<Made>(id)) {
        if let Some(texture) = made.0.get(&key) {
            return Some(texture.clone());
        }
    }
    let pixels = drawn(icon, side)?;
    let image = egui::ColorImage::from_rgba_premultiplied([side as usize, side as usize], &pixels);
    let texture = context.load_texture(
        format!("gevrek-icon-{}-{side}", icon.name()),
        image,
        egui::TextureOptions::LINEAR,
    );
    context.data_mut(|data| {
        data.get_temp_mut_or_default::<Made>(id)
            .0
            .insert(key, texture.clone());
    });
    Some(texture)
}

/// The sign as something to show: `size` points a side, in `colour`.
pub fn image(
    context: &egui::Context,
    icon: BrandIcon,
    size: f32,
    colour: Color32,
) -> Option<egui::Image<'static>> {
    let texture = texture(context, icon, size)?;
    Some(
        egui::Image::new((texture.id(), egui::vec2(size, size)))
            .tint(colour)
            .fit_to_exact_size(egui::vec2(size, size)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sign_of_the_pack_is_drawn() {
        assert_eq!(BrandIcon::ALL.len(), 24);
        for icon in BrandIcon::ALL {
            let pixels = drawn(*icon, 32).unwrap_or_else(|| panic!("{} is not drawn", icon.name()));
            assert_eq!(pixels.len(), 32 * 32 * 4);
            let solid = pixels.chunks(4).filter(|pixel| pixel[3] > 128).count();
            // Lines on nothing: some of the square, by far not all of it.
            assert!(
                (20..700).contains(&solid),
                "{}: {solid} solid pixels of 1024",
                icon.name()
            );
            // White only, to be coloured where it is shown.
            assert!(pixels
                .chunks(4)
                .all(|pixel| pixel[0] == pixel[3] && pixel[1] == pixel[3]));
        }
    }
}
