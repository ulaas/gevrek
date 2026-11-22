//! The two ways macOS measures the desktop.
//!
//! The window server (Core Graphics) and the Accessibility API put the
//! origin at the top-left corner of the main display, with y growing
//! downwards, as the engine does. AppKit (`NSScreen`, `NSWindow`) puts it
//! at the bottom-left corner of the main display, with y growing upwards.
//! The main display is the one with the menu bar when the system started
//! (`NSScreen.screens[0]`), the one whose top-left is (0, 0) in the first
//! and whose bottom-left is (0, 0) in the second.
//!
//! Everything this backend hands the engine is in the first; AppKit's
//! rectangles are turned into it here, in one place.

use gevrek_model::geometry::Rect;

/// A rectangle as AppKit gives it: points, from the bottom-left of the
/// main display, y upwards. (Fractional: a scaled display has rectangles
/// that are not whole points.)
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AppKitRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `rect` measured from the top-left of the main display, whose height in
/// points is `main_height`, rounded to whole points: the top edge is
/// `main_height` less the AppKit top.
pub fn from_appkit(rect: AppKitRect, main_height: f64) -> Rect {
    let top = main_height - (rect.y + rect.height);
    Rect::new(
        rect.x.round() as i32,
        top.round() as i32,
        rect.width.round() as i32,
        rect.height.round() as i32,
    )
}

/// The other way: a rectangle of the engine's as AppKit measures it (an
/// overlay window gevrek draws a border with is placed in these).
pub fn to_appkit(rect: Rect, main_height: f64) -> AppKitRect {
    AppKitRect {
        x: f64::from(rect.x),
        y: main_height - f64::from(rect.y) - f64::from(rect.height),
        width: f64::from(rect.width),
        height: f64::from(rect.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: f64 = 982.0;

    #[test]
    fn the_main_display_is_the_same_rectangle_in_both() {
        let main = AppKitRect {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: MAIN,
        };
        assert_eq!(from_appkit(main, MAIN), Rect::new(0, 0, 1512, 982));
    }

    #[test]
    fn the_menu_bar_is_at_the_top_of_one_and_the_bottom_of_the_other() {
        // A visible frame without the 33-point menu bar: AppKit keeps its
        // origin at the bottom, the engine moves its top down.
        let visible = AppKitRect {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: MAIN - 33.0,
        };
        assert_eq!(from_appkit(visible, MAIN), Rect::new(0, 33, 1512, 949));
    }

    #[test]
    fn a_display_above_the_main_one_is_at_negative_y() {
        // 1080 points high, standing on top of the main display.
        let above = AppKitRect {
            x: -200.0,
            y: MAIN,
            width: 1920.0,
            height: 1080.0,
        };
        assert_eq!(from_appkit(above, MAIN), Rect::new(-200, -1080, 1920, 1080));
    }

    #[test]
    fn a_display_below_the_main_one_is_at_negative_appkit_y() {
        let below = AppKitRect {
            x: 0.0,
            y: -1200.0,
            width: 1920.0,
            height: 1200.0,
        };
        assert_eq!(from_appkit(below, MAIN), Rect::new(0, 982, 1920, 1200));
    }

    #[test]
    fn there_and_back_is_where_it_was() {
        for rect in [
            Rect::new(10, 20, 300, 400),
            Rect::new(-1920, -300, 1920, 1080),
            Rect::new(1512, 982, 1, 1),
        ] {
            assert_eq!(from_appkit(to_appkit(rect, MAIN), MAIN), rect);
        }
    }

    #[test]
    fn half_points_round_to_the_nearest() {
        let rect = AppKitRect {
            x: 10.5,
            y: 0.25,
            width: 99.6,
            height: 100.0,
        };
        assert_eq!(from_appkit(rect, MAIN), Rect::new(11, 882, 100, 100));
    }
}
