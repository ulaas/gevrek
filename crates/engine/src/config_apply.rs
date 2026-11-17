//! Applying the configuration to the WM's state: the rule engine built from
//! it, monitor profiles, zones, the appearance of workspaces, and workspace
//! profiles. A host calls these at start, on a configuration reload and
//! when the set of monitors changes.

use crate::core::WmCore;
use crate::outputs::{MonitorHandle, MonitorInfo, Outputs};
use crate::rules::{FloatRule, MatchKind, MatchStrategy, RuleEngine, WindowRule};
use crate::workspace_profile::WorkspaceProfileApply;
use crate::zones::ZoneManager;
use gevrek_model::settings::{
    AppearanceProfile, DesktopConfig, MonitorSelector, WorkspaceProfile, ZoneSettings,
};
use gevrek_model::zones::ZoneModifier;

/// The rule engine for a configuration: the built-in float and system
/// rules, the configured window rules, and Gevrek's own windows floating.
pub fn build_rule_engine(config: &DesktopConfig) -> RuleEngine {
    let mut engine = RuleEngine::new();
    engine.add_common_float_rules();
    if config.apply_system_rules {
        engine.add_system_ignore_rules();
    }
    engine.set_declarative_rules(config.window_rules.clone());
    engine.add_float_rule(
        FloatRule::new(WindowRule::new(
            "Gevrek".to_string(),
            MatchKind::Title,
            MatchStrategy::Contains,
        ))
        .with_comment("Gevrek editor window should not be tiled".to_string()),
    );
    engine
}

/// The monitors a selector names, out of the attached ones in ring order.
pub fn monitor_selector_targets(
    selector: &MonitorSelector,
    monitors: &[MonitorInfo],
) -> Vec<MonitorHandle> {
    match selector {
        MonitorSelector::Primary => monitors
            .iter()
            .find(|monitor| monitor.is_primary)
            .map(|monitor| vec![monitor.handle])
            .unwrap_or_default(),
        MonitorSelector::Index(index) => monitors
            .get(*index)
            .map(|monitor| vec![monitor.handle])
            .unwrap_or_default(),
        MonitorSelector::Name(name) => monitors
            .iter()
            .filter(|monitor| monitor.is_called(name))
            .map(|monitor| monitor.handle)
            .collect(),
        MonitorSelector::Any => monitors.iter().map(|monitor| monitor.handle).collect(),
    }
}

/// Apply the monitor profiles: reserved areas, the global work-area offset
/// and the preferred order of the monitors.
pub fn configure_monitor_profiles(outputs: &mut Outputs, config: &DesktopConfig) {
    outputs.clear_reserved_areas();
    outputs.set_global_work_area_offset(config.work_area_offset);
    // Monitor index preferences: name-keyed index preferences reorder the monitor ring before any
    // index-based targeting below resolves.
    let index_preferences = config
        .monitors
        .iter()
        .filter(|profile| profile.enabled)
        .filter_map(
            |profile| match (&profile.monitor, profile.index_preference) {
                (MonitorSelector::Name(name), Some(index)) => {
                    Some((name.trim().to_string(), index))
                }
                _ => None,
            },
        )
        .collect::<Vec<_>>();
    outputs.set_monitor_index_preferences(index_preferences);
    let monitors = outputs.get_all_monitors_ordered();
    for profile in config.monitors.iter().filter(|profile| profile.enabled) {
        let targets = monitor_selector_targets(&profile.monitor, &monitors);
        if targets.is_empty() {
            log::warn!(
                "Monitor profile {:?} did not match an attached monitor",
                profile.monitor
            );
        }
        for monitor in targets {
            outputs.set_reserved_area(monitor, profile.reserved);
        }
    }
}

/// The zone manager for the zone settings and the attached monitors.
/// `modifier_probe` is the backend's way to read the activation key.
pub fn build_zone_manager(
    settings: &ZoneSettings,
    outputs: &Outputs,
    modifier_probe: fn(ZoneModifier) -> bool,
) -> ZoneManager {
    let mut manager = ZoneManager::new(settings.behavior.clone())
        .with_default_layout(settings.default_layout.clone())
        .with_modifier_probe(modifier_probe);
    let monitors = outputs.get_all_monitors_ordered();
    for profile in settings
        .monitor_layouts
        .iter()
        .filter(|profile| profile.enabled)
    {
        for monitor in monitor_selector_targets(&profile.monitor, &monitors) {
            manager.set_monitor_layout(monitor, profile.layout.clone());
        }
    }
    manager
}

/// A workspace that applying the profiles created.
#[derive(Debug, Clone)]
pub struct CreatedWorkspace {
    pub monitor: MonitorHandle,
    pub profile: WorkspaceProfile,
}

impl WmCore {
    /// Give every workspace the borders, stackbar and invisible-border
    /// settings of the appearance profile (with its theme applied).
    pub fn apply_workspace_appearance(&mut self, appearance: &AppearanceProfile) {
        // The theme (colours still at their built-in values come from
        // it; ones set by hand stay), the wallpaper's colour where the
        // theme asks for it, and high contrast last.
        let appearance = gevrek_model::theme::dressed(appearance, self.wallpaper_accent);
        // Workspaces that are made later are dressed the same.
        self.executor.wallpaper_accent = self.wallpaper_accent;
        for monitor in self.monitor_manager.get_all_monitors_ordered() {
            if let Some(workspaces) = self.monitor_manager.get_workspaces_mut(monitor.handle) {
                for workspace in workspaces.iter_mut() {
                    workspace.border_enabled = appearance.border_enabled;
                    workspace.border_width = appearance.border_width;
                    workspace.border_offset = appearance.border_offset;
                    workspace.border_style = appearance.border_style;
                    workspace.active_border_color = appearance.active_border_color;
                    workspace.inactive_border_color = appearance.inactive_border_color;
                    workspace.active_border_alpha = appearance.active_border_alpha;
                    workspace.inactive_border_alpha = appearance.inactive_border_alpha;
                    workspace.floating_look = appearance.floating.clone();
                    workspace.stackbar = appearance.stackbar.clone();
                    workspace.invisible_borders = appearance.invisible_borders;
                }
            }
        }
    }

    /// Apply the enabled workspace profiles that `include` selects:
    /// existing workspaces of that name get the profile's settings,
    /// persistent and default ones are created if missing. With
    /// `focus_default` a default profile's workspace becomes the visible
    /// one (on a display that had no workspace yet it does anyway).
    /// Afterwards each display's workspaces are as the configuration has
    /// them (`settle_workspaces`). Returns the workspaces that were
    /// created.
    pub fn apply_workspace_profile_set(
        &mut self,
        profiles: &[WorkspaceProfile],
        appearance: &AppearanceProfile,
        include: impl Fn(&WorkspaceProfile) -> bool,
        focus_default: bool,
    ) -> Vec<CreatedWorkspace> {
        // (As `apply_workspace_appearance` dresses it: a profile takes
        // the appearance's borders where it has none of its own.)
        let appearance = &gevrek_model::theme::dressed(appearance, self.wallpaper_accent);
        let mut created_workspaces = Vec::new();
        let monitors = self.monitor_manager.get_all_monitors_ordered();
        for profile in profiles.iter().filter(|profile| profile.enabled) {
            if monitor_selector_targets(&profile.monitor, &monitors).is_empty() {
                log::warn!(
                    "Workspace profile '{}' did not match an attached monitor",
                    profile.name
                );
            }
        }
        for monitor in monitors.iter().map(|info| info.handle) {
            // What this display has: for each workspace its own entry
            // before the one for every display.
            let there: Vec<WorkspaceProfile> =
                gevrek_model::settings::workspaces_where(profiles, |selector| {
                    monitor_selector_targets(selector, &monitors).contains(&monitor)
                })
                .into_iter()
                .cloned()
                .collect();
            // (A display that had no workspace yet, one that was just
            // attached, begins with the one it is to start with.)
            let new_display = self
                .monitor_manager
                .get_workspaces(monitor)
                .is_none_or(|workspaces| workspaces.is_empty());
            for profile in there.iter().filter(|profile| include(profile)) {
                let existing_index = self.monitor_manager.get_workspaces(monitor).and_then(
                    |workspaces| {
                        workspaces.iter().position(|workspace| {
                            workspace.name.trim().eq_ignore_ascii_case(profile.name.trim())
                        })
                    },
                );
                // Persistent/default profiles are eagerly materialized.
                // Other profiles remain declarative until a named selector
                // visits them.
                if existing_index.is_none() && !profile.persistent && !profile.default {
                    continue;
                }
                let mut created = false;
                let index = existing_index.or_else(|| {
                    created = true;
                    self.monitor_manager.create_workspace(
                        monitor,
                        profile.name.trim().to_string(),
                        profile.layout,
                    )
                });
                let Some(index) = index else {
                    continue;
                };
                let mut shown_as = None;
                if let Some(workspace) = self
                    .monitor_manager
                    .get_workspaces_mut(monitor)
                    .and_then(|workspaces| workspaces.get_mut(index))
                {
                    let before = workspace.label().to_string();
                    profile.apply_to_workspace(workspace, appearance);
                    if !created && before != workspace.label() {
                        shown_as = Some((workspace.id, before, workspace.label().to_string()));
                    }
                }
                // (Whoever shows the workspaces hears of another title
                // as of a renaming by hand.)
                if let Some((id, before, now)) = shown_as {
                    log::info!("Workspace '{before}' is shown as '{now}' on this display now");
                    self.executor.emit_workspace_renamed(monitor, id, &before, &now);
                }
                if profile.default && (focus_default || new_display) {
                    self.monitor_manager.focus_workspace_at(monitor, index);
                }
                if created {
                    created_workspaces.push(CreatedWorkspace {
                        monitor,
                        profile: profile.clone(),
                    });
                }
            }
            self.settle_workspaces(monitor, &there);
        }
        created_workspaces
    }

    /// After a display's entries were applied: its workspaces are as the
    /// configuration has them. One the configuration had for the display
    /// and has no more is a workspace of the moment from now on (it goes
    /// when it is empty and not shown; with windows it stays until it is
    /// left empty); and the workspaces are in the configuration's order,
    /// those it does not have after them.
    fn settle_workspaces(&mut self, monitor: MonitorHandle, there: &[WorkspaceProfile]) {
        let place = |name: &str| {
            there
                .iter()
                .position(|profile| profile.name.trim().eq_ignore_ascii_case(name.trim()))
        };
        let mut dropped = Vec::new();
        if let Some(workspaces) = self.monitor_manager.get_workspaces_mut(monitor) {
            for workspace in workspaces.iter_mut() {
                if workspace.configured && place(&workspace.name).is_none() {
                    workspace.configured = false;
                    workspace.persistent = false;
                    dropped.push(workspace.id);
                }
            }
        }
        self.monitor_manager
            .order_workspaces(monitor, |workspace| place(&workspace.name));
        for id in dropped {
            let Some((index, name)) =
                self.monitor_manager
                    .get_workspaces(monitor)
                    .and_then(|workspaces| {
                        workspaces
                            .iter()
                            .position(|workspace| workspace.id == id)
                            .and_then(|index| Some((index, workspaces.get(index)?.name.clone())))
                    })
            else {
                continue;
            };
            if self.executor.graph_creates_workspace(monitor, &name) {
                continue;
            }
            if self
                .monitor_manager
                .remove_inactive_empty_workspace(monitor, index)
            {
                log::info!("Workspace '{name}' is not in the configuration any more: it is gone");
                self.executor.emit_workspace_event(
                    crate::executor::RuntimeEventKind::WorkspaceDestroyed,
                    monitor,
                    &name,
                    false,
                );
            }
        }
    }
}

impl WmCore {
    /// Workspaces the previous graph created and the new one does not are
    /// removed when nothing else needs them. Returns what happened to each.
    /// After a new configuration's workspace profiles are in place: an
    /// output that still shows a workspace which no profile and no node of
    /// the graph names any more (the one the previous configuration made)
    /// hands its windows to the default workspace of the profiles and shows
    /// that one; the old workspace goes. Without this, a configuration with
    /// workspaces of its own would start with all of them empty and the
    /// windows left behind on a workspace it knows nothing about.
    ///
    /// Only the workspaces named in `leftovers` are handed over: those the
    /// caller knows the previous configuration made (a workspace the user
    /// opened by hand stays theirs, whatever the profiles say).
    ///
    /// Returns, per output it happened on, the old and the new name.
    pub fn hand_over_leftover_workspaces(
        &mut self,
        desktop: &mut dyn crate::contract::Desktop,
        profiles: &[WorkspaceProfile],
        leftovers: &[(crate::outputs::MonitorHandle, String)],
    ) -> Vec<(crate::outputs::MonitorHandle, String, String)> {
        let enabled: Vec<&WorkspaceProfile> =
            profiles.iter().filter(|profile| profile.enabled).collect();
        if enabled.is_empty() {
            return Vec::new();
        }
        let named = |name: &str| {
            enabled
                .iter()
                .any(|profile| profile.name.eq_ignore_ascii_case(name))
        };
        let mut handed_over = Vec::new();
        let monitors: Vec<crate::outputs::MonitorHandle> = self
            .monitor_manager
            .get_all_monitors_ordered()
            .iter()
            .map(|info| info.handle)
            .collect();
        for monitor in monitors {
            let Some(ring) = self.monitor_manager.get_workspaces(monitor) else {
                continue;
            };
            let leftover_index = ring.focused_idx();
            let Some(leftover) = ring.get(leftover_index) else {
                continue;
            };
            let leftover_name = leftover.name.clone();
            if !leftovers.iter().any(|(output, name)| {
                *output == monitor && name.eq_ignore_ascii_case(&leftover_name)
            }) || named(&leftover_name)
                || self
                    .executor
                    .graph_creates_workspace(monitor, &leftover_name)
            {
                continue;
            }
            // Where to: the default profile's workspace on this output, or
            // failing that the first workspace a profile names.
            let position = |wanted_default: bool| {
                ring.elements().iter().position(|workspace| {
                    enabled.iter().any(|profile| {
                        profile.name.eq_ignore_ascii_case(&workspace.name)
                            && (!wanted_default || profile.default)
                    })
                })
            };
            let Some(target_index) = position(true).or_else(|| position(false)) else {
                continue;
            };
            let windows = leftover.all_windows();
            let floating: Vec<bool> = windows
                .iter()
                .map(|window| self.monitor_manager.is_window_floating(*window))
                .collect();
            let Some(ring) = self.monitor_manager.get_workspaces_mut(monitor) else {
                continue;
            };
            if let Some(leftover) = ring.get_mut(leftover_index) {
                for window in &windows {
                    leftover.remove_window(*window);
                }
            }
            let Some(target) = ring.get_mut(target_index) else {
                continue;
            };
            let target_name = target.name.clone();
            for (window, floating) in windows.iter().zip(floating) {
                if floating {
                    target.add_floating_window(*window);
                } else {
                    target.add_window(*window, true);
                }
            }
            self.monitor_manager
                .focus_workspace_at(monitor, target_index);
            self.monitor_manager
                .remove_inactive_empty_workspace(monitor, leftover_index);
            // "The workspace before" was the one that is gone.
            self.monitor_manager.forget_previous_workspace(monitor);
            // Windows the target had while it was not shown are shown now.
            if let Err(error) =
                self.executor
                    .show_focused_workspace(desktop, &mut self.monitor_manager, monitor)
            {
                log::warn!("Could not show workspace '{target_name}': {error}");
            }
            log::info!(
                "Workspace '{}' is not part of the new configuration: its {} window(s) are on '{}' now",
                leftover_name,
                windows.len(),
                target_name
            );
            handed_over.push((monitor, leftover_name, target_name));
        }
        handed_over
    }

    pub fn remove_dropped_graph_workspaces(
        &mut self,
        dropped: Vec<(MonitorHandle, String)>,
    ) -> Vec<crate::reload::DroppedWorkspace> {
        let executor = &self.executor;
        crate::reload::remove_dropped_workspaces(&mut self.monitor_manager, dropped, |name| {
            executor.has_workspace_profile(name)
        })
    }

    /// Windows the rules now say to ignore leave their workspaces: their
    /// border goes, and a window hidden on another workspace is shown.
    pub fn release_newly_ignored_windows(&mut self, desktop: &mut dyn crate::contract::Desktop) {
        use crate::contract::{BorderChange, Effect};
        for id in crate::reload::managed_windows(&self.monitor_manager) {
            if !desktop.ignored_by_rules(id) {
                continue;
            }
            let Some((monitor, _)) = self.monitor_manager.find_and_remove_window(id) else {
                continue;
            };
            log::info!("Window {:?} is ignored by the new rules; releasing it", id);
            let _ = desktop.apply(&Effect::Border {
                id,
                change: BorderChange::Remove,
            });
            if desktop.is_hidden_by_backend(id) {
                let _ = desktop.apply(&Effect::SetVisible { id, visible: true });
            }
            self.executor.mark_layout_dirty(monitor);
        }
    }

    /// Recreate the borders of the workspaces whose border settings changed
    /// (all of them with `all`).
    pub fn refresh_changed_borders(
        &mut self,
        desktop: &mut dyn crate::contract::Desktop,
        before: &crate::reload::BorderSnapshot,
        all: bool,
    ) {
        use crate::contract::{BorderChange, Effect};
        let changed =
            crate::reload::workspaces_with_changed_borders(before, &self.monitor_manager, all);
        for monitor in self.monitor_manager.get_all_monitors() {
            let Some(workspaces) = self.monitor_manager.get_workspaces_mut(monitor.handle) else {
                continue;
            };
            let visible_index = workspaces.focused_idx();
            for (index, workspace) in workspaces.iter_mut().enumerate() {
                if !changed.contains(&(monitor.handle, workspace.id)) {
                    continue;
                }
                // Focused is the window that has the focus now, not the one
                // the workspace remembers (see the layout pass).
                let active = desktop.active_window();
                let focused = active.filter(|active| workspace.focused_window() == Some(*active));
                // Every tiled window, with whether it is the one its
                // container shows; then the floating ones that have a
                // border (they were left with the old one).
                let tiled: Vec<(crate::WindowHandle, bool)> = workspace
                    .containers
                    .iter()
                    .flat_map(|container| {
                        let shown = container.focused_window();
                        container
                            .windows
                            .iter()
                            .copied()
                            .map(move |id| (id, Some(id) == shown))
                    })
                    .collect();
                let floating: Vec<crate::WindowHandle> = workspace
                    .floating_windows
                    .iter()
                    .copied()
                    .filter(|id| desktop.has_border(*id))
                    .collect();
                let windows = tiled
                    .into_iter()
                    .map(|(id, shown)| (id, shown, false))
                    .chain(floating.into_iter().map(|id| (id, true, true)));
                for (id, shown_in_container, floats) in windows.collect::<Vec<_>>() {
                    let _ = desktop.apply(&Effect::Border {
                        id,
                        change: BorderChange::Remove,
                    });
                    if !desktop.exists(id) || desktop.is_minimized(id) || desktop.is_decoration(id)
                    {
                        continue;
                    }
                    let is_focused = if floats {
                        active == Some(id)
                    } else {
                        Some(id) == focused && shown_in_container
                    };
                    // The border the workspace gives a tiled window, or
                    // the one floating windows have.
                    if !crate::executor::Executor::dress_window(
                        workspace, desktop, id, floats, is_focused,
                    ) {
                        continue;
                    }
                    if index != visible_index || !shown_in_container {
                        let _ = desktop.apply(&Effect::Border {
                            id,
                            change: BorderChange::Visible(false),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(handle: isize, name: &str, primary: bool) -> MonitorInfo {
        MonitorInfo {
            handle: MonitorHandle(handle),
            name: name.to_string(),
            device_path: String::new(),
            is_primary: primary,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            work_x: 0,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            dpi_scale: 1.0,
        }
    }

    #[test]
    fn selectors_pick_monitors_by_role_position_and_name() {
        let monitors = [monitor(1, "Left", false), monitor(2, "Main", true)];
        let pick = |selector| monitor_selector_targets(&selector, &monitors);
        assert_eq!(pick(MonitorSelector::Primary), [MonitorHandle(2)]);
        assert_eq!(pick(MonitorSelector::Index(0)), [MonitorHandle(1)]);
        assert!(pick(MonitorSelector::Index(5)).is_empty());
        assert_eq!(
            pick(MonitorSelector::Name("main".to_string())),
            [MonitorHandle(2)]
        );
        assert_eq!(
            pick(MonitorSelector::Any),
            [MonitorHandle(1), MonitorHandle(2)]
        );
    }
}
