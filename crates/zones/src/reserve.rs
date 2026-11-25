//! Room that zones keep for themselves. A zone lies under the windows
//! unless its `reserves_space` is set; then tiled windows keep off it as
//! they keep off a panel. The core only says which room that is: whoever
//! runs the zones hands it to the window manager.
//!
//! A window manager keeps room at the sides of a display, so each region
//! comes with the side it is nearest to and how deep it reaches in from
//! there, beside its rectangle.

use crate::geometry::Edge;
use crate::ids::{DisplayId, ZoneId};
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// Room on a display that tiled windows are to keep off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReservedRegion {
    pub display: DisplayId,
    /// On the desktop, in logical units: where the zone is drawn.
    pub rect: Rect,
    /// The side of the display's free room the region is kept at.
    pub edge: Edge,
    /// How far the region reaches in from that side of the free room (the
    /// room that panels leave), in logical units.
    pub thickness: i32,
    /// The zones that are there: one, or the tabs that share the place.
    pub zones: Vec<ZoneId>,
}

/// The side of `area` a rectangle within it is kept at, and how far it
/// reaches in from there: the side from which that is least. Equal
/// depths go to the top, then the bottom, the left, the right, as panels
/// are mostly found.
pub fn side(rect: Rect, area: Rect) -> (Edge, i32) {
    let depths = [
        (Edge::Top, rect.y + rect.height - area.y),
        (Edge::Bottom, area.y + area.height - rect.y),
        (Edge::Left, rect.x + rect.width - area.x),
        (Edge::Right, area.x + area.width - rect.x),
    ];
    let mut best = depths[0];
    for candidate in &depths[1..] {
        if candidate.1 < best.1 {
            best = *candidate;
        }
    }
    (best.0, best.1.max(0))
}

/// The region a zone at `rect` keeps on a display whose free room is
/// `area`.
pub fn region(display: &DisplayId, rect: Rect, area: Rect, zones: Vec<ZoneId>) -> ReservedRegion {
    let (edge, thickness) = side(rect, area);
    ReservedRegion {
        display: display.clone(),
        rect,
        edge,
        thickness,
        zones,
    }
}

/// What a display's free room is once the regions kept on it are taken
/// from its sides: what is left for tiled windows.
pub fn left_over(area: Rect, display: &DisplayId, regions: &[ReservedRegion]) -> Rect {
    let (mut left, mut top, mut right, mut bottom) = (0, 0, 0, 0);
    for region in regions.iter().filter(|region| &region.display == display) {
        match region.edge {
            Edge::Left => left = left.max(region.thickness),
            Edge::Top => top = top.max(region.thickness),
            Edge::Right => right = right.max(region.thickness),
            Edge::Bottom => bottom = bottom.max(region.thickness),
        }
    }
    Rect::new(
        area.x + left,
        area.y + top,
        (area.width - left - right).max(0),
        (area.height - top - bottom).max(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_is_kept_at_the_side_it_is_nearest() {
        let area = Rect::new(0, 40, 1920, 1040);
        for (rect, edge, thickness) in [
            // A strip along the top of the free room.
            (Rect::new(0, 40, 1920, 120), Edge::Top, 120),
            // A column at the right.
            (Rect::new(1620, 40, 300, 1040), Edge::Right, 300),
            // A column at the left, a little in from the side.
            (Rect::new(10, 200, 240, 600), Edge::Left, 250),
            // A strip along the bottom.
            (Rect::new(400, 980, 800, 100), Edge::Bottom, 100),
            // A square in the top left corner: equal, so the top.
            (Rect::new(0, 40, 200, 200), Edge::Top, 200),
        ] {
            assert_eq!(side(rect, area), (edge, thickness), "{rect:?}");
        }
    }

    #[test]
    fn what_is_left_is_the_free_room_without_the_regions() {
        let area = Rect::new(0, 40, 1920, 1040);
        let display = DisplayId::new("one");
        let regions = [
            region(
                &display,
                Rect::new(0, 40, 1920, 120),
                area,
                vec![ZoneId::new("top")],
            ),
            region(
                &display,
                Rect::new(1620, 200, 300, 800),
                area,
                vec![ZoneId::new("side")],
            ),
            region(
                &DisplayId::new("other"),
                Rect::new(0, 0, 500, 1080),
                area,
                vec![ZoneId::new("else")],
            ),
        ];
        assert_eq!(
            left_over(area, &display, &regions),
            Rect::new(0, 160, 1620, 920)
        );
        assert_eq!(left_over(area, &DisplayId::new("third"), &regions), area);
    }
}
