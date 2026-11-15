//! Drawing of what surrounds a window: a soft shadow, a glow, and a border
//! with round corners that may run from one colour into another.
//!
//! The picture is made here, in plain memory, so that every backend shows
//! the same thing: the Windows backend puts it into a layered window behind
//! the program's window; another backend can hand it to its renderer.
//!
//! The picture covers the window's frame and a margin around it. The part
//! under the window itself is left empty (a window that lets light through
//! must not show its own shadow).

/// What to draw. Lengths are in pixels of the picture.
#[derive(Debug, Clone, PartialEq)]
pub struct DecorationSpec {
    /// Size of the window's visible frame.
    pub frame_width: i32,
    pub frame_height: i32,
    /// Room around the frame on every side; the picture is the frame plus
    /// twice this.
    pub margin: i32,
    /// Radius of the frame's corners.
    pub corner_radius: f32,
    /// The border: how far it reaches outside the frame's edge and inside it.
    pub border_outside: f32,
    pub border_inside: f32,
    pub border_color: [u8; 3],
    /// The colour the border runs into, if it is a gradient.
    pub border_color_end: Option<[u8; 3]>,
    /// Direction of the gradient in degrees (0: left to right, 90: downward).
    pub gradient_angle: f32,
    /// Strength of the border, 0-255 (0: no border is drawn).
    pub border_alpha: u8,
    pub glow: Option<Glow>,
    pub shadow: Option<Shadow>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    pub radius: f32,
    pub color: [u8; 3],
    pub alpha: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub radius: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub color: [u8; 3],
    pub alpha: u8,
}

/// A window as the effects see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecoratedWindow {
    /// Size of the window's visible frame.
    pub frame: (i32, i32),
    pub border_width: i32,
    pub border_offset: i32,
    pub border_color: (u8, u8, u8),
    pub border_alpha: u8,
    pub focused: bool,
}

impl DecorationSpec {
    /// Whether any of the drawn effects is switched on (otherwise a backend
    /// draws its plain border and nothing of this module).
    pub fn wanted(effects: &gevrek_model::settings::WindowEffectsProfile) -> bool {
        effects.shadow.enabled || effects.glow.enabled || effects.gradient.enabled
    }

    /// What to draw around a window, from the configuration's effects and
    /// the window's border. `gradient_angle` is the angle of the gradient
    /// now (a backend that turns it passes the turned angle).
    pub fn for_window(
        effects: &gevrek_model::settings::WindowEffectsProfile,
        window: DecoratedWindow,
        gradient_angle: f32,
    ) -> Self {
        // The band is as thick as the border: what does not reach outside
        // the frame lies inside it.
        let outside = (window.border_width + window.border_offset)
            .clamp(0, window.border_width.max(0)) as f32;
        let inside = (window.border_width as f32 - outside).max(0.0);
        let color = [
            window.border_color.0,
            window.border_color.1,
            window.border_color.2,
        ];
        let gradient = effects.gradient.enabled.then(|| {
            let end = if window.focused {
                effects.gradient.active_end
            } else {
                effects.gradient.inactive_end
            };
            [end.0, end.1, end.2]
        });
        let glow_alpha = if window.focused {
            effects.glow.opacity
        } else {
            effects.glow.inactive_opacity
        };
        let shadow_alpha = if window.focused {
            effects.shadow.opacity
        } else {
            effects.shadow.inactive_opacity
        };
        Self {
            frame_width: window.frame.0,
            frame_height: window.frame.1,
            margin: effects.margin() + outside.ceil() as i32 + 1,
            corner_radius: effects.corner_radius.0 as f32,
            border_outside: outside,
            border_inside: inside,
            border_color: color,
            border_color_end: gradient,
            gradient_angle,
            border_alpha: window.border_alpha,
            glow: (effects.glow.enabled && glow_alpha > 0).then(|| Glow {
                radius: effects.glow.radius as f32,
                color: effects
                    .glow
                    .color
                    .map(|(r, g, b)| [r, g, b])
                    .unwrap_or(color),
                alpha: glow_alpha,
            }),
            shadow: (effects.shadow.enabled && shadow_alpha > 0).then_some(Shadow {
                radius: effects.shadow.radius as f32,
                offset_x: effects.shadow.offset_x as f32,
                offset_y: effects.shadow.offset_y as f32,
                color: [
                    effects.shadow.color.0,
                    effects.shadow.color.1,
                    effects.shadow.color.2,
                ],
                alpha: shadow_alpha,
            }),
        }
    }

    /// The shadow alone under something that is no window (a surface of
    /// a shell: a panel, a launcher): the windows' shadow as the
    /// configuration describes it, whether it is on for them or not, at
    /// its strength under the focused window. `None` when that is nothing
    /// to see.
    pub fn shadow_under(
        effects: &gevrek_model::settings::WindowEffectsProfile,
        frame: (i32, i32),
        corner_radius: f32,
    ) -> Option<Self> {
        let shadow = &effects.shadow;
        if shadow.opacity == 0 || frame.0 <= 0 || frame.1 <= 0 {
            return None;
        }
        let reach = shadow.radius as i32 + shadow.offset_x.abs().max(shadow.offset_y.abs());
        Some(Self {
            frame_width: frame.0,
            frame_height: frame.1,
            margin: reach.clamp(0, 160) + 1,
            corner_radius,
            border_outside: 0.0,
            border_inside: 0.0,
            border_color: [0, 0, 0],
            border_color_end: None,
            gradient_angle: 0.0,
            border_alpha: 0,
            glow: None,
            shadow: Some(Shadow {
                radius: shadow.radius as f32,
                offset_x: shadow.offset_x as f32,
                offset_y: shadow.offset_y as f32,
                color: [shadow.color.0, shadow.color.1, shadow.color.2],
                alpha: shadow.opacity,
            }),
        })
    }

    pub fn width(&self) -> i32 {
        self.frame_width + self.margin * 2
    }

    pub fn height(&self) -> i32 {
        self.frame_height + self.margin * 2
    }
}

/// Distance of a point from the edge of a rectangle with round corners,
/// centred on the origin: negative inside, positive outside.
fn rounded_distance(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let radius = radius.min(half_w).min(half_h).max(0.0);
    let qx = px.abs() - (half_w - radius);
    let qy = py.abs() - (half_h - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// How much of a pixel lies at or below `edge`, for a pixel whose centre is
/// at distance `d`: 1 well inside, 0 well outside, a ramp one pixel wide.
fn coverage_below(d: f32, edge: f32) -> f32 {
    (edge - d + 0.5).clamp(0.0, 1.0)
}

/// A smooth fall from 1 at `t = 0` to 0 at `t = 1` (a bell's flank).
fn falloff(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let rest = 1.0 - t;
    rest * rest * (3.0 - 2.0 * rest) * rest
}

/// Put `color` at strength `alpha` (0-1) over a premultiplied pixel.
fn over(pixel: &mut [f32; 4], color: [u8; 3], alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let keep = 1.0 - alpha;
    pixel[0] = color[0] as f32 / 255.0 * alpha + pixel[0] * keep;
    pixel[1] = color[1] as f32 / 255.0 * alpha + pixel[1] * keep;
    pixel[2] = color[2] as f32 / 255.0 * alpha + pixel[2] * keep;
    pixel[3] = alpha + pixel[3] * keep;
}

/// One pixel of the border band, kept so that the border can be coloured
/// again (a turning gradient) without drawing everything else again.
#[derive(Debug, Clone, Copy)]
struct BandPixel {
    index: u32,
    /// Position relative to the middle of the frame.
    px: f32,
    py: f32,
    /// How much of the pixel the band covers, times the border's strength.
    strength: f32,
    /// How much of the pixel is not under the window.
    visible: f32,
    /// What is under the border here (shadow and glow), premultiplied.
    under: [f32; 4],
}

/// A picture that is drawn once and whose border can be turned afterwards.
#[derive(Debug, Clone)]
pub struct Prepared {
    spec: DecorationSpec,
    pixels: Vec<u32>,
    band: Vec<BandPixel>,
    reach_w: f32,
    reach_h: f32,
}

fn pack(pixel: [f32; 4], visible: f32) -> u32 {
    let a = (pixel[3] * visible).clamp(0.0, 1.0);
    if a <= 0.0 {
        return 0;
    }
    let scale = visible * 255.0;
    let alpha = (a * 255.0).round() as u32;
    let r = ((pixel[0] * scale).round().clamp(0.0, 255.0) as u32).min(alpha);
    let g = ((pixel[1] * scale).round().clamp(0.0, 255.0) as u32).min(alpha);
    let b = ((pixel[2] * scale).round().clamp(0.0, 255.0) as u32).min(alpha);
    alpha << 24 | r << 16 | g << 8 | b
}

impl Prepared {
    /// Draw `spec` (with the gradient at `spec.gradient_angle`).
    pub fn new(spec: &DecorationSpec) -> Self {
        let width = spec.width().max(0) as usize;
        let height = spec.height().max(0) as usize;
        let mut prepared = Self {
            spec: spec.clone(),
            pixels: vec![0u32; width * height],
            band: Vec::new(),
            reach_w: 0.0,
            reach_h: 0.0,
        };
        if width == 0 || height == 0 || spec.frame_width <= 0 || spec.frame_height <= 0 {
            return prepared;
        }
        let half_w = spec.frame_width as f32 / 2.0;
        let half_h = spec.frame_height as f32 / 2.0;
        let centre_x = spec.margin as f32 + half_w;
        let centre_y = spec.margin as f32 + half_h;
        let radius = spec.corner_radius.max(0.0);
        let inside = spec.border_inside.max(0.0);
        let outside = spec.border_outside.max(0.0);
        let border_alpha = spec.border_alpha as f32 / 255.0;
        prepared.reach_w = half_w + outside;
        prepared.reach_h = half_h + outside;

        // Everything drawn lies outside this rectangle: its rows are skipped
        // in one step, which is nearly all of a large window.
        let hollow_inset = (inside + radius + 2.0).ceil() as i32;
        let hollow_left = spec.margin + hollow_inset;
        let hollow_right = spec.margin + spec.frame_width - hollow_inset;
        let hollow_top = spec.margin + hollow_inset;
        let hollow_bottom = spec.margin + spec.frame_height - hollow_inset;

        for y in 0..height {
            let py = y as f32 + 0.5 - centre_y;
            let in_hollow_rows = (y as i32) >= hollow_top && (y as i32) < hollow_bottom;
            let mut x = 0usize;
            while x < width {
                if in_hollow_rows && (x as i32) >= hollow_left && (x as i32) < hollow_right {
                    x = hollow_right.max(0) as usize;
                    continue;
                }
                let px = x as f32 + 0.5 - centre_x;
                let distance = rounded_distance(px, py, half_w, half_h, radius);
                // Under the window: nothing.
                let visible = 1.0 - coverage_below(distance, -inside);
                if visible <= 0.0 {
                    x += 1;
                    continue;
                }
                let mut pixel = [0.0f32; 4];
                if let Some(shadow) = spec.shadow {
                    let d = rounded_distance(
                        px - shadow.offset_x,
                        py - shadow.offset_y,
                        half_w,
                        half_h,
                        radius,
                    );
                    let strength = if d <= 0.0 {
                        1.0
                    } else {
                        falloff(d / shadow.radius.max(1.0))
                    };
                    over(
                        &mut pixel,
                        shadow.color,
                        strength * shadow.alpha as f32 / 255.0,
                    );
                }
                if let Some(glow) = spec.glow {
                    let d = distance - outside;
                    let strength = if d <= 0.0 {
                        1.0
                    } else {
                        falloff(d / glow.radius.max(1.0))
                    };
                    over(&mut pixel, glow.color, strength * glow.alpha as f32 / 255.0);
                }
                let index = y * width + x;
                if border_alpha > 0.0 && (inside + outside) > 0.0 {
                    let band = coverage_below(distance, outside);
                    if band > 0.0 {
                        prepared.band.push(BandPixel {
                            index: index as u32,
                            px,
                            py,
                            strength: band * border_alpha,
                            visible,
                            under: pixel,
                        });
                        x += 1;
                        continue;
                    }
                }
                prepared.pixels[index] = pack(pixel, visible);
                x += 1;
            }
        }
        prepared.colour_border(spec.gradient_angle);
        prepared
    }

    fn colour_border(&mut self, angle: f32) {
        let start = self.spec.border_color;
        let end = self.spec.border_color_end;
        let radians = angle.to_radians();
        let (dir_x, dir_y) = (radians.cos(), radians.sin());
        // The gradient runs along this direction, from one end of the
        // frame to the other.
        let reach = self.reach_w * dir_x.abs() + self.reach_h * dir_y.abs();
        for band in &self.band {
            let color = match end {
                Some(end) if reach > 0.0 => {
                    let t =
                        ((band.px * dir_x + band.py * dir_y) / reach * 0.5 + 0.5).clamp(0.0, 1.0);
                    [
                        (start[0] as f32 + (end[0] as f32 - start[0] as f32) * t) as u8,
                        (start[1] as f32 + (end[1] as f32 - start[1] as f32) * t) as u8,
                        (start[2] as f32 + (end[2] as f32 - start[2] as f32) * t) as u8,
                    ]
                }
                _ => start,
            };
            let mut pixel = band.under;
            over(&mut pixel, color, band.strength);
            self.pixels[band.index as usize] = pack(pixel, band.visible);
        }
        self.spec.gradient_angle = angle;
    }

    /// Turn the gradient of the border to `angle` degrees. Only the border
    /// is drawn again.
    pub fn turn(&mut self, angle: f32) {
        if self.spec.border_color_end.is_some() && self.spec.gradient_angle != angle {
            self.colour_border(angle);
        }
    }

    /// Whether this picture shows `spec`, the direction of the gradient
    /// left aside.
    pub fn shows(&self, spec: &DecorationSpec) -> bool {
        let mut mine = self.spec.clone();
        mine.gradient_angle = spec.gradient_angle;
        mine == *spec
    }

    pub fn spec(&self) -> &DecorationSpec {
        &self.spec
    }

    /// Give the picture's memory back and keep only what it shows. (A window
    /// keeps the picture once it has been handed over; the copy here is
    /// needed again only to turn the border.) A picture without its memory
    /// cannot be turned: it is drawn anew.
    pub fn release(&mut self) {
        self.pixels = Vec::new();
        self.band = Vec::new();
    }

    pub fn is_released(&self) -> bool {
        self.pixels.is_empty()
    }

    /// `width() * height()` pixels, row after row from the top, each
    /// `0xAARRGGBB` with the colour already multiplied by its alpha (what a
    /// layered window wants).
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Write the border's pixels, and only those, into a picture's bytes
    /// (four a pixel, as [`Self::pixels`] has them, least byte first): what
    /// a turn of the border changed of a picture that is there already.
    pub fn write_band(&self, bytes: &mut [u8]) {
        for band in &self.band {
            let at = band.index as usize * 4;
            if let (Some(pixel), Some(place)) = (
                self.pixels.get(band.index as usize),
                bytes.get_mut(at..at + 4),
            ) {
                place.copy_from_slice(&pixel.to_le_bytes());
            }
        }
    }

    /// The parts of the picture the border lies in, as `(x, y, width,
    /// height)`: a strip along each side. Nothing outside them changes
    /// when the border is turned.
    pub fn band_strips(&self) -> Vec<(i32, i32, i32, i32)> {
        let width = self.spec.width();
        if width <= 0 || self.band.is_empty() {
            return Vec::new();
        }
        let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, 0, 0);
        for band in &self.band {
            let (x, y) = (band.index as i32 % width, band.index as i32 / width);
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
        // As thick as the border, and at the top and the bottom as high
        // as the corners are round besides.
        let thick = (self.spec.border_outside + self.spec.border_inside).ceil() as i32 + 2;
        let high = (thick + self.spec.corner_radius.ceil() as i32).min((bottom - top + 1) / 2);
        let thick = thick.min((right - left + 1) / 2);
        let between = (bottom - top - 2 * high).max(0);
        let mut strips = vec![
            (left, top, right - left, high),
            (left, bottom - high, right - left, high),
        ];
        if between > 0 {
            strips.push((left, top + high, thick, between));
            strips.push((right - thick, top + high, thick, between));
        }
        strips
    }
}

/// Draw `spec` once. See [`Prepared::pixels`] for the form of the result.
pub fn render(spec: &DecorationSpec) -> Vec<u32> {
    Prepared::new(spec).pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> DecorationSpec {
        DecorationSpec {
            frame_width: 200,
            frame_height: 120,
            margin: 30,
            corner_radius: 10.0,
            border_outside: 3.0,
            border_inside: 1.0,
            border_color: [0, 170, 255],
            border_color_end: None,
            gradient_angle: 0.0,
            border_alpha: 255,
            glow: None,
            shadow: None,
        }
    }

    fn at(spec: &DecorationSpec, pixels: &[u32], x: i32, y: i32) -> (u32, u32, u32, u32) {
        let p = pixels[(y * spec.width() + x) as usize];
        (p >> 24, p >> 16 & 255, p >> 8 & 255, p & 255)
    }

    #[test]
    fn the_border_is_a_band_around_the_frame_and_nothing_is_under_the_window() {
        let spec = spec();
        let pixels = render(&spec);
        assert_eq!(pixels.len(), (260 * 180) as usize);
        // The middle of the left edge: the band reaches 3 outside, 1 inside.
        let y = spec.height() / 2;
        assert_eq!(at(&spec, &pixels, 30 - 2, y), (255, 0, 170, 255));
        assert_eq!(at(&spec, &pixels, 30, y), (255, 0, 170, 255));
        assert_eq!(at(&spec, &pixels, 30 - 5, y).0, 0, "outside the band");
        assert_eq!(at(&spec, &pixels, 30 + 3, y).0, 0, "under the window");
        assert_eq!(at(&spec, &pixels, 130, 90).0, 0, "the middle of the window");
        // No pixel has more colour than alpha (premultiplied).
        for p in &pixels {
            let a = p >> 24;
            assert!(p >> 16 & 255 <= a && p >> 8 & 255 <= a && p & 255 <= a);
        }
    }

    #[test]
    fn corners_are_round() {
        let spec = spec();
        let pixels = render(&spec);
        // The very corner of the frame's bounding box is outside a round corner.
        assert_eq!(at(&spec, &pixels, 30 - 2, 30 - 2).0, 0);
        // On the diagonal the band is met further in.
        assert!((0..14).any(|i| at(&spec, &pixels, 30 + i, 30 + i).0 == 255));
    }

    #[test]
    fn a_shadow_falls_off_with_distance_and_follows_its_offset() {
        let mut spec = spec();
        spec.border_alpha = 0;
        spec.shadow = Some(Shadow {
            radius: 24.0,
            offset_x: 0.0,
            offset_y: 8.0,
            color: [0, 0, 0],
            alpha: 200,
        });
        let pixels = render(&spec);
        let x = spec.width() / 2;
        let bottom = 30 + 120;
        let near = at(&spec, &pixels, x, bottom + 4).0;
        let far = at(&spec, &pixels, x, bottom + 20).0;
        assert!(near > far && far > 0, "near {near}, far {far}");
        // Offset downward: more shadow below the window than above it.
        let above = at(&spec, &pixels, x, 30 - 8).0;
        let below = at(&spec, &pixels, x, bottom + 8).0;
        assert!(below > above, "below {below}, above {above}");
        assert_eq!(at(&spec, &pixels, x, 90).0, 0, "no shadow under the window");
    }

    #[test]
    fn a_glow_has_its_colour_and_fades_out() {
        let mut spec = spec();
        spec.glow = Some(Glow {
            radius: 20.0,
            color: [255, 0, 0],
            alpha: 255,
        });
        let pixels = render(&spec);
        let y = spec.height() / 2;
        let (a1, r1, g1, _) = at(&spec, &pixels, 30 - 8, y);
        let (a2, _, _, _) = at(&spec, &pixels, 30 - 18, y);
        assert!(a1 > a2 && a2 > 0);
        assert!(r1 > 0 && g1 == 0, "the glow is red");
        assert_eq!(at(&spec, &pixels, 2, y).0, 0, "beyond its radius");
    }

    #[test]
    fn a_gradient_border_runs_from_one_colour_into_the_other() {
        let mut spec = spec();
        spec.border_color = [255, 0, 0];
        spec.border_color_end = Some([0, 0, 255]);
        let pixels = render(&spec);
        let y = spec.height() / 2;
        let left = at(&spec, &pixels, 30 - 1, y);
        let right = at(&spec, &pixels, 30 + 200, y);
        assert!(
            left.1 > 200 && left.3 < 50,
            "the left edge is red: {left:?}"
        );
        assert!(
            right.3 > 200 && right.1 < 50,
            "the right edge is blue: {right:?}"
        );
        // Turned by 90 degrees the run is from top to bottom.
        spec.gradient_angle = 90.0;
        let pixels = render(&spec);
        let x = spec.width() / 2;
        assert!(at(&spec, &pixels, x, 30 - 1).1 > 200);
        assert!(at(&spec, &pixels, x, 30 + 120).3 > 200);
    }

    #[test]
    fn turning_the_border_gives_the_picture_drawn_at_that_angle() {
        let mut spec = spec();
        spec.border_color_end = Some([255, 0, 200]);
        spec.glow = Some(Glow {
            radius: 12.0,
            color: [0, 170, 255],
            alpha: 160,
        });
        spec.shadow = Some(Shadow {
            radius: 20.0,
            offset_x: 0.0,
            offset_y: 6.0,
            color: [0, 0, 0],
            alpha: 150,
        });
        let mut prepared = Prepared::new(&spec);
        prepared.turn(135.0);
        spec.gradient_angle = 135.0;
        assert!(prepared.shows(&spec));
        assert_eq!(prepared.pixels(), render(&spec).as_slice());
        // A picture of another window is not this one.
        spec.frame_width += 1;
        assert!(!prepared.shows(&spec));
    }

    #[test]
    fn a_frame_too_small_to_draw_gives_an_empty_picture() {
        let mut spec = spec();
        spec.frame_width = 0;
        assert!(render(&spec).iter().all(|pixel| *pixel == 0));
    }

    #[test]
    fn a_turn_changes_nothing_outside_the_borders_strips() {
        let mut spec = spec();
        spec.border_color_end = Some([255, 0, 0]);
        spec.gradient_angle = 0.0;
        let mut prepared = Prepared::new(&spec);
        let before = prepared.pixels().to_vec();
        prepared.turn(120.0);
        let after = prepared.pixels();
        let strips = prepared.band_strips();
        assert!(!strips.is_empty());
        let width = spec.width();
        let inside = |index: usize| {
            let (x, y) = (index as i32 % width, index as i32 / width);
            strips
                .iter()
                .any(|(sx, sy, sw, sh)| x >= *sx && x < sx + sw && y >= *sy && y < sy + sh)
        };
        let changed: Vec<usize> = (0..before.len())
            .filter(|i| before[*i] != after[*i])
            .collect();
        assert!(!changed.is_empty(), "the turn recoloured the border");
        assert!(
            changed.iter().all(|index| inside(*index)),
            "only inside the strips"
        );
        // The strips are far less than the picture.
        let area: i32 = strips.iter().map(|(_, _, w, h)| w * h).sum();
        assert!(
            area * 2 < width * spec.height(),
            "{area} of {}",
            width * spec.height()
        );
        // And what is written into a picture's bytes is the border alone.
        let mut bytes: Vec<u8> = before
            .iter()
            .flat_map(|pixel| pixel.to_le_bytes())
            .collect();
        prepared.write_band(&mut bytes);
        let wanted: Vec<u8> = after.iter().flat_map(|pixel| pixel.to_le_bytes()).collect();
        assert_eq!(bytes, wanted);
    }
}
