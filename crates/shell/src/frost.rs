//! The look of the shell's larger surfaces (the control centre, the
//! notifications' panel): frosted glass, and how they come and go.
//!
//! *Frosted*: behind what the surface shows lies what is on its display,
//! blurred, under a dark tint, so that it looks like a pane of glass
//! over the desktop: a picture of the display as it was when the
//! surface was opened, with its windows (taken as a screenshot is, and
//! blurred by the shell); until that picture has come, and where none
//! can be had, the wallpaper blurred, read by the shell itself. The
//! picture is not taken again while the surface is open: what moves
//! behind the glass stands still in it.
//!
//! *Coming and going*: a surface slides in from its edge while it grows
//! solid, and goes the same way back.

// (On Windows the shell's surfaces are the system's own windows, drawn
// plain: most of this waits there.)
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::time::{Duration, Instant};

/// The longer side of the blurred picture that is kept: it is shown
/// large again, which is soft anyway.
const SIDE: u32 = 512;
/// How long a surface takes to come, and to go.
const COMING: Duration = Duration::from_millis(220);
const GOING: Duration = Duration::from_millis(160);
/// How far it slides, in points.
const SLIDE: f32 = 36.0;

/// Read a picture and blur it, small. (Takes a moment: beside what draws.)
#[cfg(target_os = "linux")]
pub fn frosted(file: &std::path::Path) -> Option<egui::ColorImage> {
    Some(soft(image::open(file).ok()?))
}

/// A picture of a display (its size and its pixels, four bytes each,
/// red first), blurred and small. `None`: the bytes are no such picture.
#[cfg(target_os = "linux")]
pub fn frosted_scene(width: u32, height: u32, pixels: Vec<u8>) -> Option<egui::ColorImage> {
    let picture = image::RgbaImage::from_raw(width, height, pixels)?;
    Some(soft(image::DynamicImage::ImageRgba8(picture)))
}

#[cfg(target_os = "linux")]
fn soft(picture: image::DynamicImage) -> egui::ColorImage {
    let small = picture.thumbnail(SIDE, SIDE).to_rgba8();
    let soft = image::imageops::blur(&small, 9.0);
    let size = [soft.width() as usize, soft.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, soft.as_raw())
}

/// The part of a picture that lies behind `pane` (a rectangle on its
/// display, which is `display` large) when the picture covers the
/// display keeping its shape: from where to where, in parts of the
/// picture.
pub fn part_behind(pane: egui::Rect, display: egui::Vec2, picture: egui::Vec2) -> egui::Rect {
    let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if display.x <= 0.0 || display.y <= 0.0 || picture.x <= 0.0 || picture.y <= 0.0 {
        return whole;
    }
    // The picture is scaled until it covers the display; what is over is
    // cut off evenly at both sides.
    let scale = (display.x / picture.x).max(display.y / picture.y);
    let shown = egui::vec2(display.x / scale, display.y / scale);
    let first = egui::vec2((picture.x - shown.x) / 2.0, (picture.y - shown.y) / 2.0);
    let at = |x: f32, y: f32| {
        egui::pos2(
            ((first.x + x / scale) / picture.x).clamp(0.0, 1.0),
            ((first.y + y / scale) / picture.y).clamp(0.0, 1.0),
        )
    };
    egui::Rect::from_min_max(at(pane.left(), pane.top()), at(pane.right(), pane.bottom()))
}

/// A surface coming, there, or going.
#[derive(Debug, Clone, Copy)]
pub struct Coming {
    since: Instant,
    leaving: Option<Instant>,
}

impl Coming {
    pub fn new(now: Instant) -> Self {
        Self {
            since: now,
            leaving: None,
        }
    }

    /// Begin to go (once: asked again, it goes on going).
    pub fn leave(&mut self, now: Instant) {
        if self.leaving.is_none() {
            self.leaving = Some(now);
        }
    }

    #[cfg(test)]
    pub fn is_leaving(&self) -> bool {
        self.leaving.is_some()
    }

    /// How much of it is there, from 0 to 1. `moving`: with the
    /// animations off it is there at once and gone at once.
    pub fn shown(&self, now: Instant, moving: bool) -> f32 {
        if !moving {
            return if self.leaving.is_some() { 0.0 } else { 1.0 };
        }
        let part = |since: Instant, long: Duration| {
            (now.saturating_duration_since(since).as_secs_f32() / long.as_secs_f32())
                .clamp(0.0, 1.0)
        };
        let eased = |part: f32| part * part * (3.0 - 2.0 * part);
        match self.leaving {
            // (From however far it had come.)
            Some(leaving) => {
                let had = eased(part(self.since, COMING).min(
                    leaving.saturating_duration_since(self.since).as_secs_f32()
                        / COMING.as_secs_f32(),
                ));
                had * (1.0 - eased(part(leaving, GOING)))
            }
            None => eased(part(self.since, COMING)),
        }
    }

    /// It has gone: the surface can be closed.
    pub fn gone(&self, now: Instant, moving: bool) -> bool {
        self.leaving
            .is_some_and(|leaving| !moving || now.saturating_duration_since(leaving) >= GOING)
    }
}

/// How a surface looks in this frame.
pub struct Look<'a> {
    /// The wallpaper, blurred, where there is one and glass is wanted.
    pub glass: Option<&'a egui::TextureHandle>,
    /// How dark the glass is, 0 to 255.
    pub tint: u8,
    /// Where the surface's top left corner is on its display, and how
    /// large the display is (points): for the part of the wallpaper
    /// that is behind it.
    pub origin: egui::Pos2,
    pub display: egui::Vec2,
    /// How much of it is there (see [`Coming::shown`]).
    pub shown: f32,
    /// It slides in from the right (its edge), else from the left.
    pub from_right: bool,
}

impl Look<'_> {
    /// A plain look: no glass, all there. (For pictures in tests, and
    /// where the shell's surfaces are the system's own windows.)
    #[allow(dead_code)]
    pub fn plain() -> Look<'static> {
        Look {
            glass: None,
            tint: 170,
            origin: egui::Pos2::ZERO,
            display: egui::Vec2::ZERO,
            shown: 1.0,
            from_right: true,
        }
    }

    /// Paint the surface's ground into `ui` (all of it) and say where
    /// what stands on it goes: the card, shifted and less solid while
    /// it comes or goes. What is drawn into `ui` after this is as solid
    /// as the card.
    pub fn ground(
        &self,
        ui: &mut egui::Ui,
        palette: &gevrek_ui::theme::Palette,
        radius: u8,
    ) -> egui::Rect {
        let aside = (1.0 - self.shown) * SLIDE * if self.from_right { 1.0 } else { -1.0 };
        let card = ui.max_rect().translate(egui::vec2(aside, 0.0));
        ui.multiply_opacity(self.shown.clamp(0.0, 1.0));
        if self.shown < 1.0 {
            ui.ctx().request_repaint();
        }
        let painter = ui.painter().clone();
        match self.glass {
            Some(texture) => {
                let behind = card.translate(self.origin.to_vec2());
                let part = part_behind(behind, self.display, texture.size_vec2());
                painter.add(
                    egui::epaint::RectShape::filled(card, radius, egui::Color32::WHITE)
                        .with_texture(texture.id(), part),
                );
                let (red, green, blue, _) = palette.bg.to_tuple();
                painter.rect_filled(
                    card,
                    radius,
                    egui::Color32::from_rgba_unmultiplied(red, green, blue, self.tint),
                );
            }
            None => {
                painter.rect_filled(card, radius, palette.surface);
            }
        }
        painter.rect_stroke(
            card,
            radius,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(26)),
            egui::StrokeKind::Inside,
        );
        card
    }
}

/// The blurred wallpaper as something this surface can draw, made once
/// for each picture (`stamp` is another for another picture).
pub fn texture(
    context: &egui::Context,
    picture: Option<&(u64, std::sync::Arc<egui::ColorImage>)>,
) -> Option<egui::TextureHandle> {
    let (stamp, picture) = picture?;
    let key = egui::Id::unique("gevrek frosted wallpaper");
    let known: Option<(u64, egui::TextureHandle)> = context.data(|data| data.get_temp(key));
    if let Some((had, texture)) = known {
        if had == *stamp {
            return Some(texture);
        }
    }
    let texture = context.load_texture(
        "frosted-wallpaper",
        egui::ColorImage::clone(picture),
        egui::TextureOptions::LINEAR,
    );
    context.data_mut(|data| data.insert_temp(key, (*stamp, texture.clone())));
    Some(texture)
}

/// The size of the display that has the keyboard, in points, as the
/// window manager's state tells it.
pub fn display_with_keyboard(state: &serde_json::Value) -> Option<egui::Vec2> {
    let monitors = state["monitors"].as_array()?;
    let monitor = monitors
        .iter()
        .find(|monitor| monitor["is_focused"].as_bool().unwrap_or(false))
        .or_else(|| monitors.first())?;
    let size = &monitor["size"];
    let (width, height) = (size["width"].as_f64()?, size["height"].as_f64()?);
    (width > 0.0 && height > 0.0).then(|| egui::vec2(width as f32, height as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glass_shows_the_part_of_the_wallpaper_that_is_behind_it() {
        // A picture of the display's own shape: the pane's own part.
        let display = egui::vec2(1920.0, 1080.0);
        let pane = egui::Rect::from_min_size(egui::pos2(1528.0, 12.0), egui::vec2(380.0, 1056.0));
        let part = part_behind(pane, display, egui::vec2(480.0, 270.0));
        assert!((part.left() - 1528.0 / 1920.0).abs() < 0.001);
        assert!((part.right() - 1908.0 / 1920.0).abs() < 0.001);
        assert!((part.top() - 12.0 / 1080.0).abs() < 0.001);
        // A square picture on a wide display: its middle band is shown,
        // so the top of the display is a quarter down the picture or so.
        let part = part_behind(
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0)),
            display,
            egui::vec2(400.0, 400.0),
        );
        assert!((part.left() - 0.0).abs() < 0.001 && (part.right() - 1.0).abs() < 0.001);
        assert!((part.top() - 0.21875).abs() < 0.001 && (part.bottom() - 0.78125).abs() < 0.001);
        // Nothing known: all of it.
        assert_eq!(
            part_behind(pane, egui::Vec2::ZERO, egui::vec2(4.0, 4.0)).max,
            egui::pos2(1.0, 1.0)
        );
    }

    #[test]
    fn a_surface_comes_stays_and_goes() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut coming = Coming::new(start);
        assert_eq!(coming.shown(start, true), 0.0);
        let half = coming.shown(at(110), true);
        assert!(half > 0.3 && half < 0.7, "{half}");
        assert_eq!(coming.shown(at(400), true), 1.0);
        assert!(!coming.gone(at(400), true));
        coming.leave(at(1000));
        coming.leave(at(1050));
        assert!(coming.is_leaving());
        let going = coming.shown(at(1080), true);
        assert!(going > 0.2 && going < 0.8, "{going}");
        assert!(!coming.gone(at(1080), true));
        assert_eq!(coming.shown(at(1200), true), 0.0);
        assert!(coming.gone(at(1200), true));
        // Asked to go while still coming: from where it had come, not from whole.
        let mut early = Coming::new(start);
        early.leave(at(60));
        assert!(early.shown(at(61), true) < 0.4);
        // Without the animations: there, and gone.
        let mut still = Coming::new(start);
        assert_eq!(still.shown(start, false), 1.0);
        still.leave(at(10));
        assert_eq!(still.shown(at(10), false), 0.0);
        assert!(still.gone(at(10), false));
    }

    #[test]
    fn the_display_with_the_keyboard_is_found_in_the_state() {
        let state = serde_json::json!({"monitors": [
            {"is_focused": false, "size": {"width": 1280, "height": 720}},
            {"is_focused": true, "size": {"width": 2560, "height": 1440}}
        ]});
        assert_eq!(
            display_with_keyboard(&state),
            Some(egui::vec2(2560.0, 1440.0))
        );
        assert_eq!(display_with_keyboard(&serde_json::json!({})), None);
    }
}
