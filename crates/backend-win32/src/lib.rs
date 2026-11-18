//! gevrek-win32: the Windows backend (docs/PHASE2_PLAN.md, step 6).
//!
//! Everything that talks to the Windows API: window operations, borders and
//! stackbars (`dwm::WindowManager`), the WinEvent source and display watcher
//! (`dwm::WindowMonitor`), monitor enumeration, the hidden-window journal,
//! keyboard/mouse/touchpad input (`hotkey`), effect application (`apply`),
//! saved state (`persistence`), and host services (`host`: the process
//! guard, startup processes, the configuration file store and watcher).
//!
//! `runtime` is the WM host: the event loop, the executor (automation graph
//! and command dispatch), and hot reload. It drives the engine modules of
//! `gevrek-engine` with Windows state; it shrinks to a thin host as the
//! engine takes ownership of that state.

pub mod apply;
pub mod conflicts;
pub mod display_scale;
pub mod display_settings;
pub mod dwm;
pub mod elevated;
pub mod elevated_helper;
/// (What spoils what is read from outside: the engine's, taken in as it is.)
#[cfg(test)]
#[path = "../../engine/src/spoil.rs"]
mod spoil;
pub mod hotkey;
pub mod idle;
pub mod night_light;
pub mod persistence;
pub mod pick;
pub mod runtime;
pub mod system_look;
pub mod taskbar;
pub mod wallpaper;
pub mod wallpaper_colour;
pub mod window_kind;
pub mod windows_key;

/// Services of the WM host process.
pub mod host {
    // Shared with the Wayland host; they live in `gevrek-host`.
    pub use gevrek_host::{config_store, config_watch, process_manager};
    pub mod process_guard;
}

pub use gevrek_model::Config;

pub use runtime::Runtime;

// The moved code was written against these paths in `gevrek-core`.
mod settings;
mod config {
    pub use gevrek_model::config::*;
}
mod theme {}
mod node_editor {
    pub use gevrek_model::graph::*;
}

/// What this backend is and can do (`gevrek_host::backend_info`).
pub fn backend_info() -> gevrek_host::backend_info::BackendInfo {
    use gevrek_host::backend_info::feature;
    gevrek_host::backend_info::BackendInfo::new(
        "windows",
        "win32",
        &[
            "precision_touchpad_gestures_v12",
            "native_backdrop",
            gevrek_model::graph::FEATURE_WINDOWS_WINDOW_EFFECTS,
            feature::BAR,
            feature::DRAWN_EFFECTS,
            feature::STACKBARS,
            feature::SESSION_RESTORE,
            feature::TOUCHPAD_GESTURES,
            feature::WINDOW_EVENTS,
        ],
    )
}
