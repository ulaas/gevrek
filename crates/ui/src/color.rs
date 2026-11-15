//! Colours as text and back: CSS colour strings (`#rgb`, `#rrggbbaa`, `rgb(...)`,
//! named colours) and egui colours.

use egui::Color32;

/// Parses a CSS colour (`#rgb`, `#rrggbbaa`, `rgb(…)`, named colours…).
pub fn parse(s: &str) -> Option<Color32> {
    let c = csscolorparser::parse(s).ok()?;
    let [r, g, b, a] = c.to_rgba8();
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

/// Formats as `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn format(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// An opaque colour from the `(r, g, b)` the configuration stores.
pub fn from_rgb(rgb: (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// The `(r, g, b)` of a colour, for the configuration (alpha is dropped).
pub fn to_rgb(c: Color32) -> (u8, u8, u8) {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    (r, g, b)
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let (a, b) = (a.to_srgba_unmultiplied(), b.to_srgba_unmultiplied());
    let m = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(0), m(1), m(2), m(3))
}
