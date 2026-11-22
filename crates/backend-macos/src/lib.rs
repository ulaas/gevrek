//! gevrek-macos: the macOS backend.
//!
//! On macOS gevrek is a guest, as on Windows: the window server draws the
//! windows and gevrek moves them, through the Accessibility API. The host
//! around the core is the one every backend shares (`gevrek_host::wm`);
//! this crate is what only macOS has:
//!
//! - [`runtime`]: the loop on the main thread, and `WmHost` (`gevrek-wm`
//!   runs it: `runtime::run`).
//! - [`desktop`]: `Desktop` and `WindowQueries`: the windows as records,
//!   effects carried out on their elements.
//! - [`ax`], [`window_list`], [`observer`]: one window or program through
//!   the Accessibility API; the window server's list matched to elements;
//!   what happens to windows, as events.
//! - [`apps`]: the programs that run, and their bundle ids (what rules
//!   match as a window's application).
//! - [`displays`]: the displays, their work areas and scale.
//! - [`keys`]: the bindings' keys and mouse buttons, from an event tap
//!   (Win is Option there; see the module), and windows dragged with the
//!   mouse.
//! - [`park`] and [`journal`]: where a window of a hidden workspace waits,
//!   and the record that brings it back after a crash. macOS has no way to
//!   hide another program's window, and keeps a sliver of one on screen.
//! - [`space`]: AppKit measures from the bottom of the main display, the
//!   window server and the Accessibility API from the top.
//!
//! `park`, `space` and `journal` build (and are tested) on every system;
//! the rest only on macOS.
//!
//! - [`borders`]: the borders around windows, windows of gevrek's own.
//! - [`stackbars`]: the tabs of a container of several windows, panels of
//!   gevrek's own that are clicked.
//!
//! Not yet: animations (a program redraws for every size it is given: too slow),
//! touchpad gestures.
//!
//! Coordinates are points, the logical pixels the engine works in, with
//! the origin at the top-left of the main display.

pub mod journal;
pub mod park;
pub mod space;

#[cfg(target_os = "macos")]
mod ffi;

#[cfg(target_os = "macos")]
pub mod apps;
#[cfg(target_os = "macos")]
pub mod ax;
#[cfg(target_os = "macos")]
pub mod borders;
#[cfg(target_os = "macos")]
pub mod desktop;
#[cfg(target_os = "macos")]
pub mod displays;
#[cfg(target_os = "macos")]
pub mod keys;
#[cfg(target_os = "macos")]
pub mod observer;
#[cfg(target_os = "macos")]
pub mod runtime;
#[cfg(target_os = "macos")]
pub mod stackbars;
#[cfg(target_os = "macos")]
pub mod window_list;

/// What this backend is and can do (`gevrek_host::backend_info`).
pub fn backend_info() -> gevrek_host::backend_info::BackendInfo {
    use gevrek_host::backend_info::feature;
    gevrek_host::backend_info::BackendInfo::new("macos", "accessibility", &[feature::WINDOW_EVENTS])
}
