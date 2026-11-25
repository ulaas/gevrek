//! The keyboard's way about the zones: which zone lies in a direction
//! from another, and which item lies in a direction from another inside
//! a zone. Rectangles in, a choice out; what has the focus is kept by the
//! core (`core::ZonesCore::focus`), so that a key, a script and a screen
//! reader all mean the same zone.

use crate::ids::{DisplayId, ItemId, ZoneId};
use crate::layout::PlacedItem;
use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};

/// A way to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ];
}

/// A step among the items of a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
    /// To the first item of the zone.
    First,
    /// To the last.
    Last,
}

/// How a choice of items changes what is chosen.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SelectMode {
    /// These and no others.
    #[default]
    Replace,
    /// These as well.
    Add,
    /// These no longer.
    Remove,
    /// Each the other of what it was.
    Toggle,
}

/// In place of a zone's id: the bare desktop, where the items lie that
/// are in no zone. What has the keyboard there says so with this name,
/// and a command may name it to go there or to put items there.
pub const DESK: &str = "@desk";

/// What has the keyboard: a zone, or the loose items of a display, and
/// perhaps one item of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Focus {
    /// The zone; `@desk` when the keyboard is on the bare desktop.
    pub zone: ZoneId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemId>,
    /// On the bare desktop: the display whose loose items have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desk: Option<DisplayId>,
}

impl Focus {
    pub fn on_zone(zone: &ZoneId, item: Option<ItemId>) -> Self {
        Self {
            zone: zone.clone(),
            item,
            desk: None,
        }
    }

    pub fn on_desk(display: &DisplayId, item: Option<ItemId>) -> Self {
        Self {
            zone: ZoneId::new(DESK),
            item,
            desk: Some(display.clone()),
        }
    }
}

fn centre(rect: Rect) -> (i64, i64) {
    (
        i64::from(rect.x) + i64::from(rect.width) / 2,
        i64::from(rect.y) + i64::from(rect.height) / 2,
    )
}

/// How far `to` lies from `from` in a direction: along it, and aside of
/// it. Nothing when it does not lie that way at all.
fn distance(from: Rect, to: Rect, direction: Direction) -> Option<(i64, i64)> {
    let (from_x, from_y) = centre(from);
    let (to_x, to_y) = centre(to);
    let (along, aside) = match direction {
        Direction::Left => (from_x - to_x, to_y - from_y),
        Direction::Right => (to_x - from_x, to_y - from_y),
        Direction::Up => (from_y - to_y, to_x - from_x),
        Direction::Down => (to_y - from_y, to_x - from_x),
    };
    // What shares the other direction's span with us is straight ahead,
    // however far its centre is aside.
    let overlaps = match direction {
        Direction::Left | Direction::Right => {
            from.y < to.y + to.height && to.y < from.y + from.height
        }
        Direction::Up | Direction::Down => from.x < to.x + to.width && to.x < from.x + from.width,
    };
    (along > 0).then_some((along, if overlaps { 0 } else { aside.abs() }))
}

/// The rectangle that lies nearest in a direction from `from`, by its
/// place in `others`. What is straight ahead comes before what is aside:
/// going aside counts double.
pub fn neighbour(from: Rect, others: &[Rect], direction: Direction) -> Option<usize> {
    others
        .iter()
        .enumerate()
        .filter_map(|(index, other)| {
            let (along, aside) = distance(from, *other, direction)?;
            Some((along + 2 * aside, along, index))
        })
        .min()
        .map(|(_, _, index)| index)
}

/// The rectangle a walk begins at when nothing has the focus: the one
/// nearest the top left corner, by rows.
pub fn first(rects: &[Rect]) -> Option<usize> {
    rects
        .iter()
        .enumerate()
        .min_by_key(|(index, rect)| (rect.y, rect.x, *index))
        .map(|(index, _)| index)
}

/// The item a step from `from` leads to among the items of a zone as
/// they are laid out, by its place in `items`. From nothing, a step leads
/// to the first item (to the last, for `Last`). At the zone's side the
/// focus stays where it is.
pub fn step(items: &[PlacedItem], from: Option<&ItemId>, step: Step) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let at = from.and_then(|from| items.iter().position(|placed| &placed.item == from));
    let Some(at) = at else {
        return Some(if step == Step::Last {
            items.len() - 1
        } else {
            0
        });
    };
    let direction = match step {
        Step::First => return Some(0),
        Step::Last => return Some(items.len() - 1),
        Step::Left => Direction::Left,
        Step::Right => Direction::Right,
        Step::Up => Direction::Up,
        Step::Down => Direction::Down,
    };
    let rects: Vec<Rect> = items.iter().map(|placed| placed.rect).collect();
    let ahead = neighbour(rects[at], &rects, direction);
    // In a grid the end of a row leads to the beginning of the next, as
    // reading does.
    let wrapped = match direction {
        Direction::Right => (at + 1 < items.len()).then_some(at + 1),
        Direction::Left => at.checked_sub(1),
        _ => None,
    };
    let straight = ahead.filter(|index| match direction {
        Direction::Left | Direction::Right => rects[*index].y == rects[at].y,
        _ => true,
    });
    straight.or(wrapped).or(Some(at))
}

/// A step among items that lie where they were put, as the loose items
/// of the desktop do: to the nearest one in the direction, and nowhere
/// at the side. No end of a row leads on to another here: there are no
/// rows. `First` and `Last` are the first and last in the order given.
pub fn step_free(items: &[PlacedItem], from: Option<&ItemId>, step: Step) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let at = from.and_then(|from| items.iter().position(|placed| &placed.item == from));
    let Some(at) = at else {
        return Some(if step == Step::Last {
            items.len() - 1
        } else {
            0
        });
    };
    let direction = match step {
        Step::First => return Some(0),
        Step::Last => return Some(items.len() - 1),
        Step::Left => Direction::Left,
        Step::Right => Direction::Right,
        Step::Up => Direction::Up,
        Step::Down => Direction::Down,
    };
    let rects: Vec<Rect> = items.iter().map(|placed| placed.rect).collect();
    neighbour(rects[at], &rects, direction).or(Some(at))
}

/// The smallest rectangle that holds all of these; nothing for none.
pub fn around(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().fold(None, |all: Option<Rect>, rect| {
        Some(match all {
            None => rect,
            Some(all) => {
                let x = all.x.min(rect.x);
                let y = all.y.min(rect.y);
                let right = (all.x + all.width).max(rect.x + rect.width);
                let bottom = (all.y + all.height).max(rect.y + rect.height);
                Rect::new(x, y, right - x, bottom - y)
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::Cell;

    #[test]
    fn loose_items_are_walked_to_the_nearest_in_the_direction() {
        // Two down the first column, one far right at the top, one in the middle.
        let at = |name: &str, x: i32, y: i32| {
            PlacedItem::at(
                ItemId::new(name),
                Rect::new(x, y, 76, 82),
                Cell { column: 0, row: 0 },
            )
        };
        let items = [
            at("a", 0, 0),
            at("b", 0, 86),
            at("far", 900, 10),
            at("mid", 400, 300),
        ];
        let id = |name: &str| ItemId::new(name);
        for (from, step_taken, to) in [
            (Some("a"), Step::Down, "b"),
            (Some("b"), Step::Down, "mid"),
            (Some("mid"), Step::Down, "mid"),
            (Some("a"), Step::Right, "far"),
            (Some("b"), Step::Right, "mid"),
            (Some("far"), Step::Right, "far"),
            (Some("far"), Step::Left, "a"),
            (Some("mid"), Step::Up, "b"),
            (Some("far"), Step::Down, "mid"),
            (Some("a"), Step::Left, "a"),
            (Some("mid"), Step::First, "a"),
            (Some("a"), Step::Last, "mid"),
            (None, Step::Up, "a"),
            (Some("gone"), Step::Last, "mid"),
        ] {
            let from_id = from.map(id);
            let found = step_free(&items, from_id.as_ref(), step_taken)
                .map(|index| items[index].item.as_str());
            assert_eq!(found, Some(to), "{step_taken:?} from {from:?}");
        }
        assert_eq!(step_free(&[], None, Step::Down), None);
        assert_eq!(
            around(items.iter().map(|placed| placed.rect)),
            Some(Rect::new(0, 0, 976, 382))
        );
        assert_eq!(around([]), None);
    }

    #[test]
    fn the_neighbour_is_the_nearest_that_lies_that_way() {
        // Three in a row, one below the middle, one far down to the right.
        let rects = [
            Rect::new(0, 0, 200, 200),
            Rect::new(300, 0, 200, 200),
            Rect::new(600, 0, 200, 200),
            Rect::new(300, 300, 200, 200),
            Rect::new(900, 900, 200, 200),
        ];
        for (from, direction, to) in [
            (0, Direction::Right, Some(1)),
            (1, Direction::Right, Some(2)),
            (2, Direction::Right, Some(4)),
            (2, Direction::Left, Some(1)),
            (0, Direction::Left, None),
            (1, Direction::Down, Some(3)),
            (0, Direction::Down, Some(3)),
            (3, Direction::Up, Some(1)),
            (0, Direction::Up, None),
            (4, Direction::Up, Some(2)),
            (4, Direction::Left, Some(3)),
        ] {
            assert_eq!(
                neighbour(rects[from], &rects, direction),
                to,
                "{direction:?} of {from}"
            );
        }
        assert_eq!(first(&rects), Some(0));
        assert_eq!(first(&[]), None);
    }

    fn grid(count: usize, columns: usize) -> Vec<PlacedItem> {
        (0..count)
            .map(|index| {
                let (column, row) = (index % columns, index / columns);
                PlacedItem::at(
                    ItemId::new(format!("item-{index}")),
                    Rect::new(column as i32 * 80, row as i32 * 90, 76, 82),
                    Cell {
                        column: column as u32,
                        row: row as u32,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_step_goes_through_a_grid_as_reading_does() {
        // 0 1 2
        // 3 4 5
        // 6 7
        let items = grid(8, 3);
        let id = |index: usize| ItemId::new(format!("item-{index}"));
        for (from, step_taken, to) in [
            (Some(0), Step::Right, 1),
            (Some(2), Step::Right, 3),
            (Some(3), Step::Left, 2),
            (Some(0), Step::Left, 0),
            (Some(7), Step::Right, 7),
            (Some(1), Step::Down, 4),
            (Some(4), Step::Down, 7),
            (Some(5), Step::Down, 7),
            (Some(7), Step::Down, 7),
            (Some(7), Step::Up, 4),
            (Some(1), Step::Up, 1),
            (Some(4), Step::First, 0),
            (Some(4), Step::Last, 7),
            (None, Step::Down, 0),
            (None, Step::Last, 7),
        ] {
            let from_id = from.map(id);
            assert_eq!(
                step(&items, from_id.as_ref(), step_taken),
                Some(to),
                "{step_taken:?} from {from:?}"
            );
        }
        assert_eq!(step(&[], None, Step::Right), None);
        // An item that is gone is as good as none.
        assert_eq!(
            step(&items, Some(&ItemId::new("gone")), Step::Right),
            Some(0)
        );
    }

    #[test]
    fn a_list_is_walked_up_and_down() {
        let items = grid(4, 1);
        let id = |index: usize| ItemId::new(format!("item-{index}"));
        assert_eq!(step(&items, Some(&id(1)), Step::Down), Some(2));
        assert_eq!(step(&items, Some(&id(1)), Step::Up), Some(0));
        assert_eq!(step(&items, Some(&id(3)), Step::Down), Some(3));
        // Right and left go on to the next and the one before.
        assert_eq!(step(&items, Some(&id(1)), Step::Right), Some(2));
        assert_eq!(step(&items, Some(&id(1)), Step::Left), Some(0));
    }
}
