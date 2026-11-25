//! Where a zone is, said so that it survives a change of the displays: a
//! display by its id, a corner or edge of it, and how far from there, in
//! logical units. A zone in the bottom right corner stays in the bottom
//! right corner when the display gets another resolution or scale.

use crate::ids::DisplayId;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// A display as the backend found it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Display {
    pub id: DisplayId,
    /// Where it is among the displays, in logical units.
    pub rect: Rect,
    /// How many pixels a logical unit is.
    pub scale: f64,
    /// The one new zones go to, and zones of a display that went.
    #[serde(default)]
    pub primary: bool,
}

impl Display {
    pub fn new(id: &str, rect: Rect) -> Self {
        Self {
            id: DisplayId::new(id),
            rect,
            scale: 1.0,
            primary: false,
        }
    }
}

/// A side of a display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// Room at a side of a display that something else keeps (a panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservedArea {
    pub display: DisplayId,
    pub edge: Edge,
    pub thickness: i32,
}

/// What is left of a display for zones: its rectangle without the room
/// kept at its sides. Never less than nothing.
pub fn work_area(display: &Display, reserved: &[ReservedArea]) -> Rect {
    let mut left = 0i32;
    let mut top = 0i32;
    let mut right = 0i32;
    let mut bottom = 0i32;
    for area in reserved.iter().filter(|area| area.display == display.id) {
        let thickness = area.thickness.max(0);
        match area.edge {
            Edge::Left => left = left.saturating_add(thickness),
            Edge::Top => top = top.saturating_add(thickness),
            Edge::Right => right = right.saturating_add(thickness),
            Edge::Bottom => bottom = bottom.saturating_add(thickness),
        }
    }
    let rect = display.rect;
    // (Room kept beyond the whole display leaves the display as it is:
    // a zone must be reachable somewhere.)
    if left.saturating_add(right) >= rect.width || top.saturating_add(bottom) >= rect.height {
        return rect;
    }
    Rect::new(
        rect.x + left,
        rect.y + top,
        rect.width - left - right,
        rect.height - top - bottom,
    )
}

/// The point of a display a zone holds on to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Centre,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

/// An anchor along one direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Along {
    Start,
    Middle,
    End,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Centre,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    fn horizontal(self) -> Along {
        match self {
            Anchor::TopLeft | Anchor::Left | Anchor::BottomLeft => Along::Start,
            Anchor::Top | Anchor::Centre | Anchor::Bottom => Along::Middle,
            Anchor::TopRight | Anchor::Right | Anchor::BottomRight => Along::End,
        }
    }

    fn vertical(self) -> Along {
        match self {
            Anchor::TopLeft | Anchor::Top | Anchor::TopRight => Along::Start,
            Anchor::Left | Anchor::Centre | Anchor::Right => Along::Middle,
            Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => Along::End,
        }
    }

    fn of(horizontal: Along, vertical: Along) -> Anchor {
        match (vertical, horizontal) {
            (Along::Start, Along::Start) => Anchor::TopLeft,
            (Along::Start, Along::Middle) => Anchor::Top,
            (Along::Start, Along::End) => Anchor::TopRight,
            (Along::Middle, Along::Start) => Anchor::Left,
            (Along::Middle, Along::Middle) => Anchor::Centre,
            (Along::Middle, Along::End) => Anchor::Right,
            (Along::End, Along::Start) => Anchor::BottomLeft,
            (Along::End, Along::Middle) => Anchor::Bottom,
            (Along::End, Along::End) => Anchor::BottomRight,
        }
    }
}

/// How far a span lies from the anchor of the span around it. From a side
/// the distance counts inward; from the middle it counts to the right or
/// downward, from middle to middle.
fn offset_along(along: Along, outer: (i32, i32), inner: (i32, i32)) -> i32 {
    let (outer_start, outer_length) = outer;
    let (inner_start, inner_length) = inner;
    match along {
        // (Never past what a number holds: the spans may be from a file
        // or a command written by anyone.)
        Along::Start => inner_start.saturating_sub(outer_start),
        Along::End => outer_start
            .saturating_add(outer_length)
            .saturating_sub(inner_start.saturating_add(inner_length)),
        Along::Middle => inner_start
            .saturating_add(inner_length / 2)
            .saturating_sub(outer_start.saturating_add(outer_length / 2)),
    }
}

/// Where a span of `length` starts that lies `offset` from the anchor.
fn start_along(along: Along, outer: (i32, i32), offset: i32, length: i32) -> i32 {
    let (outer_start, outer_length) = outer;
    match along {
        Along::Start => outer_start.saturating_add(offset),
        Along::End => outer_start
            .saturating_add(outer_length)
            .saturating_sub(offset)
            .saturating_sub(length),
        Along::Middle => outer_start
            .saturating_add(outer_length / 2)
            .saturating_add(offset)
            .saturating_sub(length / 2),
    }
}

/// The side a span is nearer to, or the middle when it sits in the middle.
fn nearest_along(outer: (i32, i32), inner: (i32, i32)) -> Along {
    let before = inner.0 - outer.0;
    let after = (outer.0 + outer.1) - (inner.0 + inner.1);
    if (before - after).abs() <= 1 {
        Along::Middle
    } else if before < after {
        Along::Start
    } else {
        Along::End
    }
}

/// Where a zone is on one arrangement of displays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneGeometry {
    pub display: DisplayId,
    pub anchor: Anchor,
    /// From the anchor, in logical units (see `Anchor`).
    pub offset_x: i32,
    pub offset_y: i32,
    pub width: i32,
    pub height: i32,
}

impl ZoneGeometry {
    /// Say where `rect` is on a display, from a given anchor.
    pub fn of(rect: Rect, display: &Display, anchor: Anchor) -> Self {
        let around = display.rect;
        Self {
            display: display.id.clone(),
            anchor,
            offset_x: offset_along(
                anchor.horizontal(),
                (around.x, around.width),
                (rect.x, rect.width),
            ),
            offset_y: offset_along(
                anchor.vertical(),
                (around.y, around.height),
                (rect.y, rect.height),
            ),
            width: rect.width,
            height: rect.height,
        }
    }

    /// Say where `rect` is on a display, from the corner, edge or middle
    /// it is nearest to.
    pub fn nearest(rect: Rect, display: &Display) -> Self {
        Self::of(rect, display, nearest_anchor(rect, display.rect))
    }

    /// The rectangle this is on `display`, as large as it was stored. It
    /// may reach beyond the display when that became smaller: `snap::
    /// keep_reachable` brings it back for showing, and what is stored
    /// stays, so that the zone is where it was when the display is as it
    /// was.
    pub fn on(&self, display: &Display) -> Rect {
        let around = display.rect;
        Rect::new(
            start_along(
                self.anchor.horizontal(),
                (around.x, around.width),
                self.offset_x,
                self.width,
            ),
            start_along(
                self.anchor.vertical(),
                (around.y, around.height),
                self.offset_y,
                self.height,
            ),
            self.width,
            self.height,
        )
    }
}

/// The anchor of `around` that `rect` is nearest to.
pub fn nearest_anchor(rect: Rect, around: Rect) -> Anchor {
    Anchor::of(
        nearest_along((around.x, around.width), (rect.x, rect.width)),
        nearest_along((around.y, around.height), (rect.y, rect.height)),
    )
}

/// The display a rectangle belongs to: the one its middle is on, else the
/// one whose middle is nearest. Nothing only where there is no display.
pub fn display_of(rect: Rect, displays: &[Display]) -> Option<&Display> {
    let (x, y) = rect.center();
    displays
        .iter()
        .find(|display| display.rect.contains_point(x, y))
        .or_else(|| {
            displays.iter().min_by_key(|display| {
                let (centre_x, centre_y) = display.rect.center();
                let across = i64::from(centre_x) - i64::from(x);
                let down = i64::from(centre_y) - i64::from(y);
                across * across + down * down
            })
        })
}

/// The display zones go to when they have no other: the primary one, else
/// the one with the least id (the same one whatever order they come in).
pub fn fallback_display(displays: &[Display]) -> Option<&Display> {
    displays
        .iter()
        .filter(|display| display.primary)
        .min_by(|a, b| a.id.cmp(&b.id))
        .or_else(|| displays.iter().min_by(|a, b| a.id.cmp(&b.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Display {
        Display::new("a", Rect::new(1920, -200, 2560, 1440))
    }

    #[test]
    fn a_rectangle_comes_back_from_every_anchor() {
        let display = display();
        for rect in [
            Rect::new(1920, -200, 300, 200),
            Rect::new(2000, 0, 301, 199),
            Rect::new(4000, 1000, 480, 240),
            Rect::new(1900, -300, 77, 3001),
            Rect::new(3199, 519, 1, 1),
        ] {
            for anchor in Anchor::ALL {
                let stored = ZoneGeometry::of(rect, &display, anchor);
                assert_eq!(stored.on(&display), rect, "{anchor:?} {rect:?}");
            }
            assert_eq!(ZoneGeometry::nearest(rect, &display).on(&display), rect);
        }
    }

    #[test]
    fn a_zone_holds_on_to_its_corner_when_the_display_changes() {
        let before = Display::new("a", Rect::new(0, 0, 1920, 1080));
        let after = Display::new("a", Rect::new(0, 0, 2560, 1440));
        // (rect on the display before, anchor expected, rect on the one after)
        for (rect, anchor, expected) in [
            (Rect::new(20, 30, 300, 200), Anchor::TopLeft, Rect::new(20, 30, 300, 200)),
            (
                Rect::new(1600, 30, 300, 200),
                Anchor::TopRight,
                Rect::new(2240, 30, 300, 200),
            ),
            (
                Rect::new(1600, 860, 300, 200),
                Anchor::BottomRight,
                Rect::new(2240, 1220, 300, 200),
            ),
            (
                Rect::new(20, 860, 300, 200),
                Anchor::BottomLeft,
                Rect::new(20, 1220, 300, 200),
            ),
            (
                Rect::new(810, 440, 300, 200),
                Anchor::Centre,
                Rect::new(1130, 620, 300, 200),
            ),
            (Rect::new(810, 10, 300, 200), Anchor::Top, Rect::new(1130, 10, 300, 200)),
        ] {
            let stored = ZoneGeometry::nearest(rect, &before);
            assert_eq!(stored.anchor, anchor, "{rect:?}");
            assert_eq!(stored.on(&after), expected, "{rect:?}");
        }
    }

    #[test]
    fn the_work_area_is_the_display_without_the_room_kept() {
        let display = Display::new("a", Rect::new(100, 0, 1920, 1080));
        let other = DisplayId::new("b");
        let reserved = [
            ReservedArea { display: display.id.clone(), edge: Edge::Top, thickness: 32 },
            ReservedArea { display: display.id.clone(), edge: Edge::Left, thickness: 48 },
            ReservedArea { display: other, edge: Edge::Bottom, thickness: 500 },
        ];
        assert_eq!(work_area(&display, &reserved), Rect::new(148, 32, 1872, 1048));
        assert_eq!(work_area(&display, &[]), display.rect);
        // More kept than there is: the display as it is.
        let all = [ReservedArea { display: display.id.clone(), edge: Edge::Top, thickness: 5000 }];
        assert_eq!(work_area(&display, &all), display.rect);
    }

    #[test]
    fn a_rectangle_belongs_to_the_display_its_middle_is_on() {
        let displays = [
            Display::new("left", Rect::new(0, 0, 1920, 1080)),
            Display::new("right", Rect::new(1920, 0, 1920, 1080)),
        ];
        let on = |rect| display_of(rect, &displays).map(|display| display.id.as_str().to_string());
        assert_eq!(on(Rect::new(1800, 10, 200, 100)).as_deref(), Some("left"));
        assert_eq!(on(Rect::new(1830, 10, 200, 100)).as_deref(), Some("right"));
        // Off every display: the nearest.
        assert_eq!(on(Rect::new(9000, 10, 200, 100)).as_deref(), Some("right"));
        assert_eq!(display_of(Rect::new(0, 0, 1, 1), &[]), None);
    }

    #[test]
    fn the_fallback_display_does_not_depend_on_the_order() {
        let mut displays = vec![
            Display::new("m", Rect::new(0, 0, 10, 10)),
            Display::new("c", Rect::new(10, 0, 10, 10)),
            Display::new("x", Rect::new(20, 0, 10, 10)),
        ];
        assert_eq!(fallback_display(&displays).unwrap().id.as_str(), "c");
        displays[2].primary = true;
        assert_eq!(fallback_display(&displays).unwrap().id.as_str(), "x");
        displays.reverse();
        assert_eq!(fallback_display(&displays).unwrap().id.as_str(), "x");
    }
}
