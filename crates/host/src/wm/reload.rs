//! A new configuration, applied as a diff against the running one.
//!
//! What changed is the engine's to say (`gevrek_engine::reload`,
//! `ConfigDiff`), and so is the order of the steps; each step is carried
//! out here. Workspaces and their windows stay where they are, the
//! executor keeps its state and only swaps its graph, and only the visible
//! workspaces are laid out. A configuration identical to the running one
//! changes nothing.
//!
//! What only a system has (its displays' modes, its pointer, its session's
//! variables) it applies itself, in `reload_system`, before the steps.

use super::bindings::Bindings;
use super::WmHost;
use gevrek_engine::executor::RuntimeEventKind;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::reload::{self as plan, ConfigDiff, ReloadStep};
use gevrek_model::config::Config;
use gevrek_model::settings::WorkspaceProfile;

/// The host's reload, for every backend.
pub trait Reload: WmHost {
    /// Apply `new_config` as a diff against the running configuration.
    fn reload_configuration(&mut self, new_config: Config) {
        let old = {
            let parts = self.parts();
            parts.host.config_dirty = true;
            std::mem::replace(parts.config, new_config)
        };
        self.reload_system(&old);
        let diff = ConfigDiff::between(&old, self.config());
        if diff.unchanged {
            log::info!("Configuration reload: nothing changed");
            emit_reloaded(self);
            return;
        }
        log::info!("Applying configuration reload as a diff");
        let desktop = self.config().desktop.clone();
        let mut borders_before = None;
        let mut windows_changed_workspace = false;
        // Workspaces the previous configuration made and the new one does
        // not: those the old graph created and the new one dropped, and,
        // when the old configuration named no workspace at all, the one the
        // WM made by itself.
        let mut leftovers: Vec<(MonitorHandle, String)> = Vec::new();
        if !old.desktop.workspaces.iter().any(|profile| profile.enabled) {
            let core = &*self.parts().core;
            for monitor in core.monitor_manager.get_all_monitors() {
                if let Some(workspace) = core
                    .monitor_manager
                    .get_workspaces(monitor.handle)
                    .and_then(|ring| ring.focused())
                {
                    if !core
                        .executor
                        .graph_creates_workspace(monitor.handle, &workspace.name)
                    {
                        leftovers.push((monitor.handle, workspace.name.clone()));
                    }
                }
            }
        }

        for step in diff.steps() {
            match step {
                ReloadStep::Watcher => {
                    if let Some(watcher) = &mut self.parts().host.watcher {
                        watcher.set_enabled(desktop.watch_configuration);
                    }
                }
                ReloadStep::Startup => self.parts().host.startup.configure(&desktop.startup, true),
                ReloadStep::Animations => self.set_animations(&desktop.animations),
                ReloadStep::Focus => self
                    .parts()
                    .core
                    .focus_manager
                    .set_config(desktop.focus.clone()),
                ReloadStep::Monitors => self.configure_monitors(),
                ReloadStep::SnapZones => {
                    let probe = self.snap_zone_modifier_probe();
                    let core = self.parts().core;
                    core.snap_zone_manager = gevrek_engine::config_apply::build_snap_zone_manager(
                        &desktop.snap_zones,
                        &core.monitor_manager,
                        probe,
                    );
                }
                ReloadStep::Rules => {
                    *self
                        .rules()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        gevrek_engine::config_apply::build_rule_engine(&desktop);
                }
                ReloadStep::Scratchpads => {
                    let parts = self.parts();
                    parts.core.executor.release_removed_scratchpads(
                        &desktop.scratchpads,
                        parts.desktop,
                        &mut parts.core.monitor_manager,
                    );
                    parts
                        .core
                        .executor
                        .set_scratchpad_profiles(desktop.scratchpads.clone());
                }
                ReloadStep::StoreProfiles => {
                    let appearance = self.system_appearance(&desktop.appearance);
                    self.parts()
                        .core
                        .executor
                        .set_workspace_profiles(desktop.workspaces.clone(), appearance);
                }
                ReloadStep::PruneWindows => {
                    self.prune_windows();
                    let parts = self.parts();
                    parts.core.monitor_manager.cleanup_stale_windows(parts.desktop);
                }
                ReloadStep::SnapshotBorders => {
                    borders_before = Some(plan::border_snapshot(&self.parts().core.monitor_manager));
                }
                ReloadStep::Graph => {
                    let rerun = {
                        let parts = self.parts();
                        parts.core.executor.set_graph(parts.config.node_graph.clone());
                        parts.core.executor.clear_state();
                        parts
                            .core
                            .executor
                            .rerun_startup_for_reload(parts.desktop, &mut parts.core.monitor_manager)
                    };
                    match rerun {
                        Ok(dropped) => {
                            leftovers.extend(dropped.iter().cloned());
                            remove_dropped_graph_workspaces(self, dropped);
                        }
                        Err(error) => {
                            log::error!("Startup automation failed during reload: {error}")
                        }
                    }
                }
                ReloadStep::DynamicRules => {
                    let parts = self.parts();
                    if let Err(error) = parts
                        .core
                        .executor
                        .reevaluate_all_dynamic_rules(parts.desktop, &mut parts.core.monitor_manager)
                    {
                        log::error!("Dynamic rule re-evaluation failed during reload: {error}");
                    }
                }
                ReloadStep::Bindings => {
                    if let Err(error) = self.refresh_bindings() {
                        log::error!("Error re-registering bindings after reload: {error}");
                    }
                }
                ReloadStep::Appearance => self.apply_appearance(),
                // On reload a profile's `default` flag does not switch the
                // visible workspace.
                ReloadStep::ReapplyProfiles => {
                    self.apply_workspace_profile_set(
                        |profile| diff.reapplies_profile(&profile.name),
                        false,
                    );
                    let handed_over = {
                        let parts = self.parts();
                        parts.core.hand_over_leftover_workspaces(
                            parts.desktop,
                            &desktop.workspaces,
                            &leftovers,
                        )
                    };
                    for (monitor, old, new) in handed_over {
                        // Windows are on a workspace with other settings now.
                        windows_changed_workspace = true;
                        self.host().emit(
                            RuntimeEventKind::WorkspaceDestroyed,
                            None,
                            [
                                ("monitor".to_string(), monitor.0.to_string()),
                                ("name".to_string(), old),
                                ("reason".to_string(), format!("its windows went to '{new}'")),
                            ],
                        );
                    }
                }
                ReloadStep::ReleaseIgnored => {
                    let parts = self.parts();
                    parts.core.release_newly_ignored_windows(parts.desktop);
                }
                ReloadStep::RefreshBorders => {
                    if let Some(before) = &borders_before {
                        let parts = self.parts();
                        parts.core.refresh_changed_borders(
                            parts.desktop,
                            before,
                            diff.appearance || windows_changed_workspace,
                        );
                    }
                }
                // Only visible workspaces are laid out (the settle covers
                // the focused workspace of each output).
                ReloadStep::MarkLayouts => {
                    let core = self.parts().core;
                    for monitor in core.monitor_manager.get_all_monitors() {
                        core.executor.mark_layout_dirty(monitor.handle);
                    }
                }
                ReloadStep::ReassertScratchpads => {
                    let parts = self.parts();
                    parts
                        .core
                        .executor
                        .reassert_scratchpads(parts.desktop, &mut parts.core.monitor_manager);
                }
                // The settle, below, lets them follow.
                ReloadStep::FocusEffects => {}
            }
        }
        self.settle();
        log::info!(
            "Configuration reload applied (graph {})",
            if diff.graph { "changed" } else { "unchanged" }
        );
        emit_reloaded(self);
    }

    /// Apply the workspace profiles `include` selects, tell of the
    /// workspaces that were made, and start the program of a profile whose
    /// workspace was made empty (once a session). `focus_default` switches
    /// each output to its `default` profile's workspace, which only the
    /// start does.
    fn apply_workspace_profile_set(
        &mut self,
        include: impl Fn(&WorkspaceProfile) -> bool,
        focus_default: bool,
    ) {
        let appearance = self.system_appearance(&self.config().desktop.appearance);
        let created = {
            let parts = self.parts();
            parts.core.apply_workspace_profile_set(
                &parts.config.desktop.workspaces,
                &appearance,
                include,
                focus_default,
            )
        };
        for workspace in created {
            self.host().emit(
                RuntimeEventKind::WorkspaceCreated,
                None,
                [
                    ("monitor".to_string(), workspace.monitor.0.to_string()),
                    ("name".to_string(), workspace.profile.name.clone()),
                    (
                        "persistent".to_string(),
                        workspace.profile.persistent.to_string(),
                    ),
                ],
            );
            let Some(launch) = &workspace.profile.on_created_empty else {
                continue;
            };
            let parts = self.parts();
            let key = (
                workspace.monitor,
                workspace.profile.name.trim().to_ascii_lowercase(),
            );
            if !parts.host.workspace_hooks_fired.insert(key) {
                continue;
            }
            match gevrek_engine::Desktop::launch(parts.desktop, launch) {
                Ok(pid) => log::info!(
                    "Workspace '{}' created empty; launched process {pid}",
                    workspace.profile.name
                ),
                Err(error) => log::error!(
                    "Workspace '{}' on-created-empty launch failed: {error}",
                    workspace.profile.name
                ),
            }
        }
    }
}

impl<T: WmHost + ?Sized> Reload for T {}

/// Workspaces the previous graph created and the new one does not are
/// removed when nothing else needs them: empty, not visible, and not
/// defined by a workspace profile. Anything else stays.
fn remove_dropped_graph_workspaces<H: WmHost + ?Sized>(
    host: &mut H,
    dropped: Vec<(MonitorHandle, String)>,
) {
    let results = host.parts().core.remove_dropped_graph_workspaces(dropped);
    for dropped in results {
        if dropped.removed {
            host.host().emit(
                RuntimeEventKind::WorkspaceDestroyed,
                None,
                [
                    ("monitor".to_string(), dropped.output.0.to_string()),
                    ("name".to_string(), dropped.name),
                    ("reason".to_string(), "removed from the graph".to_string()),
                ],
            );
        } else {
            log::info!(
                "Workspace '{}' is no longer created by the graph but is kept (visible or not empty)",
                dropped.name
            );
        }
    }
}

fn emit_reloaded<H: WmHost + ?Sized>(host: &H) {
    let config = host.config();
    host.host().emit(
        RuntimeEventKind::ConfigurationReloaded,
        None,
        [
            ("config_version".to_string(), config.version.clone()),
            (
                "schema_version".to_string(),
                config.desktop.schema_version.to_string(),
            ),
        ],
    );
}
