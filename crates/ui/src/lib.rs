//! gevrek-ui: the design system of every gevrek program.
//!
//! One look for the studio, the panel, the launcher, the lock screen and
//! every other surface gevrek draws: colour palettes with semantic tokens
//! ([`theme::Palette`]), the egui style built from them, fonts and icons,
//! and the widgets the programs are made of ([`widgets`]).
//!
//! It came from niri-studio (see `import/niri-studio/IMPORTED.md`).
//! Nothing in it knows about a window manager or a configuration: it draws.

pub mod brand;
pub mod brand_icons;
pub mod browser;
pub mod color;
pub mod controls;
pub mod fonts;
pub mod gallery;
#[cfg(feature = "picture")]
pub mod picture;
pub mod theme;
pub mod widgets;

pub use theme::{icons, pal, Accent, Density, Palette, Preset, Settings};
