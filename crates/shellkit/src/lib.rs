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
