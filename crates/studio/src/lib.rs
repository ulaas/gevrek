//! gevrek-studio: the settings program.
//!
//! - [`document`]: the configuration being edited, with undo and the
//!   model's own check after every change;
//! - [`form`]: forms drawn from the schema (`gevrek-schema`), so that every
//!   setting has one;
//! - [`pages`]: which settings each page shows;
//! - [`session`]: revisions and conflicts with the window manager;
//! - [`link`]: the line to the window manager, on its own thread;
//! - [`graph`]: the automation graph, on a sheet;
//! - [`app`]: the window.
#[cfg(target_os = "linux")]
pub mod all_keys;
pub mod app;
pub mod bindings;
pub mod displays;
pub mod document;
pub mod form;
pub mod gesture_live;
pub mod graph;
pub mod history;
pub mod inspect;
pub mod keyboard_layouts;
pub mod night_light;
/// The line to the window manager (its own crate: the shell uses it too).
pub use gevrek_link as link;
pub mod pages;
pub mod panel_modules;
#[cfg(target_os = "linux")]
pub mod pinch;
pub mod preview;
pub mod rawtext;
pub mod reorder;
pub mod rules_live;
pub mod sample;
pub mod sandbox;
pub mod session;
pub mod shortcuts;
/// The system's own idle times, where the system keeps them.
#[cfg(windows)]
pub mod system_idle;
pub mod theme_card;
pub mod tryout;
pub mod wallpaper_preview;
pub mod welcome;
pub mod zones_edit;
