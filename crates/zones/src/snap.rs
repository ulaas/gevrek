//! What a zone is drawn to while it is moved or sized: the room that is
//! free on its display, the display's own sides, the other zones (always
//! with the same gap between two), and a grid where nothing else pulls.
//! And the one thing that holds whatever the person does: a zone can
//! always be reached.

use crate::config::SnapSettings;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// What is around a zone that is moved.
#[derive(Debug, Clone, Copy)]
pub struct Surroundings<'a> {
    /// The whole display.
    pub display: Rect,
    /// The display without the room panels keep.
    pub work_area: Rect,
    /// The other zones shown on it.
    pub others: &'a [Rect],
}

/// The sides of a zone that a resize moves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Edges {
    pub const BOTTOM_RIGHT: Edges = Edges { left: false, top: false, right: true, bottom: true };
}

/// One direction of a rectangle: where it starts and how long it is.
type Span = (i32, i32);

fn across(rect: Rect) -> Span {
    (rect.x, rect.width)
}

fn down(rect: Rect) -> Span {
    (rect.y, rect.height)
}

fn end(span: Span) -> i32 {
    span.0 + span.1
}

/// Whether two spans come within `reach` of each other.
fn near(one: Span, other: Span, reach: i32) -> bool {
    one.0 <= end(other) + reach && other.0 <= end(one) + reach
}

/// The lines a side of a zone may snap to, each with its rank: among
/// lines equally near, the lower rank wins (the free room before the
/// display's side, a neighbour's gap before lining up with it).
struct Lines {
    /// For the side a zone starts with (left, top).
    starts: Vec<(i32, u8)>,
    /// For the side it ends with (right, bottom).
    ends: Vec<(i32, u8)>,
}

/// The lines along one direction. `other_way` is the zone's span in the
/// other direction: a zone far above or below pulls nothing sideways.
fn lines(
    around: &Surroundings,
    settings: &SnapSettings,
    pick: fn(Rect) -> Span,
    cross: fn(Rect) -> Span,
    other_way: Span,
) -> Lines {
    let work = pick(around.work_area);
    let display = pick(around.display);
    let mut starts = vec![(work.0, 0), (display.0, 1)];
    let mut ends = vec![(end(work), 0), (end(display), 1)];
    let reach = settings.distance.max(0) + settings.gap.max(0);
    for other in around.others {
        if !near(cross(*other), other_way, reach) {
            continue;
        }
        let span = pick(*other);
        // Beside it, the gap between; or in a line with it.
        starts.push((end(span) + settings.gap, 2));
        starts.push((span.0, 3));
        ends.push((span.0 - settings.gap, 2));
        ends.push((end(span), 3));
    }
    Lines { starts, ends }
}

/// The nearest line within the distance: how far to go to be on it.
fn pull(at: i32, lines: &[(i32, u8)], distance: i32) -> Option<(i32, u8)> {
    lines
        .iter()
        .map(|(line, rank)| (line - at, *rank))
        .filter(|(by, _)| by.abs() <= distance)
        .min_by_key(|(by, rank)| (by.abs(), *rank))
}

/// `at` on the nearest line of the grid, which starts where the free room
/// does.
fn on_grid(at: i32, origin: i32, grid: i32) -> i32 {
    if grid <= 1 {
        return at;
    }
    let from = at - origin;
    origin + (from + grid.div_euclid(2)).div_euclid(grid) * grid
}

/// Where a zone that is moved to `rect` comes to rest. Its size stays.
pub fn snap_move(rect: Rect, around: &Surroundings, settings: &SnapSettings) -> Rect {
    if !settings.enabled {
        return keep_reachable(rect, around.work_area);
    }
    let one_way = |pick: fn(Rect) -> Span, cross: fn(Rect) -> Span| {
        let span = pick(rect);
        let lines = lines(around, settings, pick, cross, cross(rect));
        let by_start = pull(span.0, &lines.starts, settings.distance);
        let by_end = pull(end(span), &lines.ends, settings.distance);
        let nearest = [by_start, by_end]
            .into_iter()
            .flatten()
            .min_by_key(|(by, rank)| (by.abs(), *rank));
        match nearest {
            Some((by, _)) => span.0 + by,
            None => on_grid(span.0, pick(around.work_area).0, settings.grid),
        }
    };
    let snapped = Rect::new(one_way(across, down), one_way(down, across), rect.width, rect.height);
    keep_reachable(snapped, around.work_area)
}

/// What a zone that is sized to `rect` becomes: the sides that move snap,
/// the others stay, and the zone is never smaller than `least` nor reaches
/// out of the free room with a side that moved.
pub fn snap_resize(
    rect: Rect,
    moving: Edges,
    around: &Surroundings,
    settings: &SnapSettings,
    least: (i32, i32),
) -> Rect {
    let one_way = |pick: fn(Rect) -> Span,
                   cross: fn(Rect) -> Span,
                   start_moves: bool,
                   end_moves: bool,
                   least: i32| {
        let span = pick(rect);
        let room = pick(around.work_area);
        let mut start = span.0;
        let mut finish = end(span);
        let lines = lines(around, settings, pick, cross, cross(rect));
        let settle = |at: i32, lines: &[(i32, u8)]| {
            if !settings.enabled {
                return at;
            }
            match pull(at, lines, settings.distance) {
                Some((by, _)) => at + by,
                None => on_grid(at, room.0, settings.grid),
            }
        };
        if start_moves {
            start = settle(start, &lines.starts).max(room.0);
            // The side that moved gives way, the other one stays.
            start = start.min(finish - least);
        }
        if end_moves {
            finish = settle(finish, &lines.ends).min(end(room));
            finish = finish.max(start + least);
        }
        (start, (finish - start).max(least))
    };
    let (x, width) = one_way(across, down, moving.left, moving.right, least.0.max(1));
    let (y, height) = one_way(down, across, moving.top, moving.bottom, least.1.max(1));
    Rect::new(x, y, width, height)
}

/// `rect` brought to where it can be reached: no larger than `bounds`, and
/// wholly within them. A rectangle that is within them already is left as
/// it is.
pub fn keep_reachable(rect: Rect, bounds: Rect) -> Rect {
    let width = rect.width.min(bounds.width).max(1);
    let height = rect.height.min(bounds.height).max(1);
    let x = rect.x.min(bounds.x + bounds.width - width).max(bounds.x);
    let y = rect.y.min(bounds.y + bounds.height - height).max(bounds.y);
    Rect::new(x, y, width, height)
}

/// Whether a rectangle lies wholly within another.
pub fn within(rect: Rect, bounds: Rect) -> bool {
    rect.x >= bounds.x
        && rect.y >= bounds.y
        && rect.x + rect.width <= bounds.x + bounds.width
        && rect.y + rect.height <= bounds.y + bounds.height
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAY: Rect = Rect { x: 0, y: 0, width: 1920, height: 1080 };
    /// A panel of 40 at the top.
    const WORK: Rect = Rect { x: 0, y: 40, width: 1920, height: 1040 };

    fn settings() -> SnapSettings {
        SnapSettings { enabled: true, distance: 12, gap: 8, grid: 0, ..SnapSettings::default() }
    }

    #[test]
    fn a_moved_zone_snaps_to_what_is_near() {
        let neighbour = Rect::new(600, 300, 300, 200);
        let others = [neighbour];
        let around = Surroundings { display: DISPLAY, work_area: WORK, others: &others };
        // (what, moved to, comes to rest at)
        for (what, to, rest) in [
            ("the display's left side", Rect::new(7, 500, 200, 100), Rect::new(0, 500, 200, 100)),
            ("its right side", Rect::new(1711, 500, 200, 100), Rect::new(1720, 500, 200, 100)),
            ("the free room's top", Rect::new(300, 49, 200, 100), Rect::new(300, 40, 200, 100)),
            ("the bottom", Rect::new(300, 975, 200, 100), Rect::new(300, 980, 200, 100)),
            (
                "the room wins over the display's own top",
                Rect::new(300, 20, 200, 100),
                Rect::new(300, 40, 200, 100),
            ),
            (
                "right of the neighbour, with the gap",
                Rect::new(903, 350, 200, 100),
                Rect::new(908, 350, 200, 100),
            ),
            (
                "left of the neighbour, with the gap",
                Rect::new(395, 350, 200, 100),
                Rect::new(392, 350, 200, 100),
            ),
            (
                "below the neighbour, with the gap, and in a line with its left",
                Rect::new(604, 503, 200, 100),
                Rect::new(600, 508, 200, 100),
            ),
            (
                "in a line with the neighbour's top",
                Rect::new(908, 305, 200, 100),
                Rect::new(908, 300, 200, 100),
            ),
            ("nothing near", Rect::new(1200, 700, 200, 100), Rect::new(1200, 700, 200, 100)),
            (
                "a neighbour far above pulls nothing sideways",
                Rect::new(903, 800, 200, 100),
                Rect::new(903, 800, 200, 100),
            ),
            ("beyond the display", Rect::new(-500, 5000, 200, 100), Rect::new(0, 980, 200, 100)),
        ] {
            assert_eq!(snap_move(to, &around, &settings()), rest, "{what}");
        }
    }

    #[test]
    fn two_zones_snapped_to_each_other_keep_the_same_gap_on_every_side() {
        let middle = Rect::new(800, 400, 300, 200);
        let others = [middle];
        let around = Surroundings { display: DISPLAY, work_area: WORK, others: &others };
        let settings = settings();
        let left = snap_move(Rect::new(495, 400, 300, 200), &around, &settings);
        let right = snap_move(Rect::new(1105, 400, 300, 200), &around, &settings);
        let above = snap_move(Rect::new(800, 190, 300, 200), &around, &settings);
        let below = snap_move(Rect::new(800, 611, 300, 200), &around, &settings);
        assert_eq!(middle.x - (left.x + left.width), settings.gap);
        assert_eq!(right.x - (middle.x + middle.width), settings.gap);
        assert_eq!(middle.y - (above.y + above.height), settings.gap);
        assert_eq!(below.y - (middle.y + middle.height), settings.gap);
    }

    #[test]
    fn a_grid_holds_what_nothing_else_pulls() {
        let around = Surroundings { display: DISPLAY, work_area: WORK, others: &[] };
        let settings = SnapSettings { grid: 50, ..settings() };
        // The grid starts at the free room's corner (0, 40).
        for (to, rest) in [
            (Rect::new(120, 160, 200, 100), Rect::new(100, 140, 200, 100)),
            (Rect::new(126, 166, 200, 100), Rect::new(150, 190, 200, 100)),
            // An edge within reach wins over the grid.
            (Rect::new(9, 500, 200, 100), Rect::new(0, 490, 200, 100)),
        ] {
            assert_eq!(snap_move(to, &around, &settings), rest, "{to:?}");
        }
        // Without snapping a zone goes where it is put, if it can be reached there.
        let off = SnapSettings { enabled: false, ..settings };
        assert_eq!(
            snap_move(Rect::new(7, 49, 200, 100), &around, &off),
            Rect::new(7, 49, 200, 100)
        );
    }

    #[test]
    fn a_sized_zone_snaps_with_the_sides_that_move() {
        let neighbour = Rect::new(600, 300, 300, 200);
        let others = [neighbour];
        let around = Surroundings { display: DISPLAY, work_area: WORK, others: &others };
        let least = (120, 28);
        let right = Edges { right: true, ..Edges::default() };
        let left = Edges { left: true, ..Edges::default() };
        let top_left = Edges { left: true, top: true, ..Edges::default() };
        for (what, to, moving, becomes) in [
            (
                "the right side meets the neighbour's gap",
                Rect::new(200, 350, 395, 100),
                right,
                Rect::new(200, 350, 392, 100),
            ),
            (
                "the left side does not move when the right one is dragged",
                Rect::new(5, 350, 300, 100),
                right,
                Rect::new(5, 350, 300, 100),
            ),
            (
                "the left side meets the display",
                Rect::new(5, 350, 300, 100),
                left,
                Rect::new(0, 350, 305, 100),
            ),
            (
                "top and left at once",
                Rect::new(6, 46, 300, 100),
                top_left,
                Rect::new(0, 40, 306, 106),
            ),
            (
                "not smaller than the least",
                Rect::new(1000, 600, 10, 5),
                Edges::BOTTOM_RIGHT,
                Rect::new(1000, 600, 120, 28),
            ),
            (
                "dragged past its own other side",
                Rect::new(1400, 600, -50, 100),
                right,
                Rect::new(1400, 600, 120, 100),
            ),
            (
                "not out of the free room",
                Rect::new(1700, 900, 900, 900),
                Edges::BOTTOM_RIGHT,
                Rect::new(1700, 900, 220, 180),
            ),
        ] {
            assert_eq!(snap_resize(to, moving, &around, &settings(), least), becomes, "{what}");
        }
    }

    #[test]
    fn a_zone_is_brought_back_to_where_it_can_be_reached() {
        for (rect, expected) in [
            (Rect::new(100, 100, 200, 100), Rect::new(100, 100, 200, 100)),
            (Rect::new(-50, 10, 200, 100), Rect::new(0, 40, 200, 100)),
            (Rect::new(1900, 1070, 200, 100), Rect::new(1720, 980, 200, 100)),
            (Rect::new(10, 50, 5000, 5000), Rect::new(0, 40, 1920, 1040)),
        ] {
            let reachable = keep_reachable(rect, WORK);
            assert_eq!(reachable, expected);
            assert!(within(reachable, WORK));
        }
    }
}
