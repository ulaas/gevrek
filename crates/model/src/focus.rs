//! Focus behaviour settings.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Configuration for focus management behavior
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct FocusConfig {
    /// Maximum number of windows to track in history
    pub history_size: usize,
    /// Whether focus follows mouse cursor
    pub focus_follows_mouse: bool,
    /// Whether the mouse cursor is warped to the center of a window when
    /// focus changes through a dispatch.
    /// Focus changes caused by pointer movement (focus-follows-mouse) never
    /// warp the cursor.
    #[serde(default)]
    pub mouse_follows_focus: bool,
    /// Delay before focus follows mouse (ms)
    pub focus_delay_ms: u32,
    /// Whether to prevent focus stealing
    pub prevent_focus_steal: bool,
    /// Apps allowed to steal focus (by exe name)
    #[serde(serialize_with = "crate::ordered_set")]
    pub focus_steal_whitelist: HashSet<String>,
    /// When the focused window closes, the window used before it on the
    /// same workspace gets the focus. Off: the system chooses, which on
    /// Windows can be a window on another display. (A Wayland compositor
    /// has nobody else to choose: there the last used window that is shown
    /// always follows.)
    pub focus_next_on_close: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            history_size: 20,
            focus_follows_mouse: false,
            mouse_follows_focus: false,
            focus_delay_ms: 100,
            prevent_focus_steal: true,
            focus_steal_whitelist: HashSet::new(),
            focus_next_on_close: true,
        }
    }
}
