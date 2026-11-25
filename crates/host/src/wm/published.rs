//! The state the clients read: the outputs and their workspaces, the
//! windows, the live submap, the rules, the startup programs, what there is
//! to tell the person at the machine.
//!
//! It is published into the slot the server hands out (`GetOverlayData`,
//! `QueryState`), now and then and whenever something was told or done
//! (`HostState::state_stale`). Subscribers hear what changed of the outputs
//! (`overlay_data::changes`) and of the notices as it is published.

use super::WmHost;
use gevrek_engine::executor::RuntimeEventKind;
use gevrek_engine::overlay_data::{
    DisplayFrames, GestureProgressData, KeyboardLayoutNow, Notice, OverlayData, RuleStateData,
    StartupStateData,
};

/// What only the system knows of the published state.
#[derive(Debug, Clone, Default)]
pub struct SystemState {
    /// Passes of the loop a second.
    pub event_loop_fps: f64,
    pub active_animations: usize,
    /// The touchpad: whether swipes are delivered, by what, from how many
    /// devices, and the swipe going on.
    pub gestures_available: bool,
    pub gesture_backend: String,
    pub gesture_devices: u32,
    pub gesture: Option<GestureProgressData>,
    pub gesture_error: Option<String>,
    /// What there is to tell the person at the machine.
    pub notices: Vec<Notice>,
    pub keyboard_layout: Option<KeyboardLayoutNow>,
    /// The frames each display showed (where gevrek draws them).
    pub frames: Vec<DisplayFrames>,
}

/// The host's publishing, for every backend.
pub trait Published: WmHost {
    /// Publish the state the clients read, now; `last_event` says why.
    fn publish_state(&mut self, last_event: Option<String>) {
        self.host().state_stale.set(false);
        if self.host().overlay.is_none() {
            return;
        }
        gevrek_engine::counts::count(gevrek_engine::counts::Counted::StatePublished);
        let (mut monitors, total_windows) = {
            let parts = self.parts();
            parts
                .core
                .monitor_data(parts.desktop, &parts.config.desktop)
        };
        self.adjust_published_monitors(&mut monitors);
        let changes =
            gevrek_engine::overlay_data::changes(&self.host().published_monitors, &monitors);
        for (kind, fields) in changes {
            self.host().emit(kind, None, fields);
        }
        // (What was just told is in what is published now: not a reason to
        // publish again.)
        self.host().state_stale.set(false);
        let system = self.system_state();
        let rule_states = self
            .rules()
            .lock()
            .map(|engine| {
                engine
                    .rule_states()
                    .into_iter()
                    .map(|(name, enabled, dynamic)| RuleStateData {
                        name,
                        enabled,
                        dynamic,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let parts = self.parts();
        parts.host.published_monitors = monitors.clone();
        parts.host.last_published = std::time::Instant::now();
        let gestures = &parts.config.desktop.gestures;
        let data = OverlayData {
            timestamp: std::time::SystemTime::now(),
            monitors,
            graph_node_count: parts.config.node_graph.nodes.len(),
            total_windows,
            event_loop_fps: system.event_loop_fps,
            last_event,
            active_animations: system.active_animations,
            active_binding_submap: parts.host.active_submap.clone(),
            tiling_suspended: parts.core.executor.tiling_suspended(),
            desktop_shown: parts.core.executor.desktop_shown(),
            gesture: gevrek_engine::overlay_data::GestureStateData {
                available: system.gestures_available,
                backend: system.gesture_backend,
                device_count: system.gesture_devices,
                configured_count: gestures
                    .profiles
                    .iter()
                    .filter(|profile| gestures.enabled && profile.enabled)
                    .count() as u32,
                active: system.gesture,
                last_error: system.gesture_error,
            },
            rule_states,
            scratchpad_states: parts.core.scratchpad_data(),
            startup_states: parts
                .host
                .startup
                .states()
                .into_iter()
                .map(|state| StartupStateData {
                    id: state.id,
                    name: state.name,
                    enabled: state.enabled,
                    status: state.status,
                    pid: state.pid,
                    restart_count: state.restart_count,
                    last_exit_code: state.last_exit_code,
                    last_error: state.last_error,
                })
                .collect(),
            metrics: None,
            notices: system.notices,
            accent: parts.core.wallpaper_accent,
            keyboard_layout: system.keyboard_layout,
            frames: system.frames,
        };
        // (The whole of each: a notice whose text changes is news too.)
        let notices = data.notices.clone();
        if let Some(overlay) = &parts.host.overlay {
            *overlay
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(data);
        }
        // Whoever shows the notices (the panel) hears that they are others
        // now and asks for the state, instead of at its next question.
        if notices != parts.host.notices_told {
            let count = notices.len();
            parts.host.notices_told = notices;
            parts.host.emit(
                RuntimeEventKind::NoticesChanged,
                None,
                [("count".to_string(), count.to_string())],
            );
        }
    }
}

impl<T: WmHost + ?Sized> Published for T {}
