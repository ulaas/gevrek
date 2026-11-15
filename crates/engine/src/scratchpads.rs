//! Scratchpads: named windows that are toggled in and out of view over the
//! workspace (docs/PHASE2_PLAN.md, step 4.6).
//!
//! `Scratchpads` keeps the profiles, the window bound to each scratchpad and
//! whether it shows, and the scratchpads waiting for a window they launched.
//! It decides; the backend matches windows, launches programs, and shows,
//! hides and places the windows. A visibility change is recorded with
//! `set_visible` once the backend has carried it out.

use crate::outputs::Outputs;
use gevrek_model::geometry::Rect;
use gevrek_model::ids::{OutputId, WindowId};
use gevrek_model::settings::{MonitorSelector, ScratchpadPosition, ScratchpadProfile};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScratchpadError {
    /// No profile with this id (as the caller wrote it).
    NoProfile(String),
    Disabled(String),
    NoWindowAndNoLaunch(String),
}

impl std::fmt::Display for ScratchpadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProfile(id) => write!(f, "No scratchpad profile named '{id}' exists"),
            Self::Disabled(id) => write!(f, "Scratchpad '{id}' is disabled"),
            Self::NoWindowAndNoLaunch(id) => write!(
                f,
                "No matching window exists for scratchpad '{id}' and no launch command is configured"
            ),
        }
    }
}

impl std::error::Error for ScratchpadError {}

/// What a scratchpad looks like to clients (state queries).
#[derive(Debug, Clone, PartialEq)]
pub struct ScratchpadRuntimeState {
    pub id: String,
    pub enabled: bool,
    pub handle: Option<WindowId>,
    pub visible: bool,
    pub pending_spawn: bool,
}

/// What toggling a scratchpad asks the backend to do.
#[derive(Debug, Clone)]
pub enum Toggle {
    /// Its launched window has not appeared yet; nothing to do.
    Waiting,
    /// Hide its window, then `set_visible(id, false)`.
    Hide(WindowId),
    /// Place and show its window and focus it, then `set_visible(id, true)`.
    Show {
        window: WindowId,
        profile: Box<ScratchpadProfile>,
    },
    /// It has no window: adopt a matching window that is not in `assigned`
    /// (`bind`), or launch its program (`mark_pending`).
    Find {
        profile: Box<ScratchpadProfile>,
        assigned: HashSet<WindowId>,
    },
}

/// Scratchpad ids are case-insensitive.
pub fn normalize_id(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

#[derive(Debug, Default)]
pub struct Scratchpads {
    /// By normalized id.
    profiles: HashMap<String, ScratchpadProfile>,
    /// The window bound to a scratchpad, and whether it shows.
    windows: HashMap<String, (WindowId, bool)>,
    /// Scratchpads waiting for the window of the program they launched.
    pending: HashSet<String>,
}

impl Scratchpads {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- profiles ----

    /// Replace the profiles. Windows and pending launches of profiles that
    /// are gone are dropped.
    pub fn set_profiles(&mut self, profiles: Vec<ScratchpadProfile>) {
        self.profiles = profiles
            .into_iter()
            .map(|profile| (normalize_id(&profile.id), profile))
            .collect();
        self.pending.retain(|id| self.profiles.contains_key(id));
        self.windows.retain(|id, _| self.profiles.contains_key(id));
    }

    /// Add or replace one profile (automation graph nodes define their own).
    pub fn insert_profile(&mut self, profile: ScratchpadProfile) -> String {
        let id = normalize_id(&profile.id);
        self.profiles.insert(id.clone(), profile);
        id
    }

    pub fn profile(&self, id: &str) -> Option<&ScratchpadProfile> {
        self.profiles.get(&normalize_id(id))
    }

    pub fn runtime_states(&self) -> Vec<ScratchpadRuntimeState> {
        let mut states: Vec<ScratchpadRuntimeState> = self
            .profiles
            .iter()
            .map(|(id, profile)| {
                let bound = self.windows.get(id).copied();
                ScratchpadRuntimeState {
                    id: profile.id.clone(),
                    enabled: profile.enabled,
                    handle: bound.map(|(window, _)| window),
                    visible: bound.is_some_and(|(_, visible)| visible),
                    pending_spawn: self.pending.contains(id),
                }
            })
            .collect();
        states.sort_by_key(|state| state.id.to_lowercase());
        states
    }

    // ---- windows ----

    /// The window bound to a scratchpad and whether it shows.
    pub fn window(&self, id: &str) -> Option<(WindowId, bool)> {
        self.windows.get(&normalize_id(id)).copied()
    }

    pub fn is_scratchpad_window(&self, window: WindowId) -> bool {
        self.windows.values().any(|(bound, _)| *bound == window)
    }

    /// Bind a window to a scratchpad (it is no longer waiting for one).
    pub fn bind(&mut self, id: &str, window: WindowId, visible: bool) {
        let id = normalize_id(id);
        self.pending.remove(&id);
        self.windows.insert(id, (window, visible));
    }

    /// Record that the bound window is now shown or hidden.
    pub fn set_visible(&mut self, id: &str, visible: bool) {
        if let Some(entry) = self.windows.get_mut(&normalize_id(id)) {
            entry.1 = visible;
        }
    }

    pub fn unbind(&mut self, id: &str) {
        self.windows.remove(&normalize_id(id));
    }

    /// A window was destroyed.
    pub fn forget_window(&mut self, window: WindowId) {
        self.windows.retain(|_, (bound, _)| *bound != window);
    }

    /// Drop bindings to windows that no longer exist.
    pub fn retain_existing(&mut self, exists: impl Fn(WindowId) -> bool) {
        self.windows.retain(|_, (window, _)| exists(*window));
    }

    /// Every binding: id, window, visible.
    pub fn bindings(&self) -> Vec<(String, WindowId, bool)> {
        let mut bindings: Vec<_> = self
            .windows
            .iter()
            .map(|(id, (window, visible))| (id.clone(), *window, *visible))
            .collect();
        bindings.sort();
        bindings
    }

    /// The windows of hidden scratchpads.
    pub fn hidden_windows(&self) -> Vec<WindowId> {
        self.windows
            .values()
            .filter_map(|(window, visible)| (!*visible).then_some(*window))
            .collect()
    }

    /// Every scratchpad window was shown again (restore-windows, shutdown).
    pub fn mark_all_visible(&mut self) {
        for (_, visible) in self.windows.values_mut() {
            *visible = true;
        }
    }

    /// Hide every scratchpad: marks them hidden and returns the windows that
    /// were showing.
    pub fn hide_all(&mut self) -> Vec<WindowId> {
        let mut showing = Vec::new();
        for (window, visible) in self.windows.values_mut() {
            if *visible {
                showing.push(*window);
            }
            *visible = false;
        }
        showing
    }

    /// Focus moved to `focused`: the visible scratchpads with
    /// `hide_on_focus_loss` (other than the focused window itself) hide.
    /// Marks them hidden and returns their windows.
    pub fn hide_on_focus_loss(&mut self, focused: WindowId) -> Vec<WindowId> {
        let mut hide = Vec::new();
        for (id, (window, visible)) in self.windows.iter_mut() {
            let wants = self
                .profiles
                .get(id)
                .is_some_and(|profile| profile.hide_on_focus_loss);
            if *visible && *window != focused && wants {
                *visible = false;
                hide.push(*window);
            }
        }
        hide
    }

    /// The profiles changed: unbind the windows of scratchpads that are gone
    /// or disabled, and return them so they can be shown and managed again.
    pub fn release_removed(&mut self, profiles: &[ScratchpadProfile]) -> Vec<WindowId> {
        let retained: HashSet<String> = profiles
            .iter()
            .filter(|profile| profile.enabled)
            .map(|profile| normalize_id(&profile.id))
            .collect();
        let removed: Vec<(String, WindowId)> = self
            .windows
            .iter()
            .filter(|(id, _)| !retained.contains(*id))
            .map(|(id, (window, _))| (id.clone(), *window))
            .collect();
        removed
            .into_iter()
            .map(|(id, window)| {
                self.windows.remove(&id);
                window
            })
            .collect()
    }

    // ---- launches ----

    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.contains(&normalize_id(id))
    }

    /// The scratchpad launched its program and waits for the window.
    pub fn mark_pending(&mut self, id: &str) {
        self.pending.insert(normalize_id(id));
    }

    /// The enabled scratchpads waiting for a window, with their profiles
    /// (ids whose profile is gone stop waiting).
    pub fn pending_profiles(&mut self) -> Vec<(String, ScratchpadProfile)> {
        self.pending.retain(|id| self.profiles.contains_key(id));
        let mut waiting: Vec<(String, ScratchpadProfile)> = self
            .pending
            .iter()
            .filter_map(|id| Some((id.clone(), self.profiles.get(id)?.clone())))
            .filter(|(_, profile)| profile.enabled)
            .collect();
        waiting.sort_by(|left, right| left.0.cmp(&right.0));
        waiting
    }

    // ---- toggling ----

    /// Decide what toggling scratchpad `id` means. `exists` tells whether a
    /// window is still alive; a binding to a dead window is dropped.
    pub fn toggle(
        &mut self,
        id: &str,
        exists: impl Fn(WindowId) -> bool,
    ) -> Result<Toggle, ScratchpadError> {
        let normalized = normalize_id(id);
        let profile = self
            .profiles
            .get(&normalized)
            .cloned()
            .ok_or_else(|| ScratchpadError::NoProfile(id.to_string()))?;
        if !profile.enabled {
            return Err(ScratchpadError::Disabled(profile.id));
        }
        if self.pending.contains(&normalized) {
            return Ok(Toggle::Waiting);
        }
        if let Some((window, visible)) = self.windows.get(&normalized).copied() {
            if !exists(window) {
                self.windows.remove(&normalized);
            } else if visible {
                return Ok(Toggle::Hide(window));
            } else {
                return Ok(Toggle::Show {
                    window,
                    profile: Box::new(profile),
                });
            }
        }
        Ok(Toggle::Find {
            assigned: self.windows.values().map(|(window, _)| *window).collect(),
            profile: Box::new(profile),
        })
    }
}

/// The output a scratchpad opens on: the one its profile names, else the
/// active output, else the primary one.
pub fn resolve_output(
    outputs: &Outputs,
    selector: &MonitorSelector,
    active: Option<OutputId>,
) -> Option<OutputId> {
    match selector {
        MonitorSelector::Any => active,
        MonitorSelector::Primary => outputs.get_primary_monitor_handle(),
        MonitorSelector::Index(index) => outputs
            .get_all_monitors_ordered()
            .get(*index)
            .map(|monitor| monitor.handle),
        MonitorSelector::Name(name) => outputs
            .get_all_monitors_ordered()
            .into_iter()
            .find(|monitor| monitor.is_called(name))
            .map(|monitor| monitor.handle),
    }
    .or(active)
    .or_else(|| outputs.get_primary_monitor_handle())
}

/// Where a scratchpad window goes in a work area: its size is a share of the
/// area, its position one of the anchors.
pub fn placement(profile: &ScratchpadProfile, area: Rect) -> Rect {
    let w = (area.width as f64 * profile.width_percent.clamp(1, 100) as f64 / 100.0) as i32;
    let h = (area.height as f64 * profile.height_percent.clamp(1, 100) as f64 / 100.0) as i32;
    let (x, y) = match profile.position {
        ScratchpadPosition::Top => (area.x + (area.width - w) / 2, area.y),
        ScratchpadPosition::Bottom => (area.x + (area.width - w) / 2, area.y + area.height - h),
        ScratchpadPosition::Center => (
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
        ),
        ScratchpadPosition::TopLeft => (area.x, area.y),
        ScratchpadPosition::TopRight => (area.x + area.width - w, area.y),
        ScratchpadPosition::BottomLeft => (area.x, area.y + area.height - h),
        ScratchpadPosition::BottomRight => (area.x + area.width - w, area.y + area.height - h),
    };
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERM: WindowId = WindowId(1);
    const OTHER: WindowId = WindowId(2);

    fn profile(id: &str) -> ScratchpadProfile {
        ScratchpadProfile {
            id: id.to_string(),
            enabled: true,
            ..ScratchpadProfile::default()
        }
    }

    fn pads() -> Scratchpads {
        let mut pads = Scratchpads::new();
        pads.set_profiles(vec![profile("Term"), profile("notes")]);
        pads
    }

    #[test]
    fn a_toggle_finds_shows_and_hides_in_turn() {
        let mut pads = pads();
        pads.bind("notes", OTHER, false);

        // No window yet: look for one that is not another scratchpad's.
        let Toggle::Find { assigned, .. } = pads.toggle(" TERM ", |_| true).unwrap() else {
            panic!("expected Find");
        };
        assert!(assigned.contains(&OTHER));

        pads.bind("term", TERM, true);
        assert!(matches!(
            pads.toggle("term", |_| true),
            Ok(Toggle::Hide(TERM))
        ));
        pads.set_visible("term", false);
        assert!(matches!(
            pads.toggle("term", |_| true),
            Ok(Toggle::Show { window: TERM, .. })
        ));
    }

    #[test]
    fn a_dead_window_is_unbound_and_a_pending_launch_waits() {
        let mut pads = pads();
        pads.bind("term", TERM, true);
        assert!(matches!(
            pads.toggle("term", |_| false),
            Ok(Toggle::Find { .. })
        ));
        assert_eq!(pads.window("term"), None);

        pads.mark_pending("term");
        assert!(matches!(pads.toggle("term", |_| true), Ok(Toggle::Waiting)));
        assert_eq!(pads.pending_profiles().len(), 1);
        pads.bind("term", TERM, true);
        assert!(!pads.is_pending("term"), "the launched window arrived");
    }

    #[test]
    fn unknown_and_disabled_scratchpads_are_refused() {
        let mut pads = pads();
        assert_eq!(
            pads.toggle("Nope", |_| true).unwrap_err().to_string(),
            "No scratchpad profile named 'Nope' exists"
        );
        let mut disabled = profile("off");
        disabled.enabled = false;
        pads.insert_profile(disabled);
        assert_eq!(
            pads.toggle("off", |_| true).unwrap_err().to_string(),
            "Scratchpad 'off' is disabled"
        );
    }

    #[test]
    fn focus_loss_hides_only_scratchpads_that_ask_for_it() {
        let mut pads = Scratchpads::new();
        let mut drop_down = profile("term");
        drop_down.hide_on_focus_loss = true;
        pads.set_profiles(vec![drop_down, profile("notes")]);
        pads.bind("term", TERM, true);
        pads.bind("notes", OTHER, true);

        assert!(
            pads.hide_on_focus_loss(TERM).is_empty(),
            "it has focus itself"
        );
        assert_eq!(pads.hide_on_focus_loss(WindowId(9)), vec![TERM]);
        assert_eq!(pads.window("term"), Some((TERM, false)));
        assert_eq!(pads.window("notes"), Some((OTHER, true)));
        assert_eq!(pads.hidden_windows(), vec![TERM]);
    }

    #[test]
    fn removed_profiles_release_their_windows() {
        let mut pads = pads();
        pads.bind("term", TERM, false);
        pads.bind("notes", OTHER, true);
        assert_eq!(pads.release_removed(&[profile("notes")]), vec![TERM]);
        assert!(!pads.is_scratchpad_window(TERM));

        pads.set_profiles(vec![]);
        assert!(pads.bindings().is_empty());
        assert!(pads.runtime_states().is_empty());
    }

    #[test]
    fn states_report_binding_visibility_and_pending_launches() {
        let mut pads = pads();
        pads.bind("term", TERM, true);
        pads.mark_pending("notes");
        let states = pads.runtime_states();
        assert_eq!(states[0].id, "notes");
        assert!(states[0].pending_spawn && states[0].handle.is_none());
        assert_eq!((states[1].handle, states[1].visible), (Some(TERM), true));

        assert_eq!(pads.hide_all(), vec![TERM]);
        pads.mark_all_visible();
        assert_eq!(pads.window("term"), Some((TERM, true)));
    }

    #[test]
    fn placement_sizes_by_percentage_and_anchors() {
        let area = Rect::new(100, 50, 1000, 800);
        let mut p = profile("term");
        p.width_percent = 50;
        p.height_percent = 25;
        p.position = ScratchpadPosition::Top;
        assert_eq!(placement(&p, area), Rect::new(350, 50, 500, 200));
        p.position = ScratchpadPosition::BottomRight;
        assert_eq!(placement(&p, area), Rect::new(600, 650, 500, 200));
        p.position = ScratchpadPosition::Center;
        assert_eq!(placement(&p, area), Rect::new(350, 350, 500, 200));
    }
}
