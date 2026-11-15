//! What a backend reports about a window when it appears or changes.

use crate::ids::WindowId;
use serde::{Deserialize, Serialize};

/// Properties of a window, as the backend sees them. Rules match on these.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WindowProps {
    /// Wayland `app_id`; on Windows the executable stem.
    pub app_id: String,
    pub title: String,
    /// Win32 window class / X11 `WM_CLASS`.
    pub class: Option<String>,
    /// Full path of the executable, when known.
    pub executable: Option<String>,
    pub pid: Option<u32>,
    pub kind: WindowKind,
    /// The owner (Win32) or transient-for parent (Wayland/X11).
    pub parent: Option<WindowId>,
    pub size_hints: SizeHints,
}

/// A window as the WM lists it: identity, title, geometry and state.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub handle: WindowId,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub is_visible: bool,
    pub is_minimized: bool,
    pub is_maximized: bool,
}

/// The role of a window, which decides its default policy (tile or float).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum WindowKind {
    #[default]
    Normal,
    Dialog,
    Utility,
    Popup,
    Splash,
}

/// Size limits the application asked for. A window with equal minimum and
/// maximum is fixed-size.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct SizeHints {
    pub min_width: Option<i32>,
    pub min_height: Option<i32>,
    pub max_width: Option<i32>,
    pub max_height: Option<i32>,
}

impl SizeHints {
    pub fn is_fixed_size(&self) -> bool {
        self.min_width.is_some()
            && self.min_width == self.max_width
            && self.min_height.is_some()
            && self.min_height == self.max_height
    }
}

/// What a backend can do. Reported to the editor and CLI, and checked by the
/// engine before it emits an effect a backend cannot carry out.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct Capabilities {
    /// The backend interpolates `Effect::Animate` itself (a compositor).
    /// Otherwise the engine emits one `Configure` per animation frame.
    pub render_time_animation: bool,
    /// Windows-only effects (`PlatformEffect::Windows`) are applied.
    pub windows_platform_effects: bool,
    /// The backend can move the pointer.
    pub warp_pointer: bool,
    /// The backend draws stackbars for tabbed containers.
    pub stackbars: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_min_and_max_is_fixed_size() {
        let fixed = SizeHints {
            min_width: Some(400),
            min_height: Some(300),
            max_width: Some(400),
            max_height: Some(300),
        };
        assert!(fixed.is_fixed_size());
        assert!(!SizeHints::default().is_fixed_size());
    }
}
