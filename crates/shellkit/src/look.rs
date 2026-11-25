//! How a surface looks beyond what is drawn on it: what lies behind
//! parts of it shown out of focus.
//!
//! A surface may cover a whole display and draw several things on it,
//! each with a ground of its own that is seen through (the zones of a
//! desktop). The desktop behind such a ground is to be soft, and behind
//! the rest of the surface as it is: so what is asked for is a list of
//! parts, each a rectangle with round corners, not a look of the whole
//! surface. Who draws the surface says where its grounds are, each time
//! they change ([`crate::host::Control::set_blur_regions`]).
//!
//! No system is called here: this is what both halves of the host share,
//! the parts as whole pixels and as the rows a region is made of.

/// A part of a surface behind which the desktop is shown soft: a
/// rectangle in the surface's own points, with corners of this radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurRegion {
    pub rect: egui::Rect,
    pub radius: f32,
}

impl BlurRegion {
    pub fn new(rect: egui::Rect, radius: f32) -> Self {
        Self { rect, radius }
    }
}

/// A part in whole units: the display's pixels (Windows, where a window
/// is measured in them), or logical pixels (Wayland, where a surface is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rounded {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub radius: i32,
}

/// A part in whole units at `scale` units to a point. Its edges are
/// rounded each by itself (not its size), so two parts that touch in
/// points touch in pixels at every scale. The radius is never more than
/// half the shorter side. `None` for a part of no size, or one that is no
/// rectangle at all (a number that is none).
pub fn rounded(region: &BlurRegion, scale: f32) -> Option<Rounded> {
    let rect = region.rect;
    let numbers = [rect.min.x, rect.min.y, rect.max.x, rect.max.y, scale];
    if numbers.iter().any(|number| !number.is_finite()) || scale <= 0.0 {
        return None;
    }
    let edge = |points: f32| (points * scale).round().clamp(-1.0e7, 1.0e7) as i32;
    let (left, top, right, bottom) = (
        edge(rect.min.x),
        edge(rect.min.y),
        edge(rect.max.x),
        edge(rect.max.y),
    );
    let (width, height) = (right - left, bottom - top);
    if width <= 0 || height <= 0 {
        return None;
    }
    let radius = if region.radius.is_finite() {
        (region.radius * scale).round().max(0.0) as i32
    } else {
        0
    };
    Some(Rounded {
        x: left,
        y: top,
        width,
        height,
        radius: radius.min(width / 2).min(height / 2),
    })
}

/// A part as rows: rectangles (x, y, width, height) that lie under each
/// other, do not overlap, and together are the rectangle with its corners
/// cut round. For a system that takes a region as rectangles (a Wayland
/// region). Rows of the same width are one rectangle, so a part without
/// round corners is one, and one with them is a few more than twice its
/// radius at the most.
pub fn strips(shape: Rounded) -> Vec<(i32, i32, i32, i32)> {
    let radius = shape.radius.clamp(0, shape.width / 2).min(shape.height / 2);
    if shape.width <= 0 || shape.height <= 0 {
        return Vec::new();
    }
    // How far a row of a corner stands in from the side: by the circle,
    // taken at the middle of the row.
    let inset = |row: i32| {
        let down = radius as f32 - row as f32 - 0.5;
        let across = ((radius * radius) as f32 - down * down).max(0.0).sqrt();
        ((radius as f32 - across).round() as i32).clamp(0, radius)
    };
    let mut rows: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut push = |y: i32, height: i32, inset: i32| {
        let width = shape.width - 2 * inset;
        if height <= 0 || width <= 0 {
            return;
        }
        match rows.last_mut() {
            // (The same width as the row above: one rectangle.)
            Some(last) if last.0 == shape.x + inset && last.2 == width && last.1 + last.3 == y => {
                last.3 += height;
            }
            _ => rows.push((shape.x + inset, y, width, height)),
        }
    };
    for row in 0..radius {
        push(shape.y + row, 1, inset(row));
    }
    push(shape.y + radius, shape.height - 2 * radius, 0);
    for row in (0..radius).rev() {
        push(shape.y + shape.height - 1 - row, 1, inset(row));
    }
    rows
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
    fn a_part_is_as_many_pixels_as_its_display_has_to_a_point() {
        let part = region(10.0, 20.0, 300.0, 200.0, 12.0);
        let at = |scale: f32| rounded(&part, scale).expect("a part");
        assert_eq!(
            at(1.0),
            Rounded {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
                radius: 12
            }
        );
        assert_eq!(
            at(1.5),
            Rounded {
                x: 15,
                y: 30,
                width: 450,
                height: 300,
                radius: 18
            }
        );
        assert_eq!(
            at(2.0),
            Rounded {
                x: 20,
                y: 40,
                width: 600,
                height: 400,
                radius: 24
            }
        );
        // A quarter more: the edges are rounded, each by itself.
        assert_eq!(
            at(1.25),
            Rounded {
                x: 13,
                y: 25,
                width: 375,
                height: 250,
                radius: 15
            }
        );
    }

    #[test]
    fn parts_that_touch_in_points_touch_in_pixels_at_every_scale() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let left = rounded(&region(0.3, 0.0, 100.4, 50.0, 0.0), scale).expect("left");
            let right = rounded(&region(100.7, 0.0, 80.1, 50.0, 0.0), scale).expect("right");
            assert_eq!(left.x + left.width, right.x, "at {scale}");
        }
    }

    #[test]
    fn a_part_of_no_size_or_of_no_numbers_is_none() {
        assert_eq!(rounded(&region(5.0, 5.0, 0.0, 10.0, 4.0), 1.0), None);
        assert_eq!(rounded(&region(5.0, 5.0, 10.0, -3.0, 4.0), 1.0), None);
        assert_eq!(rounded(&region(5.0, 5.0, 0.2, 10.0, 0.0), 1.0), None);
        assert_eq!(rounded(&region(f32::NAN, 5.0, 10.0, 10.0, 0.0), 1.0), None);
        assert_eq!(
            rounded(&region(5.0, 5.0, f32::INFINITY, 10.0, 0.0), 1.0),
            None
        );
        assert_eq!(rounded(&region(5.0, 5.0, 10.0, 10.0, 0.0), 0.0), None);
        // A radius that is no number is no radius.
        let plain = rounded(&region(0.0, 0.0, 10.0, 10.0, f32::NAN), 1.0).expect("a part");
        assert_eq!(plain.radius, 0);
        let plain = rounded(&region(0.0, 0.0, 10.0, 10.0, -4.0), 1.0).expect("a part");
        assert_eq!(plain.radius, 0);
    }

    #[test]
    fn a_radius_is_never_more_than_half_the_shorter_side() {
        let pill = rounded(&region(0.0, 0.0, 200.0, 30.0, 400.0), 1.0).expect("a part");
        assert_eq!(pill.radius, 15);
        let tall = rounded(&region(0.0, 0.0, 9.0, 200.0, 400.0), 2.0).expect("a part");
        assert_eq!(tall.radius, 9);
    }

    #[test]
    fn a_part_without_round_corners_is_one_rectangle() {
        let shape = Rounded {
            x: 4,
            y: 6,
            width: 100,
            height: 40,
            radius: 0,
        };
        assert_eq!(strips(shape), [(4, 6, 100, 40)]);
    }

    #[test]
    fn rows_lie_under_each_other_inside_the_part_and_mirror_each_other() {
        for (width, height, radius) in [(300, 200, 12), (40, 40, 20), (64, 20, 10), (7, 9, 3)] {
            let shape = Rounded {
                x: 10,
                y: 20,
                width,
                height,
                radius,
            };
            let rows = strips(shape);
            assert!(rows.len() <= 2 * radius as usize + 1, "{rows:?}");
            // From the top to the bottom without a gap or an overlap.
            let mut y = shape.y;
            for row in &rows {
                assert_eq!(row.1, y, "{rows:?}");
                assert!(row.3 > 0 && row.2 > 0);
                assert!(row.0 >= shape.x && row.0 + row.2 <= shape.x + width);
                // As far from one side as from the other.
                assert_eq!(row.0 - shape.x, shape.x + width - (row.0 + row.2));
                y += row.3;
            }
            assert_eq!(y, shape.y + height);
            // The top is the bottom turned over.
            let widths = |rows: &[(i32, i32, i32, i32)]| -> Vec<i32> {
                rows.iter()
                    .flat_map(|row| std::iter::repeat_n(row.2, row.3 as usize))
                    .collect()
            };
            let down = widths(&rows);
            let mut up = down.clone();
            up.reverse();
            assert_eq!(down, up);
            // Narrowest at the very top, full width in the middle.
            assert!(down[0] < width || radius == 0);
            assert_eq!(down[height as usize / 2], width);
            // Each row of a corner is no narrower than the one above it.
            assert!(down[..=height as usize / 2].is_sorted());
        }
    }

    #[test]
    fn round_corners_take_away_about_what_a_circle_leaves_of_its_square() {
        let radius = 40;
        let shape = Rounded {
            x: 0,
            y: 0,
            width: 200,
            height: 200,
            radius,
        };
        let covered: i64 = strips(shape)
            .iter()
            .map(|row| i64::from(row.2) * i64::from(row.3))
            .sum();
        // Four corners, each a square without its quarter of a circle.
        let cut = 200.0 * 200.0 - covered as f64;
        let expected = 4.0 * f64::from(radius * radius) * (1.0 - std::f64::consts::FRAC_PI_4);
        assert!(
            (cut - expected).abs() < expected * 0.06,
            "{cut} cut away, {expected} expected"
        );
    }
}
