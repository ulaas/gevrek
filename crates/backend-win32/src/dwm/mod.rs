// Backend-neutral modules now live in gevrek-engine; re-exported here so
// existing `crate::dwm::...` paths keep working (Phase 2, decision D1).
pub use gevrek_engine::{
    animation_scheduler, animations, container, focus_manager, layouts, ring, rules, workspace,
};
mod aura;
pub mod hidden_journal;
pub mod hwnd_map;
pub mod monitor;
mod overview;
pub mod reach;
mod stand_in;
pub mod veil;
use gevrek_engine::overlay_data;
pub(crate) mod window_manager;
mod window_monitor;
pub use gevrek_engine::snap_zones;

pub use animation_scheduler::{
    AnimationFrame as RuntimeAnimationFrame, BorderFrame as RuntimeBorderFrame,
    RuntimeAnimationScheduler, TransitionScope,
};
pub use animations::AnimationType as DwmAnimationType;
pub use container::{Container, Padding, Rect};
pub use focus_manager::{FocusConfig, FocusManager};
pub use monitor::{
    enumerate_monitors, HmonitorExt, MonitorChanges, MonitorHandle, MonitorInfo, MonitorManager,
    MonitorManagerWin32, StaleWindow, WorkspaceSettings,
};
pub use overlay_data::{
    DetailedWindowInfo, GestureProgressData, GestureStateData, MonitorData, OverlayData,
    PerformanceMetrics, RuleStateData, ScratchpadStateData, StartupStateData, WorkspaceData,
};
pub use rules::{
    FloatRule, IgnoreRule, ManageRule, MatchKind, MatchStrategy, RuleEngine, RulePosition,
    RuleResolution, WindowRule,
};
pub use snap_zones::{
    presets as snap_zone_presets, SnapZone, SnapZoneConfig, SnapZoneHit, SnapZoneLayout,
    SnapZoneManager, SnapZoneModifier,
};
pub use window_manager::{
    invisible_border_content_rect, stackbar_content_rect, HwndExt, IgnoreReason,
    RuleResolutionScope, StackbarContainerState, SystemBackdrop, WindowCorner, WindowHandle,
    WindowInfo, WindowManager,
};
pub use window_monitor::{WindowEvent, WindowMonitor};
pub use workspace::{ExposeState, WindowContainerBehaviour, Workspace};
