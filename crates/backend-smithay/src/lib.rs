//! The Wayland backend of gevrek: a Smithay compositor whose desktop
//! implements the engine's `Desktop` contract and whose state feeds the
//! core (`WmCore`) the window events of its clients.
//!
//! Linux only; on other systems the crate is empty.
#![cfg(target_os = "linux")]

pub mod background_effect;
pub mod bar;
pub mod border;
pub mod capture;
pub mod clip;
pub mod cursor;
pub mod desktop;
pub mod devices;
pub mod dmabuf;
pub mod dnd;
pub mod gestures;
pub mod headless;
pub mod host;
pub mod host_keys;
mod image_capture;
pub mod inject;
pub mod input;
pub mod input_method;
pub mod keys;
pub mod launch;
pub mod lock;
pub mod more_protocols;
pub mod notices;
mod output_management;
pub mod output_trial;
pub mod overview;
pub mod pick;
pub mod protocols;
pub mod render;
mod session;
pub mod session_env;
pub mod sharing;
pub mod soft;
pub mod stackbar;
pub mod state;
pub mod surface;
pub mod toplevel_icon;
mod toplevel_management;
pub mod udev;
mod virtual_pointer;
pub mod wallpaper;
pub mod window_overview;
pub mod winit;
mod workspace_management;
pub mod xdnd;
pub mod xwayland;

pub use desktop::WlDesktop;
pub use state::{ClientState, State};

/// What this backend is and can do (`gevrek_host::backend_info`).
pub fn backend_info() -> gevrek_host::backend_info::BackendInfo {
    use gevrek_host::backend_info::feature;
    gevrek_host::backend_info::BackendInfo::new(
        "wayland",
        "smithay",
        &[
            // The compositor draws borders itself and moves windows only
            // in what it draws.
            "server_side_borders",
            "render_time_animation",
            feature::BAR,
            feature::DRAWN_EFFECTS,
            feature::STACKBARS,
            feature::TOUCHPAD_GESTURES,
            feature::OUTPUT_SCALE,
            feature::XWAYLAND,
            feature::LAYER_SHELL,
            feature::WINDOW_EVENTS,
        ],
    )
}
