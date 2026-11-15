//! What a backend says about itself: the system it runs on and what it can
//! do. Each backend builds one; the IPC server hands it to clients
//! (`GetCapabilities`), which offer or grey out settings by it. Nothing
//! else decides what a system can do, least of all the system the server
//! happened to be compiled for.

/// Feature names a backend may report. A setting that only some backends
/// carry out names the feature it needs; a client connected to a backend
/// that does not report it shows the setting as not available there.
pub mod feature {
    /// gevrek's own bar is placed and its space kept free.
    pub const BAR: &str = "bar";
    /// Shadow, glow, gradient border and tint are drawn.
    pub const DRAWN_EFFECTS: &str = "drawn_effects";
    /// Tab strips of grouped windows are drawn.
    pub const STACKBARS: &str = "stackbars";
    /// Windows are put back on their workspaces after a restart.
    pub const SESSION_RESTORE: &str = "session_restore";
    /// Touchpad swipes run gesture profiles.
    pub const TOUCHPAD_GESTURES: &str = "touchpad_gestures";
    /// The scale of an output is set from the configuration.
    pub const OUTPUT_SCALE: &str = "output_scale";
    /// Programs of the other window system run (X11 under Wayland).
    pub const XWAYLAND: &str = "xwayland";
    /// Shell surfaces of any program are placed by the layer-shell protocol.
    pub const LAYER_SHELL: &str = "layer_shell";
    /// `window.*` events are sent to subscribers.
    pub const WINDOW_EVENTS: &str = "window_events";
}

/// A backend's own account of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInfo {
    /// The window system: "windows", "wayland".
    pub platform: String,
    /// The implementation: "win32", "smithay".
    pub backend: String,
    /// What it can do beyond what every backend can.
    pub features: Vec<String>,
}

impl BackendInfo {
    pub fn new(platform: &str, backend: &str, features: &[&str]) -> Self {
        Self {
            platform: platform.to_string(),
            backend: backend.to_string(),
            features: features.iter().map(|feature| feature.to_string()).collect(),
        }
    }

    /// For a server that runs without a real backend (tests): nothing
    /// beyond what every backend can do.
    pub fn none() -> Self {
        Self::new("none", "none", &[])
    }
}
