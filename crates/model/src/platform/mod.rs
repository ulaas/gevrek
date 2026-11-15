//! Backend-specific settings: `desktop.platform.windows` holds what only
//! the Windows backend can apply, `desktop.platform.wayland` what only the
//! Wayland backend can. A backend ignores the other one's section, so one
//! configuration file works on both systems.

pub mod wayland;
pub mod windows;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PlatformConfig {
    // (`self::`: the check for the `windows` crate in this crate, in CI
    // and scripts/check-linux.ps1, reads a bare `windows::` as that crate.)
    pub windows: self::windows::WindowsPlatformConfig,
    pub wayland: wayland::WaylandPlatformConfig,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}
