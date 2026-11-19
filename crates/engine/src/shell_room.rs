//! Room the shell keeps on a display for what it draws on the desktop:
//! zones that tiled windows are to keep off, as they keep off a panel.
//!
//! The shell says which room that is (the zones' snapshot it publishes to
//! the server names it, for each display by the display's lasting id);
//! the window manager takes it off the display's work area
//! (`Outputs::set_shell_room`), after the room of the panel, since a
//! region's depth is measured from the side of what the panel leaves.
//!
//! Only where the window manager is the one that keeps room: on Windows.
//! A Wayland compositor is asked for room by the shell's own surfaces
//! (their exclusive zones), and takes it off there; it never looks here,
//! so nothing is kept twice.
//!
//! The server and the window manager's loop are two threads of one
//! program with no line between them for this. What the server last
//! heard is left here ([`tell`]) and the loop looks whether it is
//! another than it last took ([`told_since`]): a number that counts up,
//! and the regions under a lock that is held for a moment.

use gevrek_model::settings::ReservedArea;
use std::collections::HashMap;
use std::sync::Mutex;

/// A side of a display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

impl Side {
    /// The side of this name, as the zones write it (`left`, `top`,
    /// `right`, `bottom`).
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "left" => Some(Self::Left),
            "top" => Some(Self::Top),
            "right" => Some(Self::Right),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

/// Room kept at a side of a display: so deep, in the lengths the
/// configuration is written in (on a display with a larger scale it is
/// so many more pixels), from the side of what the panel leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRoom {
    /// The display's lasting id (`MonitorInfo::stable_id`).
    pub display: String,
    pub side: Side,
    pub depth: i32,
}

/// The rooms of each display as one reservation: at each side the
/// deepest of those kept there (two zones at the same side do not add
/// up; they lie in the same strip). Rooms of no depth count for nothing.
pub fn by_display(rooms: &[ShellRoom]) -> HashMap<String, ReservedArea> {
    let mut all: HashMap<String, ReservedArea> = HashMap::new();
    for room in rooms.iter().filter(|room| room.depth > 0) {
        let kept = all.entry(room.display.clone()).or_default();
        let side = match room.side {
            Side::Left => &mut kept.left,
            Side::Top => &mut kept.top,
            Side::Right => &mut kept.right,
            Side::Bottom => &mut kept.bottom,
        };
        *side = (*side).max(room.depth);
    }
    all
}

/// What was last told, and how often something else was told.
static TOLD: Mutex<(u64, Vec<ShellRoom>)> = Mutex::new((0, Vec::new()));

/// The shell keeps this room now (all of it: what is not named is given
/// back). Told again with nothing changed, it is no news.
pub fn tell(rooms: Vec<ShellRoom>) {
    let mut told = TOLD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if told.1 != rooms {
        told.0 += 1;
        told.1 = rooms;
    }
}

/// The room the shell keeps, if it is another than when `seen` was the
/// number given back: that number then, and the rooms. `None`: nothing
/// new. (Begin with a `seen` of 0: nothing was ever told then, and no
/// room is kept.)
pub fn told_since(seen: u64) -> Option<(u64, Vec<ShellRoom>)> {
    let told = TOLD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    (told.0 != seen).then(|| (told.0, told.1.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(display: &str, side: Side, depth: i32) -> ShellRoom {
        ShellRoom {
            display: display.to_string(),
            side,
            depth,
        }
    }

    #[test]
    fn the_rooms_of_a_display_are_the_deepest_at_each_side() {
        let all = by_display(&[
            room("edid:A", Side::Right, 280),
            room("edid:A", Side::Right, 120),
            room("edid:A", Side::Top, 96),
            room("edid:B", Side::Left, 64),
            room("edid:B", Side::Bottom, 0),
            room("edid:C", Side::Bottom, -40),
        ]);
        assert_eq!(
            all["edid:A"],
            ReservedArea {
                left: 0,
                top: 96,
                right: 280,
                bottom: 0
            }
        );
        assert_eq!(
            all["edid:B"],
            ReservedArea {
                left: 64,
                ..ReservedArea::default()
            }
        );
        // Room of no depth is no room.
        assert!(!all.contains_key("edid:C"));
        assert!(by_display(&[]).is_empty());
    }

    #[test]
    fn a_side_is_known_by_the_name_the_zones_write() {
        assert_eq!(Side::named("left"), Some(Side::Left));
        assert_eq!(Side::named("top"), Some(Side::Top));
        assert_eq!(Side::named("right"), Some(Side::Right));
        assert_eq!(Side::named("bottom"), Some(Side::Bottom));
        assert_eq!(Side::named("Top"), None);
        assert_eq!(Side::named(""), None);
    }

    #[test]
    fn what_is_told_is_news_once_and_only_when_it_is_another() {
        // (The one test that tells: what is told is one for the program.)
        let start = told_since(u64::MAX).expect("a number").0;
        tell(vec![room("edid:A", Side::Top, 96)]);
        let (first, rooms) = told_since(start).expect("news");
        assert_eq!(rooms, [room("edid:A", Side::Top, 96)]);
        assert_eq!(told_since(first), None);
        // The same again is no news.
        tell(vec![room("edid:A", Side::Top, 96)]);
        assert_eq!(told_since(first), None);
        // Given back.
        tell(Vec::new());
        let (second, rooms) = told_since(first).expect("news");
        assert!(rooms.is_empty() && second != first);
    }
}
