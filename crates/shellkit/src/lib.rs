//! Surfaces for gevrek's shell.
//!
//! A panel, a launcher or a notification is not a window: it has a place
//! on the screen that the desktop gives it (an edge, a layer above or
//! below the windows, room kept free for it), and it must not be tiled.
//! The toolkit's usual window host cannot ask for that. This crate is the
//! host that can: it makes the surface, feeds what happens on it to egui,
//! and puts what egui draws on it, with the graphics card.
//!
//! - Linux: a layer-shell surface of the Wayland compositor ([`layer`]).
//! - Windows: a borderless tool window, an application bar where it keeps
//!   room free ([`win`]).
//!
//! The two halves have the same shape (`Shell`, `Control`, `SurfaceId`,
//! `Sender`, `Draw`, `run`), and [`host`] is the one of the system the
//! program is built for: what is written against it needs no word about
//! the system. The session lock is in the Linux half only; on Windows the
//! system has its own.
//!
//! Files dragged between a surface and other programs are in [`dnd`]: a
//! surface asks for them, and one that does not is as it was.
//!
//! What a surface is beyond its place and its picture is asked for while
//! it is open, through `Control`, and a surface that never asks is as it
//! was: another level among the windows (`set_level`), the desktop shown
//! soft behind parts of it ([`look`], `set_blur_regions`), and being left
//! out of pictures taken of the screen (`exclude_from_capture`).

pub mod dnd;
pub mod look;
#[cfg(target_os = "linux")]
pub mod layer;
#[cfg(windows)]
pub mod win;

#[cfg(target_os = "linux")]
pub use layer as host;
#[cfg(windows)]
pub use win as host;

/// Which edge of the screen a surface is fixed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    /// A corner (notifications).
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    /// Every edge: the whole display (something to choose a part of it on).
    Fill,
    /// No edge: in the middle of the screen (a launcher, a dialog).
    Centre,
}

/// How a surface lies among the windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Behind everything (a wallpaper).
    Background,
    /// Above the wallpaper, below the windows.
    Below,
    /// Above the windows (a panel).
    Above,
    /// Above everything, fullscreen windows too (a launcher, a lock).
    Overlay,
    /// Above everything, and nothing of it can be pressed: the pointer
    /// and its presses go through to what lies under it (marks drawn
    /// over the desktop: a ring around what a lesson is about).
    Marks,
}

/// Whether a surface gets the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyboard {
    Never,
    /// When it is clicked, like a window.
    OnDemand,
    /// Always, while it is shown (a launcher).
    Exclusive,
}

/// What a surface asks of the desktop.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// A name the compositor knows the surface by (for its rules).
    pub name: String,
    pub edge: Edge,
    pub level: Level,
    /// In logical pixels. Along the edge it is fixed to, 0 means as long
    /// as the screen.
    pub width: u32,
    pub height: u32,
    /// Windows keep clear of the surface (a panel), or lie under it.
    pub reserve: bool,
    pub keyboard: Keyboard,
    /// Room between the surface and the screen's edges.
    pub margin: i32,
    /// The display it is on, by the name of its connector; `None`: the
    /// one the compositor picks (where the pointer or the keyboard is).
    pub output: Option<String>,
}

impl Place {
    /// Room at an edge of a display that windows keep clear of, with
    /// nothing in it: `depth` logical pixels along the whole of `edge`
    /// (`Top`, `Bottom`, `Left` or `Right`). For what is drawn elsewhere
    /// and wants the room: a surface that fills its display cannot keep
    /// any, so the zones of a desktop put one of these beside theirs.
    /// Whoever opens it draws nothing on it and lets the pointer through
    /// (`Control::reach` with no parts, on Linux); its depth is changed
    /// with `Control::resize`.
    pub fn reserved_strip(name: &str, edge: Edge, depth: u32, output: Option<String>) -> Self {
        let upright = matches!(edge, Edge::Left | Edge::Right);
        Self {
            name: name.to_string(),
            edge,
            level: Level::Background,
            width: if upright { depth } else { 0 },
            height: if upright { 0 } else { depth },
            reserve: true,
            keyboard: Keyboard::Never,
            margin: 0,
            output,
        }
    }

    /// A bar along the top of the screen that windows keep clear of.
    pub fn panel(name: &str, height: u32) -> Self {
        Self {
            name: name.to_string(),
            edge: Edge::Top,
            level: Level::Above,
            width: 0,
            height,
            reserve: true,
            keyboard: Keyboard::OnDemand,
            margin: 0,
            output: None,
        }
    }
}

/// A button of the pointer going down on a surface, as the system saw it.
///
/// A drawing hears of presses through egui as well; what egui cannot say
/// is when the system saw each one. A frame is drawn when the shell gets
/// to it, and a shell that was busy for a moment draws two presses later
/// and further apart than they were made: counted by the time of the
/// frames, a double press may then be none. Whoever must tell a double
/// press from two single ones (the desktop's surfaces do) counts by these
/// ([`presses::Counter`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Press {
    pub button: egui::PointerButton,
    /// Where, in the surface's own points.
    pub at: egui::Pos2,
    /// The system's time of the press in thousandths of a second. It
    /// begins anywhere and starts again from nothing after some weeks:
    /// only how far two of them are apart means something
    /// (`later.wrapping_sub(earlier)`).
    pub time_ms: u32,
}

pub mod presses {
    //! Telling a double press from two single ones.

    use super::Press;

    /// Counts the presses of each button that follow each other quickly
    /// and close together. A third press in quick succession begins anew:
    /// it is the first of the next pair, so that pressing four times does
    /// twice what pressing twice does.
    #[derive(Debug, Clone, Default)]
    pub struct Counter {
        /// The press of each button that a next one may make a pair with.
        last: Vec<Press>,
    }

    impl Counter {
        /// Take in a press. `true`: it is the second of a pair, by the
        /// longest time between the two in thousandths of a second and
        /// the farthest they may be apart in points.
        pub fn press(&mut self, press: Press, within_ms: u32, apart: f32) -> bool {
            let before = self.last.iter().position(|last| last.button == press.button);
            let second = before.is_some_and(|at| {
                let last = self.last[at];
                press.time_ms.wrapping_sub(last.time_ms) <= within_ms && last.at.distance(press.at) <= apart
            });
            if let Some(at) = before {
                self.last.remove(at);
            }
            if !second {
                self.last.push(press);
            }
            second
        }

        /// Forget what was pressed (the surface is another, or what was
        /// under the pointer is gone).
        pub fn forget(&mut self) {
            self.last.clear();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use egui::{pos2, PointerButton};

        fn press(button: PointerButton, x: f32, time_ms: u32) -> Press {
            Press { button, at: pos2(x, 10.0), time_ms }
        }

        #[test]
        fn two_presses_close_in_time_and_place_are_a_pair_and_the_third_begins_anew() {
            let mut counter = Counter::default();
            let main = PointerButton::Primary;
            assert!(!counter.press(press(main, 100.0, 1000), 500, 6.0));
            assert!(counter.press(press(main, 102.0, 1300), 500, 6.0));
            // The third is the first of the next pair, the fourth its second.
            assert!(!counter.press(press(main, 102.0, 1400), 500, 6.0));
            assert!(counter.press(press(main, 101.0, 1600), 500, 6.0));
        }

        #[test]
        fn too_late_or_too_far_is_a_first_press_again() {
            let mut counter = Counter::default();
            let main = PointerButton::Primary;
            counter.press(press(main, 100.0, 1000), 500, 6.0);
            assert!(!counter.press(press(main, 100.0, 1501), 500, 6.0), "too late");
            // (And that one counts as a first: the next in time is its second.)
            assert!(counter.press(press(main, 100.0, 1700), 500, 6.0));
            counter.press(press(main, 100.0, 3000), 500, 6.0);
            assert!(!counter.press(press(main, 120.0, 3100), 500, 6.0), "too far");
        }

        #[test]
        fn each_button_is_counted_by_itself() {
            let mut counter = Counter::default();
            assert!(!counter.press(press(PointerButton::Primary, 100.0, 1000), 500, 6.0));
            assert!(!counter.press(press(PointerButton::Middle, 100.0, 1100), 500, 6.0));
            assert!(counter.press(press(PointerButton::Middle, 100.0, 1200), 500, 6.0));
            assert!(counter.press(press(PointerButton::Primary, 100.0, 1300), 500, 6.0));
        }

        #[test]
        fn the_systems_clock_starting_again_breaks_no_pair() {
            let mut counter = Counter::default();
            let main = PointerButton::Primary;
            counter.press(press(main, 100.0, u32::MAX - 100), 500, 6.0);
            assert!(counter.press(press(main, 100.0, 150), 500, 6.0));
        }

        #[test]
        fn what_was_forgotten_makes_no_pair() {
            let mut counter = Counter::default();
            let main = PointerButton::Primary;
            counter.press(press(main, 100.0, 1000), 500, 6.0);
            counter.forget();
            assert!(!counter.press(press(main, 100.0, 1100), 500, 6.0));
        }
    }
}

/// What the drawing says after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// Close the surface and return from the host.
    Close,
}

/// How long the frames took, for the spike's measurements.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Timings {
    pub frames: u64,
    /// The time to lay out and draw a frame and hand it over, in
    /// milliseconds: the mean and the longest.
    pub mean_ms: f64,
    pub worst_ms: f64,
    /// From the start of the host to the first frame on the surface.
    pub first_frame_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reserved_strip_is_as_long_as_its_edge_and_takes_nothing() {
        let left = Place::reserved_strip("gevrek-zones-room", Edge::Left, 280, Some("DP-1".into()));
        assert_eq!((left.width, left.height), (280, 0));
        assert!(left.reserve);
        assert_eq!(left.keyboard, Keyboard::Never);
        assert_eq!(left.level, Level::Background);
        assert_eq!(left.output.as_deref(), Some("DP-1"));
        let top = Place::reserved_strip("gevrek-zones-room", Edge::Top, 96, None);
        assert_eq!((top.width, top.height), (0, 96));
    }
}
