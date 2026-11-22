//! Where a window waits while its workspace is not shown.
//!
//! Windows can cloak a window and a Wayland compositor simply does not draw
//! it; macOS has neither for another program's window. Minimizing would
//! work, but it animates into the Dock and takes the window out of the
//! order the user knows. Programs can be hidden only whole, not one window.
//! Spaces (the system's own desktops) cannot be driven from outside without
//! turning off System Integrity Protection.
//!
//! What is left is to move the window out of the way, and macOS does not
//! let a window go wholly off the screen. Measured on macOS 27 with a
//! Finder window on a 1512x982-point display:
//!
//! - asked to stand 1 point inside the bottom-right corner, it stood there
//!   (x = 1511): one point of it showed at the right;
//! - asked to stand at the corner itself, it was put back to show 40
//!   points;
//! - at the bottom it always kept 52 points showing (its title bar).
//!
//! So a waiting window is a sliver in a bottom corner of its display, one
//! point wide. The corner chosen is one beyond which there is no other
//! display: a window parked against a neighbouring display would stand on
//! that display instead. Where every corner has a neighbour (displays on
//! both sides), the corner where the least of the window lands on another
//! display is chosen.

use gevrek_model::geometry::{Point, Rect};

/// How much of a waiting window is left on its display at the side: the
/// least macOS let stand (it moved a window asked for 0 back to 40).
pub const KEPT_WIDE: i32 = 1;

/// How much of a waiting window macOS keeps on the display at the bottom,
/// whatever it is asked: the window really stands this much higher than it
/// is asked to. Used to know where it ends up, not to ask.
pub const KEPT_HIGH: i32 = 52;

/// How much of a window macOS leaves showing at the side when it is asked
/// to put it wholly off the display: the most a parked window can show
/// there, if macOS ever moves it back.
pub const PUSHED_BACK: i32 = 40;

/// A bottom corner of a display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    BottomRight,
    BottomLeft,
}

/// Where to put a window of `size` (width, height) to wait on `display`,
/// with the other displays at `others`; and which corner that is. The
/// point is the window's top-left as it is to be asked for; macOS keeps
/// [`KEPT_HIGH`] of it showing at the bottom.
pub fn spot(size: (i32, i32), display: Rect, others: &[Rect]) -> (Corner, Point) {
    let (width, height) = size;
    let bottom = display.y + display.height;
    let ask_y = bottom - KEPT_WIDE;
    let at = |corner| match corner {
        Corner::BottomRight => Point {
            x: display.x + display.width - KEPT_WIDE,
            y: ask_y,
        },
        Corner::BottomLeft => Point {
            x: display.x - width + KEPT_WIDE,
            y: ask_y,
        },
    };
    // Where the window really ends up: macOS lifts it until KEPT_HIGH of it
    // shows.
    let landed = |corner| {
        let point = at(corner);
        Rect::new(point.x, bottom - KEPT_HIGH, width, height)
    };
    let spilt = |corner| -> i64 {
        others
            .iter()
            .map(|other| overlap(landed(corner), *other))
            .sum()
    };
    let corner = [Corner::BottomRight, Corner::BottomLeft]
        .into_iter()
        .min_by_key(|corner| spilt(*corner))
        .unwrap_or(Corner::BottomRight);
    (corner, at(corner))
}

/// The area two rectangles share, in square points.
fn overlap(a: Rect, b: Rect) -> i64 {
    let width = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let height = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if width <= 0 || height <= 0 {
        0
    } else {
        i64::from(width) * i64::from(height)
    }
}

/// Whether a window at `frame` is one this backend parked on `display`
/// (it may have to be told after a crash, from where the window stands):
/// it shows no more than the slivers parking leaves.
pub fn looks_parked(frame: Rect, display: Rect) -> bool {
    let shown = overlap(frame, display);
    let sliver = i64::from(PUSHED_BACK) * i64::from(KEPT_HIGH);
    shown > 0 && shown <= sliver && frame.y + KEPT_HIGH >= display.y + display.height - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Rect = Rect {
        x: 0,
        y: 0,
        width: 1512,
        height: 982,
    };
    const WINDOW: (i32, i32) = (920, 436);

    #[test]
    fn alone_it_waits_bottom_right_with_one_point_showing() {
        let (corner, at) = spot(WINDOW, LAPTOP, &[]);
        assert_eq!(corner, Corner::BottomRight);
        assert_eq!(at, Point { x: 1511, y: 981 });
    }

    #[test]
    fn with_a_display_to_the_right_it_waits_bottom_left() {
        let right = Rect::new(1512, 0, 2560, 1440);
        let (corner, at) = spot(WINDOW, LAPTOP, &[right]);
        assert_eq!(corner, Corner::BottomLeft);
        assert_eq!(at, Point { x: 1 - 920, y: 981 });
    }

    #[test]
    fn with_a_display_to_the_left_it_still_waits_bottom_right() {
        let left = Rect::new(-1920, 0, 1920, 1080);
        assert_eq!(spot(WINDOW, LAPTOP, &[left]).0, Corner::BottomRight);
    }

    #[test]
    fn a_display_above_is_no_neighbour_of_a_bottom_corner() {
        let above = Rect::new(-200, -1080, 1920, 1080);
        assert_eq!(spot(WINDOW, LAPTOP, &[above]).0, Corner::BottomRight);
    }

    #[test]
    fn between_two_displays_it_takes_the_side_where_less_of_it_spills() {
        // To the right a display that reaches down past the window; to the
        // left one that ends just under the laptop's bottom edge.
        let right = Rect::new(1512, 0, 2560, 1440);
        let left = Rect::new(-1920, 900, 1920, 100);
        assert_eq!(spot(WINDOW, LAPTOP, &[right, left]).0, Corner::BottomLeft);
        let left_tall = Rect::new(-1920, 0, 1920, 2000);
        let right_short = Rect::new(1512, 900, 1920, 100);
        assert_eq!(
            spot(WINDOW, LAPTOP, &[left_tall, right_short]).0,
            Corner::BottomRight
        );
    }

    #[test]
    fn a_display_below_and_to_the_right_counts() {
        let below_right = Rect::new(1512, 982, 1920, 1080);
        assert_eq!(spot(WINDOW, LAPTOP, &[below_right]).0, Corner::BottomLeft);
    }

    #[test]
    fn where_it_landed_is_told_from_where_it_was_left() {
        // As macOS left it: x as asked, lifted to show 52 points.
        let landed = Rect::new(1511, 930, 920, 436);
        assert!(looks_parked(landed, LAPTOP));
        // Pushed back by macOS to show 40 points: still parked.
        assert!(looks_parked(Rect::new(1472, 930, 920, 436), LAPTOP));
        // A window in the middle of the display, or half off it, is not.
        assert!(!looks_parked(Rect::new(377, 330, 920, 436), LAPTOP));
        assert!(!looks_parked(Rect::new(1000, 930, 920, 436), LAPTOP));
        // Nor one wholly on another display.
        assert!(!looks_parked(Rect::new(2000, 100, 920, 436), LAPTOP));
    }
}
