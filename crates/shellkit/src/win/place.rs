//! Where a surface goes on a display, worked out without asking Windows:
//! the rectangle a [`Place`] means, the strip an edge panel keeps free,
//! where a popup finds room, and the names windows and displays go by.

use crate::{Edge, Place};

/// A rectangle on the desktop, in the pixels of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Area {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn holds(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// How much of one rectangle lies in another, in pixels.
pub fn shared(one: Area, other: Area) -> i64 {
    let width = one.right().min(other.right()) - one.x.max(other.x);
    let height = one.bottom().min(other.bottom()) - one.y.max(other.y);
    if width <= 0 || height <= 0 {
        0
    } else {
        i64::from(width) * i64::from(height)
    }
}

/// The display a rectangle is on: the one that holds most of it (the
/// first of equals), by its place in `displays`; `None` when no display
/// holds any of it.
pub fn display_holding(area: Area, displays: &[Area]) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (index, display) in displays.iter().enumerate() {
        let held = shared(area, *display);
        if held > 0 && best.is_none_or(|(_, most)| held > most) {
            best = Some((index, held));
        }
    }
    best.map(|(index, _)| index)
}

/// A length in logical pixels as pixels of a display with this scale.
pub fn pixels(logical: i32, scale: f32) -> i32 {
    (logical as f32 * scale).round() as i32
}

/// The rectangle of a surface within the room a display has for it.
/// A length of 0 is all the room there is; a surface never leaves it.
pub fn placed(place: &Place, within: Area, scale: f32) -> Area {
    let margin = pixels(place.margin, scale);
    let room = Area {
        x: within.x + margin,
        y: within.y + margin,
        width: (within.width - 2 * margin).max(1),
        height: (within.height - 2 * margin).max(1),
    };
    let length = |logical: u32, all: i32| {
        if logical == 0 {
            all
        } else {
            pixels(logical as i32, scale).clamp(1, all)
        }
    };
    let (width, height) = if place.edge == Edge::Fill {
        (room.width, room.height)
    } else {
        (
            length(place.width, room.width),
            length(place.height, room.height),
        )
    };
    let left = room.x;
    let right = room.right() - width;
    let across = room.x + (room.width - width) / 2;
    let top = room.y;
    let bottom = room.bottom() - height;
    let middle = room.y + (room.height - height) / 2;
    let (x, y) = match place.edge {
        Edge::Top => (across, top),
        Edge::Bottom => (across, bottom),
        Edge::Left => (left, middle),
        Edge::Right => (right, middle),
        Edge::TopLeft | Edge::Fill => (left, top),
        Edge::TopRight => (right, top),
        Edge::BottomLeft => (left, bottom),
        Edge::BottomRight => (right, bottom),
        Edge::Centre => (across, middle),
    };
    Area {
        x,
        y,
        width,
        height,
    }
}

/// The side of a display an application bar (Windows' name for a window
/// that others keep clear of) is fixed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

impl Side {
    /// The number Windows knows the side by (`ABE_LEFT` and the others).
    pub fn number(self) -> u32 {
        match self {
            Side::Left => 0,
            Side::Top => 1,
            Side::Right => 2,
            Side::Bottom => 3,
        }
    }
}

/// The side a surface keeps free, if it keeps any: only a surface along
/// one whole edge can, as on Linux.
pub fn reserving_side(place: &Place) -> Option<Side> {
    if !place.reserve {
        return None;
    }
    match place.edge {
        Edge::Top => Some(Side::Top),
        Edge::Bottom => Some(Side::Bottom),
        Edge::Left => Some(Side::Left),
        Edge::Right => Some(Side::Right),
        _ => None,
    }
}

/// How deep the strip is that windows keep clear of: the surface and
/// the gap between it and the screen's edge.
pub fn reserved_depth(place: &Place, side: Side, scale: f32) -> i32 {
    let surface = match side {
        Side::Top | Side::Bottom => place.height,
        Side::Left | Side::Right => place.width,
    };
    (pixels(surface as i32, scale) + pixels(place.margin, scale)).max(1)
}

/// The strip along a side of a display, `depth` deep: what is asked of
/// Windows first.
pub fn strip(side: Side, display: Area, depth: i32) -> Area {
    match side {
        Side::Top => Area::new(display.x, display.y, display.width, depth),
        Side::Bottom => Area::new(display.x, display.bottom() - depth, display.width, depth),
        Side::Left => Area::new(display.x, display.y, depth, display.height),
        Side::Right => Area::new(display.right() - depth, display.y, depth, display.height),
    }
}

/// Windows answers the first question with a rectangle moved clear of the
/// bars already there, which may have lost its depth on the way: this is
/// the strip of the right depth at the edge Windows left it at.
pub fn settled(side: Side, answered: Area, depth: i32) -> Area {
    match side {
        Side::Top => Area::new(answered.x, answered.y, answered.width, depth),
        Side::Bottom => Area::new(answered.x, answered.bottom() - depth, answered.width, depth),
        Side::Left => Area::new(answered.x, answered.y, depth, answered.height),
        Side::Right => Area::new(answered.right() - depth, answered.y, depth, answered.height),
    }
}

/// The rectangle of a surface in the strip kept clear for it: the margin
/// is at the screen's edge and at the strip's ends, and windows begin
/// where the surface ends.
pub fn placed_in_strip(place: &Place, side: Side, kept: Area, scale: f32) -> Area {
    // The room a strip gives has no margin at its inner side: it is
    // made that much deeper for the reckoning.
    let margin = pixels(place.margin, scale);
    let room = match side {
        Side::Top => Area::new(kept.x, kept.y, kept.width, kept.height + margin),
        Side::Bottom => Area::new(kept.x, kept.y - margin, kept.width, kept.height + margin),
        Side::Left => Area::new(kept.x, kept.y, kept.width + margin, kept.height),
        Side::Right => Area::new(kept.x - margin, kept.y, kept.width + margin, kept.height),
    };
    placed(place, room, scale)
}

/// Where a popup goes: below the rectangle it belongs to, their left
/// edges in line; above it where it would leave the display below, and
/// slid along where it would leave it at a side. As the Linux host asks
/// of the compositor.
pub fn popup_at(anchor: Area, size: (i32, i32), display: Area, gap: i32) -> Area {
    let (width, height) = (size.0.max(1), size.1.max(1));
    let below = anchor.bottom() + gap;
    let above = anchor.y - gap - height;
    let y = if below + height <= display.bottom() || above < display.y {
        below
    } else {
        above
    };
    let x = anchor.x.min(display.right() - width).max(display.x);
    Area {
        x,
        y,
        width,
        height,
    }
}

/// Where a hint beside a panel that goes down a side lies: to the right
/// of the rectangle it is about, their middles in line; to its left where
/// it would leave the display at the right, and slid along where it would
/// leave it above or below.
pub fn beside(anchor: Area, size: (i32, i32), display: Area, gap: i32) -> Area {
    let (width, height) = (size.0.max(1), size.1.max(1));
    let right = anchor.right() + gap;
    let left = anchor.x - gap - width;
    let x = if right + width <= display.right() || left < display.x {
        right
    } else {
        left
    };
    let y = (anchor.y + (anchor.height - height) / 2)
        .min(display.bottom() - height)
        .max(display.y);
    Area {
        x,
        y,
        width,
        height,
    }
}

/// Every window class of the shell's surfaces begins with this; the
/// window manager leaves such windows alone (its rules match the start).
pub const CLASS_PREFIX: &str = "Gevrek_Shell";

const SURFACE_CLASS: &str = "Gevrek_Shell_";
const POPUP_CLASS: &str = "Gevrek_ShellPopup_";
const HINT_CLASS: &str = "Gevrek_ShellHint_";

/// The window class of a hint of the surface with this name.
pub fn hint_class_of(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|letter| if letter.is_control() { '_' } else { letter })
        .take(200)
        .collect();
    format!("{HINT_CLASS}{name}")
}

/// The window class of a surface with this name, or of a popup of it.
/// The name is in the class so that a rule, or a look at the windows,
/// can tell a panel from a launcher without reading titles.
pub fn class_of(name: &str, popup: bool) -> String {
    // Windows takes 256 letters for a class name at most.
    let name: String = name
        .chars()
        .map(|letter| if letter.is_control() { '_' } else { letter })
        .take(200)
        .collect();
    format!("{}{name}", if popup { POPUP_CLASS } else { SURFACE_CLASS })
}

/// The surface's name from a window class, and whether it is a popup's;
/// `None` for a class that is not a shell surface's.
pub fn name_of(class: &str) -> Option<(&str, bool)> {
    if let Some(name) = class.strip_prefix(POPUP_CLASS) {
        return Some((name, true));
    }
    class.strip_prefix(SURFACE_CLASS).map(|name| (name, false))
}

/// The name a display goes by: Windows' own (`\\.\DISPLAY1`) without the
/// marks in front.
pub fn display_name(device: &str) -> &str {
    device.strip_prefix(r"\\.\").unwrap_or(device)
}

/// Whether a display is the one asked for, however it is written.
pub fn is_display(wanted: &str, device: &str) -> bool {
    display_name(wanted).eq_ignore_ascii_case(display_name(device))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Keyboard, Level};

    const DISPLAY: Area = Area {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    fn place(edge: Edge, width: u32, height: u32, margin: i32) -> Place {
        Place {
            name: "test".to_string(),
            edge,
            level: Level::Above,
            width,
            height,
            reserve: false,
            keyboard: Keyboard::Never,
            margin,
            output: None,
        }
    }

    #[test]
    fn a_hint_lies_beside_what_it_is_about_where_there_is_room() {
        // A panel down the left side: the hint to its right, middles in line.
        let anchor = Area::new(4, 100, 40, 30);
        assert_eq!(
            beside(anchor, (120, 20), DISPLAY, 6),
            Area::new(50, 105, 120, 20)
        );
        // Down the right side: to the left.
        let anchor = Area::new(1876, 100, 40, 30);
        assert_eq!(
            beside(anchor, (120, 20), DISPLAY, 6),
            Area::new(1750, 105, 120, 20)
        );
        // At the display's upper end it is slid down on to the display.
        let anchor = Area::new(4, 0, 40, 10);
        assert_eq!(beside(anchor, (120, 40), DISPLAY, 6).y, 0);
    }

    #[test]
    fn a_panel_at_the_top_is_as_wide_as_its_display() {
        let panel = Place::panel("panel", 40);
        assert_eq!(placed(&panel, DISPLAY, 1.0), Area::new(0, 0, 1920, 40));
    }

    #[test]
    fn a_panel_on_a_scaled_display_is_as_many_pixels_higher() {
        let panel = Place::panel("panel", 40);
        assert_eq!(placed(&panel, DISPLAY, 1.5), Area::new(0, 0, 1920, 60));
        assert_eq!(placed(&panel, DISPLAY, 1.25), Area::new(0, 0, 1920, 50));
    }

    #[test]
    fn a_panel_on_the_second_display_starts_where_that_display_does() {
        let second = Area::new(1920, -200, 2560, 1440);
        let panel = place(Edge::Bottom, 0, 30, 0);
        assert_eq!(placed(&panel, second, 1.0), Area::new(1920, 1210, 2560, 30));
    }

    #[test]
    fn a_margin_keeps_a_panel_off_every_edge_it_would_touch() {
        let panel = place(Edge::Top, 0, 40, 8);
        assert_eq!(placed(&panel, DISPLAY, 1.0), Area::new(8, 8, 1904, 40));
        let side = place(Edge::Right, 60, 0, 8);
        assert_eq!(placed(&side, DISPLAY, 1.0), Area::new(1852, 8, 60, 1064));
    }

    #[test]
    fn a_short_panel_lies_in_the_middle_of_its_edge() {
        let panel = place(Edge::Top, 600, 40, 0);
        assert_eq!(placed(&panel, DISPLAY, 1.0), Area::new(660, 0, 600, 40));
        let side = place(Edge::Left, 60, 400, 0);
        assert_eq!(placed(&side, DISPLAY, 1.0), Area::new(0, 340, 60, 400));
    }

    #[test]
    fn a_notification_lies_in_its_corner_with_its_margin() {
        let note = place(Edge::TopRight, 360, 90, 12);
        assert_eq!(placed(&note, DISPLAY, 1.0), Area::new(1548, 12, 360, 90));
        let note = place(Edge::BottomLeft, 360, 90, 12);
        assert_eq!(placed(&note, DISPLAY, 1.0), Area::new(12, 978, 360, 90));
        let note = place(Edge::BottomRight, 360, 90, 0);
        assert_eq!(placed(&note, DISPLAY, 1.0), Area::new(1560, 990, 360, 90));
        let note = place(Edge::TopLeft, 360, 90, 0);
        assert_eq!(placed(&note, DISPLAY, 1.0), Area::new(0, 0, 360, 90));
    }

    #[test]
    fn a_launcher_lies_in_the_middle_of_the_display() {
        let launcher = place(Edge::Centre, 640, 420, 0);
        assert_eq!(
            placed(&launcher, DISPLAY, 1.0),
            Area::new(640, 330, 640, 420)
        );
        // On a scaled display it is larger, and still in the middle.
        assert_eq!(
            placed(&launcher, DISPLAY, 2.0),
            Area::new(320, 120, 1280, 840)
        );
    }

    #[test]
    fn a_surface_that_fills_covers_the_display_whatever_size_it_names() {
        let cover = place(Edge::Fill, 10, 10, 0);
        assert_eq!(placed(&cover, DISPLAY, 1.0), DISPLAY);
    }

    #[test]
    fn a_surface_larger_than_the_display_is_cut_to_it() {
        let launcher = place(Edge::Centre, 4000, 3000, 0);
        assert_eq!(placed(&launcher, DISPLAY, 1.0), DISPLAY);
    }

    #[test]
    fn only_a_surface_along_an_edge_keeps_room_free() {
        let mut panel = Place::panel("panel", 40);
        assert_eq!(reserving_side(&panel), Some(Side::Top));
        panel.edge = Edge::Left;
        assert_eq!(reserving_side(&panel), Some(Side::Left));
        panel.edge = Edge::TopRight;
        assert_eq!(reserving_side(&panel), None);
        panel.edge = Edge::Bottom;
        panel.reserve = false;
        assert_eq!(reserving_side(&panel), None);
    }

    #[test]
    fn the_sides_have_the_numbers_windows_gives_them() {
        use windows::Win32::UI::Shell::{ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP};
        assert_eq!(Side::Left.number(), ABE_LEFT);
        assert_eq!(Side::Top.number(), ABE_TOP);
        assert_eq!(Side::Right.number(), ABE_RIGHT);
        assert_eq!(Side::Bottom.number(), ABE_BOTTOM);
    }

    #[test]
    fn the_room_kept_free_is_the_panel_and_its_margin() {
        let mut panel = Place::panel("panel", 40);
        panel.margin = 6;
        assert_eq!(reserved_depth(&panel, Side::Top, 1.0), 46);
        assert_eq!(reserved_depth(&panel, Side::Top, 1.5), 69);
        assert_eq!(strip(Side::Top, DISPLAY, 46), Area::new(0, 0, 1920, 46));
        assert_eq!(
            strip(Side::Bottom, DISPLAY, 46),
            Area::new(0, 1034, 1920, 46)
        );
        assert_eq!(strip(Side::Left, DISPLAY, 46), Area::new(0, 0, 46, 1080));
        assert_eq!(
            strip(Side::Right, DISPLAY, 46),
            Area::new(1874, 0, 46, 1080)
        );
    }

    #[test]
    fn a_panel_with_a_margin_lies_at_the_inner_side_of_its_strip() {
        let mut panel = Place::panel("panel", 40);
        panel.margin = 6;
        let kept = strip(Side::Top, DISPLAY, reserved_depth(&panel, Side::Top, 1.0));
        // Windows end where the panel ends: the margin is at the screen's edge.
        assert_eq!(
            placed_in_strip(&panel, Side::Top, kept, 1.0),
            Area::new(6, 6, 1908, 40)
        );
        panel.edge = Edge::Bottom;
        let kept = strip(Side::Bottom, DISPLAY, 46);
        assert_eq!(
            placed_in_strip(&panel, Side::Bottom, kept, 1.0),
            Area::new(6, 1034, 1908, 40)
        );
        panel.edge = Edge::Left;
        panel.width = 50;
        panel.height = 0;
        let kept = strip(Side::Left, DISPLAY, 56);
        assert_eq!(
            placed_in_strip(&panel, Side::Left, kept, 1.0),
            Area::new(6, 6, 50, 1068)
        );
        panel.edge = Edge::Right;
        let kept = strip(Side::Right, DISPLAY, 56);
        assert_eq!(
            placed_in_strip(&panel, Side::Right, kept, 1.0),
            Area::new(1864, 6, 50, 1068)
        );
    }

    #[test]
    fn a_strip_moved_by_another_bar_keeps_its_depth() {
        // The taskbar is at the top already: Windows moved the strip below
        // it, past its own far edge.
        let answered = Area::new(0, 48, 1920, -8);
        assert_eq!(settled(Side::Top, answered, 40), Area::new(0, 48, 1920, 40));
        // At the bottom, the strip ends where the taskbar begins.
        let answered = Area::new(0, 1040, 1920, 0);
        assert_eq!(
            settled(Side::Bottom, answered, 40),
            Area::new(0, 1000, 1920, 40)
        );
        let answered = Area::new(1870, 0, -10, 1080);
        assert_eq!(
            settled(Side::Right, answered, 50),
            Area::new(1810, 0, 50, 1080)
        );
        let answered = Area::new(60, 0, 10, 1080);
        assert_eq!(
            settled(Side::Left, answered, 50),
            Area::new(60, 0, 50, 1080)
        );
    }

    #[test]
    fn a_menu_opens_below_its_button_their_left_edges_in_line() {
        let button = Area::new(300, 5, 120, 30);
        assert_eq!(
            popup_at(button, (220, 130), DISPLAY, 4),
            Area::new(300, 39, 220, 130)
        );
    }

    #[test]
    fn a_menu_of_a_panel_at_the_bottom_opens_upwards() {
        let button = Area::new(300, 1045, 120, 30);
        assert_eq!(
            popup_at(button, (220, 130), DISPLAY, 4),
            Area::new(300, 911, 220, 130)
        );
    }

    #[test]
    fn a_menu_at_the_right_edge_is_slid_back_onto_the_display() {
        let button = Area::new(1850, 5, 60, 30);
        assert_eq!(
            popup_at(button, (220, 130), DISPLAY, 4),
            Area::new(1700, 39, 220, 130)
        );
        // On a display to the left of the first, it never leaves by the left.
        let left = Area::new(-1920, 0, 1920, 1080);
        let button = Area::new(-1930, 5, 60, 30);
        assert_eq!(popup_at(button, (220, 130), left, 4).x, -1920);
    }

    #[test]
    fn a_surfaces_name_is_read_back_from_its_window_class() {
        let class = class_of("gevrek-panel", false);
        assert_eq!(class, "Gevrek_Shell_gevrek-panel");
        assert!(class.starts_with(CLASS_PREFIX));
        assert_eq!(name_of(&class), Some(("gevrek-panel", false)));
        let popup = class_of("gevrek-panel", true);
        assert!(popup.starts_with(CLASS_PREFIX));
        assert_eq!(name_of(&popup), Some(("gevrek-panel", true)));
        assert_eq!(name_of("Gevrek_Border"), None);
        assert_eq!(name_of("Shell_TrayWnd"), None);
    }

    #[test]
    fn a_name_no_window_class_can_carry_is_made_to_fit() {
        let class = class_of(&"x".repeat(1000), false);
        assert!(class.encode_utf16().count() < 256);
        assert_eq!(class_of("a\0b", false), "Gevrek_Shell_a_b");
    }

    #[test]
    fn a_display_is_found_by_its_name_with_or_without_windows_marks() {
        assert_eq!(display_name(r"\\.\DISPLAY1"), "DISPLAY1");
        assert!(is_display("DISPLAY1", r"\\.\DISPLAY1"));
        assert!(is_display(r"\\.\DISPLAY1", r"\\.\DISPLAY1"));
        assert!(is_display("display1", r"\\.\DISPLAY1"));
        assert!(!is_display("DISPLAY2", r"\\.\DISPLAY1"));
    }

    #[test]
    fn a_point_is_in_an_area_up_to_its_far_edges() {
        let area = Area::new(10, 10, 100, 50);
        assert!(area.holds(10, 10));
        assert!(area.holds(109, 59));
        assert!(!area.holds(110, 59));
        assert!(!area.holds(9, 30));
    }
    #[test]
    fn a_rectangle_is_on_the_display_that_holds_most_of_it() {
        let left = Area::new(0, 0, 1920, 1080);
        let right = Area::new(1920, 0, 2560, 1440);
        let displays = [left, right];
        // A panel along the top of the second display.
        assert_eq!(
            display_holding(Area::new(1926, 6, 2548, 36), &displays),
            Some(1)
        );
        // Across the border: where most of it is.
        assert_eq!(
            display_holding(Area::new(1800, 0, 400, 30), &displays),
            Some(1)
        );
        assert_eq!(
            display_holding(Area::new(1700, 0, 400, 30), &displays),
            Some(0)
        );
        // Half and half: the first.
        assert_eq!(
            display_holding(Area::new(1820, 0, 200, 30), &displays),
            Some(0)
        );
        // Far off every display, or with no size: on none.
        assert_eq!(
            display_holding(Area::new(-32000, -32000, 200, 30), &displays),
            None
        );
        assert_eq!(display_holding(Area::new(10, 10, 0, 30), &displays), None);
        assert_eq!(display_holding(Area::new(10, 10, 100, 30), &[]), None);
    }
}
