//! Room for windows that will not be as small as their tile.
//!
//! A program may say that its window has a smallest size. A layout knows
//! nothing of that: it divides the display by its own rule, and a window
//! that is given less than it takes is cut to its tile by the backend.
//! This pass comes after the layout and mends what can be mended: where a
//! tile is narrower (or lower) than its window's smallest size, the line
//! between it and its neighbours is moved, as far as the neighbours can
//! give without falling under their own smallest size or under a floor
//! that keeps every window usable. What cannot be had stays cut.
//!
//! It works on the tiles as they are, whatever layout made them: a line
//! is the edge that tiles on one side end at and tiles on the other side
//! begin at, and it is moved for all of them together, so the tiles go on
//! filling the display without overlap.

use gevrek_model::geometry::Rect;

/// No tile is made narrower or lower than this for another's sake, in
/// pixels (unless it was smaller already).
pub const FLOOR: i32 = 160;
/// Two tiles are neighbours across a line when the room between them is
/// no more than this (the gap between windows).
const NEAR: i32 = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Across,
    Down,
}

/// A tile along one axis: where it begins, how long it is, and where it
/// lies on the other axis.
fn along(rect: &Rect, axis: Axis) -> (i32, i32, i32, i32) {
    match axis {
        Axis::Across => (rect.x, rect.width, rect.y, rect.height),
        Axis::Down => (rect.y, rect.height, rect.x, rect.width),
    }
}

fn set_along(rect: &mut Rect, axis: Axis, start: i32, length: i32) {
    match axis {
        Axis::Across => {
            rect.x = start;
            rect.width = length;
        }
        Axis::Down => {
            rect.y = start;
            rect.height = length;
        }
    }
}

fn least(smallest: (i32, i32), axis: Axis) -> i32 {
    match axis {
        Axis::Across => smallest.0,
        Axis::Down => smallest.1,
    }
}

/// The tiles on both sides of the line at the end (`after`) or at the
/// beginning of tile `of`: those that end at it and those that begin at
/// it, as far as they hang together. `None` when nothing is on the other
/// side (the display's edge).
fn line(rects: &[Rect], of: usize, axis: Axis, after: bool) -> Option<(Vec<usize>, Vec<usize>)> {
    let overlap = |a: &Rect, b: &Rect| {
        let (_, _, a_from, a_long) = along(a, axis);
        let (_, _, b_from, b_long) = along(b, axis);
        a_from < b_from + b_long && b_from < a_from + a_long
    };
    let end = |rect: &Rect| {
        let (start, length, ..) = along(rect, axis);
        start + length
    };
    let begin = |rect: &Rect| along(rect, axis).0;
    // `before`: the tiles that end at the line; `behind`: those that
    // begin at it.
    let (mut before, mut behind): (Vec<usize>, Vec<usize>) = if after {
        (vec![of], Vec::new())
    } else {
        (Vec::new(), vec![of])
    };
    let at = if after {
        end(&rects[of])
    } else {
        begin(&rects[of])
    };
    loop {
        let mut grown = false;
        for (index, rect) in rects.iter().enumerate() {
            if !before.contains(&index)
                && !behind.contains(&index)
                && end(rect) <= at
                && at - end(rect) <= NEAR
                && behind.iter().any(|other| overlap(rect, &rects[*other]))
            {
                before.push(index);
                grown = true;
            }
            if !behind.contains(&index)
                && !before.contains(&index)
                && begin(rect) >= at
                && begin(rect) - at <= NEAR
                && before.iter().any(|other| overlap(rect, &rects[*other]))
            {
                behind.push(index);
                grown = true;
            }
        }
        if !grown {
            break;
        }
    }
    (!before.is_empty() && !behind.is_empty()).then_some((before, behind))
}

/// How far these tiles can be made shorter along the axis: each down to
/// its own smallest size, and to the floor.
fn slack(rects: &[Rect], smallest: &[(i32, i32)], tiles: &[usize], axis: Axis) -> i32 {
    tiles
        .iter()
        .map(|index| {
            let length = along(&rects[*index], axis).1;
            (length - least(smallest[*index], axis).max(FLOOR.min(length))).max(0)
        })
        .min()
        .unwrap_or(0)
}

/// Give the tiles that are smaller than their window's smallest size the
/// room their neighbours can spare. `smallest[i]` belongs to `rects[i]`:
/// a width and a height, 0 where the window names none. Whether a tile
/// was changed.
pub fn make_room(rects: &mut [Rect], smallest: &[(i32, i32)]) -> bool {
    if rects.len() < 2 || rects.len() != smallest.len() {
        return false;
    }
    let mut changed = false;
    for axis in [Axis::Across, Axis::Down] {
        // (Each move mends one tile at one line; a few rounds reach
        // those behind one another.)
        for _ in 0..rects.len() * 2 {
            let mut moved = false;
            for index in 0..rects.len() {
                for after in [true, false] {
                    let want = least(smallest[index], axis) - along(&rects[index], axis).1;
                    if want <= 0 {
                        break;
                    }
                    let Some((before, behind)) = line(rects, index, axis, after) else {
                        continue;
                    };
                    // The line moves away from the tile that wants room:
                    // the tiles on the other side give.
                    let giving = if after { &behind } else { &before };
                    let by = want.min(slack(rects, smallest, giving, axis));
                    if by <= 0 {
                        continue;
                    }
                    let shift = if after { by } else { -by };
                    for tile in &before {
                        let (start, length, ..) = along(&rects[*tile], axis);
                        set_along(&mut rects[*tile], axis, start, length + shift);
                    }
                    for tile in &behind {
                        let (start, length, ..) = along(&rects[*tile], axis);
                        set_along(&mut rects[*tile], axis, start + shift, length - shift);
                    }
                    moved = true;
                    changed = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(count: usize) -> Vec<(i32, i32)> {
        vec![(0, 0); count]
    }

    #[test]
    fn a_tile_too_narrow_takes_from_the_one_beside_it() {
        // Two beside one another with eight between them, 500 each; the
        // right one's window is 700 wide at least.
        let mut tiles = [Rect::new(0, 0, 500, 600), Rect::new(508, 0, 500, 600)];
        assert!(make_room(&mut tiles, &[(0, 0), (700, 0)]));
        assert_eq!(tiles[1], Rect::new(308, 0, 700, 600));
        assert_eq!(tiles[0], Rect::new(0, 0, 300, 600));
        // Nothing to mend: nothing changed.
        let mut fine = [Rect::new(0, 0, 500, 600), Rect::new(508, 0, 500, 600)];
        assert!(!make_room(&mut fine, &[(400, 300), (0, 0)]));
        assert_eq!(fine[0].width, 500);
    }

    #[test]
    fn a_neighbour_gives_no_more_than_it_can() {
        // The left one would have 900 of the 1008: the right one gives
        // down to the floor and no further.
        let mut tiles = [Rect::new(0, 0, 500, 600), Rect::new(508, 0, 500, 600)];
        assert!(make_room(&mut tiles, &[(900, 0), (0, 0)]));
        assert_eq!(tiles[1].width, FLOOR);
        assert_eq!(tiles[0].width, 500 + (500 - FLOOR));
        assert_eq!(tiles[1].x, tiles[0].x + tiles[0].width + 8);
        // Nor under its own smallest size.
        let mut both = [Rect::new(0, 0, 500, 600), Rect::new(508, 0, 500, 600)];
        assert!(make_room(&mut both, &[(900, 0), (450, 0)]));
        assert_eq!((both[0].width, both[1].width), (550, 450));
    }

    #[test]
    fn a_line_moves_for_every_tile_on_it() {
        // A master at the left, two stacked at the right: the master's
        // window wants 700, and both of the stack give.
        let mut tiles = [
            Rect::new(0, 0, 500, 608),
            Rect::new(508, 0, 500, 300),
            Rect::new(508, 308, 500, 300),
        ];
        assert!(make_room(&mut tiles, &[(700, 0), (0, 0), (0, 0)]));
        assert_eq!(tiles[0].width, 700);
        assert_eq!((tiles[1].x, tiles[1].width), (708, 300));
        assert_eq!((tiles[2].x, tiles[2].width), (708, 300));
        // One of the stack wants to be higher: the other of the stack
        // gives, the master stays as high as it is.
        let mut tiles = [
            Rect::new(0, 0, 500, 608),
            Rect::new(508, 0, 500, 300),
            Rect::new(508, 308, 500, 300),
        ];
        assert!(make_room(&mut tiles, &[(0, 0), (0, 0), (0, 420)]));
        assert_eq!((tiles[2].y, tiles[2].height), (188, 420));
        assert_eq!(tiles[1].height, 180);
        assert_eq!(tiles[0], Rect::new(0, 0, 500, 608));
    }

    #[test]
    fn a_tile_alone_or_at_the_displays_edge_stays_as_it_is() {
        let mut one = [Rect::new(0, 0, 500, 600)];
        assert!(!make_room(&mut one, &[(900, 900)]));
        // Two under one another: wider there is nothing to be had.
        let mut tiles = [Rect::new(0, 0, 500, 300), Rect::new(0, 308, 500, 300)];
        assert!(!make_room(&mut tiles, &[(900, 0), (0, 0)]));
        assert!(!make_room(&mut tiles, &none(2)));
    }

    #[test]
    fn the_tiles_go_on_filling_their_room_without_lying_on_one_another() {
        // Four in a grid; one wants more both ways.
        let mut tiles = [
            Rect::new(0, 0, 500, 300),
            Rect::new(508, 0, 500, 300),
            Rect::new(0, 308, 500, 300),
            Rect::new(508, 308, 500, 300),
        ];
        assert!(make_room(&mut tiles, &[(640, 400), (0, 0), (0, 0), (0, 0)]));
        assert_eq!(tiles[0], Rect::new(0, 0, 640, 400));
        for (index, a) in tiles.iter().enumerate() {
            for b in &tiles[index + 1..] {
                let apart = a.x + a.width <= b.x
                    || b.x + b.width <= a.x
                    || a.y + a.height <= b.y
                    || b.y + b.height <= a.y;
                assert!(apart, "{a:?} lies on {b:?}");
            }
        }
        let right = tiles.iter().map(|tile| tile.x + tile.width).max();
        let bottom = tiles.iter().map(|tile| tile.y + tile.height).max();
        assert_eq!((right, bottom), (Some(1008), Some(608)));
    }
}
