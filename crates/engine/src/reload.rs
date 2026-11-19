//! Hot reload as a diff against the running configuration
//! (docs/PHASE2_PLAN.md, step 4.10).
//!
//! Each part of the configuration is applied only when it changed:
//! workspaces and their windows stay where they are, and a configuration
//! identical to the running one changes nothing. This module computes what
//! changed; the host applies each changed part.

use crate::outputs::Outputs;
use crate::workspace::Workspace;
use gevrek_model::ids::{OutputId, WindowId};
use gevrek_model::settings::WorkspaceProfile;
use gevrek_model::Config;
use std::collections::{HashMap, HashSet};

/// Structural equality through the serialized form (the configuration types
/// do not implement `PartialEq`).
pub fn same<T: serde::Serialize>(left: &T, right: &T) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn profile_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Workspace profiles whose definition is new or differs from `old`
/// (lower-case names).
pub fn changed_profile_names(
    old: &[WorkspaceProfile],
    new: &[WorkspaceProfile],
) -> HashSet<String> {
    // (A workspace can have several entries: one for every display and
    // one of a display's own. All of them together are what is compared;
    // one that is no longer written down has changed too.)
    let grouped = |profiles: &[WorkspaceProfile]| {
        let mut by_name: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        for profile in profiles {
            by_name
                .entry(profile_key(&profile.name))
                .or_default()
                .push(serde_json::to_value(profile).unwrap_or_default());
        }
        by_name
    };
    let (before, after) = (grouped(old), grouped(new));
    let mut changed: HashSet<String> = after
        .iter()
        .filter(|(name, entries)| before.get(*name) != Some(*entries))
        .map(|(name, _)| name.clone())
        .collect();
    changed.extend(before.keys().filter(|name| !after.contains_key(*name)).cloned());
    // Written in another order: the displays' workspaces are put in it.
    // Every one counts as changed then (nothing else says "the order").
    let order = |profiles: &[WorkspaceProfile]| -> Vec<String> {
        let mut seen = Vec::new();
        for profile in profiles {
            let name = profile_key(&profile.name);
            if !seen.contains(&name) {
                seen.push(name);
            }
        }
        seen
    };
    let (was, is) = (order(old), order(new));
    let kept = |names: &[String], other: &[String]| -> Vec<String> {
        names.iter().filter(|name| other.contains(name)).cloned().collect()
    };
    if kept(&was, &is) != kept(&is, &was) {
        changed.extend(is);
    }
    changed
}

/// Which parts of the configuration differ between the running one and a
/// new one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigDiff {
    /// The two configurations are identical: nothing is applied.
    pub unchanged: bool,
    pub graph: bool,
    pub appearance: bool,
    pub startup: bool,
    pub animations: bool,
    pub focus: bool,
    /// Monitor profiles or the global work-area offset.
    pub monitors: bool,
    pub snap_zones: bool,
    pub scratchpads: bool,
    /// Workspace profiles that are new or edited (lower-case names).
    pub profiles: HashSet<String>,
}

impl ConfigDiff {
    pub fn between(old: &Config, new: &Config) -> Self {
        if same(old, new) {
            return Self {
                unchanged: true,
                ..Self::default()
            };
        }
        let (before, after) = (&old.desktop, &new.desktop);
        Self {
            unchanged: false,
            graph: !same(&old.node_graph, &new.node_graph),
            // What only one backend applies to every window (backdrop,
            // corners, shadow) is part of how windows look.
            appearance: !same(&before.appearance, &after.appearance)
                || !same(&before.platform, &after.platform),
            startup: !same(&before.startup, &after.startup),
            animations: !same(&before.animations, &after.animations),
            focus: !same(&before.focus, &after.focus),
            monitors: !same(&before.monitors, &after.monitors)
                || !same(&before.work_area_offset, &after.work_area_offset)
                || !same(&before.shell.panel, &after.shell.panel),
            snap_zones: !same(&before.snap_zones, &after.snap_zones),
            scratchpads: !same(&before.scratchpads, &after.scratchpads),
            profiles: changed_profile_names(&before.workspaces, &after.workspaces),
        }
    }

    /// Whether a workspace profile must be applied again. When the
    /// appearance changed, every profile is.
    pub fn reapplies_profile(&self, name: &str) -> bool {
        self.appearance || self.profiles.contains(&profile_key(name))
    }

    pub fn reapplies_any_profile(&self) -> bool {
        self.appearance || !self.profiles.is_empty()
    }

    /// Snap zone layouts depend on the monitors as well.
    pub fn rebuilds_snap_zones(&self) -> bool {
        self.monitors || self.snap_zones
    }
}

/// One step of applying a configuration change to a running WM. Both
/// hosts (Windows, Wayland) carry the steps out with their own means; the
/// order, and which steps a change needs, is decided here once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadStep {
    /// The file watcher follows `watch_configuration`.
    Watcher,
    /// The startup programs changed.
    Startup,
    /// The animation profile changed.
    Animations,
    /// The focus settings changed.
    Focus,
    /// Monitor profiles or the global work-area offset changed.
    Monitors,
    /// Snap zone layouts changed, or the monitors they depend on.
    SnapZones,
    /// The window rules are rebuilt.
    Rules,
    /// Scratchpads changed: release the removed ones, store the new set.
    Scratchpads,
    /// The executor is given the workspace profiles and the appearance.
    StoreProfiles,
    /// Windows that are gone are dropped.
    PruneWindows,
    /// Remember what borders look like now (for `RefreshBorders`).
    SnapshotBorders,
    /// The graph changed: swap it and run its startup again.
    Graph,
    /// The graph did not change: evaluate the dynamic rules again, with
    /// the workspaces as the configuration has them now.
    DynamicRules,
    /// Key bindings and gestures are registered again.
    Bindings,
    /// The appearance changed: apply it to workspaces and windows.
    Appearance,
    /// Apply the workspace profiles that changed.
    ReapplyProfiles,
    /// Windows the new rules ignore leave management.
    ReleaseIgnored,
    /// Recreate the borders whose settings changed.
    RefreshBorders,
    /// Every output is laid out again.
    MarkLayouts,
    /// Scratchpads are put back where they belong.
    ReassertScratchpads,
    /// Opacity and borders follow the focus.
    FocusEffects,
}

impl ConfigDiff {
    /// The steps this change needs, in the order they must run.
    pub fn steps(&self) -> Vec<ReloadStep> {
        use ReloadStep::*;
        if self.unchanged {
            return Vec::new();
        }
        let mut steps = vec![Watcher];
        let mut when = |needed: bool, step: ReloadStep| {
            if needed {
                steps.push(step);
            }
        };
        when(self.startup, Startup);
        when(self.animations, Animations);
        when(self.focus, Focus);
        when(self.monitors, Monitors);
        when(self.rebuilds_snap_zones(), SnapZones);
        when(true, Rules);
        when(self.scratchpads, Scratchpads);
        when(true, StoreProfiles);
        when(true, PruneWindows);
        when(true, SnapshotBorders);
        when(self.graph, Graph);
        when(true, Bindings);
        when(self.appearance, Appearance);
        when(self.reapplies_any_profile(), ReapplyProfiles);
        // After the workspaces are as the configuration has them: a rule
        // that goes by a window's workspace (its name, its place among
        // the display's) is held against the workspaces there are now,
        // not against those of before (a workspace taken out, another
        // put before it).
        when(!self.graph, DynamicRules);
        when(true, ReleaseIgnored);
        when(true, RefreshBorders);
        when(true, MarkLayouts);
        when(true, ReassertScratchpads);
        when(true, FocusEffects);
        steps
    }
}

/// Every window that is a workspace member, on any output.
pub fn managed_windows(outputs: &Outputs) -> Vec<WindowId> {
    outputs
        .get_all_monitors()
        .into_iter()
        .filter_map(|monitor| outputs.get_workspaces(monitor.handle))
        .flat_map(|workspaces| {
            workspaces
                .iter()
                .flat_map(|workspace| workspace.all_windows())
        })
        .collect()
}

/// What happened to a workspace the new graph no longer creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedWorkspace {
    pub output: OutputId,
    pub name: String,
    /// False when it was kept because it is visible or holds windows.
    pub removed: bool,
}

/// Workspaces the previous graph created and the new one does not are
/// removed when nothing else needs them: empty, not visible, and not
/// defined by a workspace profile (`has_profile`). Anything else stays.
pub fn remove_dropped_workspaces(
    outputs: &mut Outputs,
    dropped: Vec<(OutputId, String)>,
    has_profile: impl Fn(&str) -> bool,
) -> Vec<DroppedWorkspace> {
    let mut result = Vec::new();
    for (output, name) in dropped {
        if has_profile(&name) {
            continue;
        }
        let Some(index) = outputs.find_workspace_index(output, &name) else {
            continue;
        };
        let removed = outputs.remove_inactive_empty_workspace(output, index);
        result.push(DroppedWorkspace {
            output,
            name,
            removed,
        });
    }
    result
}

/// The border settings of every workspace, keyed by output and workspace id.
pub type BorderSnapshot = HashMap<(OutputId, usize), String>;

fn border_settings(workspace: &Workspace) -> String {
    format!(
        "{:?}",
        (
            workspace.border_enabled,
            workspace.border_width,
            workspace.border_offset,
            workspace.border_style,
            workspace.active_border_color,
            workspace.inactive_border_color,
            workspace.active_border_alpha,
            workspace.inactive_border_alpha,
            &workspace.floating_look,
        )
    )
}

pub fn border_snapshot(outputs: &Outputs) -> BorderSnapshot {
    let mut snapshot = HashMap::new();
    for monitor in outputs.get_all_monitors() {
        let Some(workspaces) = outputs.get_workspaces(monitor.handle) else {
            continue;
        };
        for workspace in workspaces.iter() {
            snapshot.insert((monitor.handle, workspace.id), border_settings(workspace));
        }
    }
    snapshot
}

/// The workspaces whose border settings differ from `before` (all of them
/// when `all` is set), as output and workspace id.
pub fn workspaces_with_changed_borders(
    before: &BorderSnapshot,
    outputs: &Outputs,
    all: bool,
) -> HashSet<(OutputId, usize)> {
    border_snapshot(outputs)
        .into_iter()
        .filter(|(key, settings)| all || before.get(key) != Some(settings))
        .map(|(key, _)| key)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::LayoutType;

    fn profile(name: &str) -> WorkspaceProfile {
        WorkspaceProfile {
            name: name.to_string(),
            ..WorkspaceProfile::default()
        }
    }

    #[test]
    fn an_identical_configuration_is_unchanged() {
        let config = Config::default();
        let diff = ConfigDiff::between(&config, &config.clone());
        assert!(diff.unchanged);
        assert!(!diff.reapplies_any_profile());
    }

    #[test]
    fn each_section_is_reported_on_its_own() {
        let old = Config::default();
        let mut new = old.clone();
        new.desktop.workspaces = vec![profile("Web")];
        let diff = ConfigDiff::between(&old, &new);
        assert!(!diff.unchanged && !diff.graph && !diff.appearance && !diff.monitors);
        assert!(diff.reapplies_profile(" WEB "));
        assert!(!diff.reapplies_profile("other"));

        let mut edited = new.clone();
        edited.desktop.workspaces.push(profile("Mail"));
        let diff = ConfigDiff::between(&new, &edited);
        assert_eq!(
            diff.profiles,
            HashSet::from(["mail".to_string()]),
            "only the new profile"
        );
    }

    #[test]
    fn dropped_graph_workspaces_go_only_when_unused() {
        const A: OutputId = OutputId(1);
        let mut outputs = Outputs::new();
        outputs.add_test_monitor(A, "A");
        for name in ["one", "old", "busy", "profiled"] {
            outputs.create_workspace(A, name.into(), LayoutType::BSP);
        }
        outputs.focus_workspace_at(A, 0);
        let busy = outputs.find_workspace_index(A, "busy").unwrap();
        let workspaces = outputs.get_workspaces_mut(A).unwrap();
        workspaces
            .get_mut(busy)
            .unwrap()
            .add_window(WindowId(1), true);

        let dropped = ["one", "old", "busy", "profiled", "missing"]
            .map(|name| (A, name.to_string()))
            .to_vec();
        let result = remove_dropped_workspaces(&mut outputs, dropped, |name| name == "profiled");
        let removed: Vec<(&str, bool)> = result
            .iter()
            .map(|dropped| (dropped.name.as_str(), dropped.removed))
            .collect();
        assert_eq!(
            removed,
            vec![("one", false), ("old", true), ("busy", false)]
        );
        assert_eq!(managed_windows(&outputs), vec![WindowId(1)]);
    }

    #[test]
    fn only_workspaces_whose_border_settings_changed_are_reported() {
        const A: OutputId = OutputId(1);
        let mut outputs = Outputs::new();
        outputs.add_test_monitor(A, "A");
        outputs.create_workspace(A, "one".into(), LayoutType::BSP);
        outputs.create_workspace(A, "two".into(), LayoutType::BSP);
        let before = border_snapshot(&outputs);
        assert!(workspaces_with_changed_borders(&before, &outputs, false).is_empty());

        let (id, _) = {
            let workspace = outputs.get_workspaces_mut(A).unwrap().get_mut(1).unwrap();
            workspace.border_width += 2;
            (workspace.id, ())
        };
        assert_eq!(
            workspaces_with_changed_borders(&before, &outputs, false),
            HashSet::from([(A, id)])
        );
        assert_eq!(
            workspaces_with_changed_borders(&before, &outputs, true).len(),
            2
        );
    }

    #[test]
    fn the_steps_of_a_reload_follow_what_changed_in_one_order() {
        use super::ReloadStep::*;
        let base = Config::with_default_graph();
        assert!(ConfigDiff::between(&base, &base).steps().is_empty());

        // One unrelated setting: only the steps every change needs.
        let mut focus = base.clone();
        focus.desktop.focus.history_size += 1;
        assert_eq!(
            ConfigDiff::between(&base, &focus).steps(),
            [
                Watcher,
                Focus,
                Rules,
                StoreProfiles,
                PruneWindows,
                SnapshotBorders,
                Bindings,
                DynamicRules,
                ReleaseIgnored,
                RefreshBorders,
                MarkLayouts,
                ReassertScratchpads,
                FocusEffects
            ]
        );

        // The rules are held against the workspaces as they are after
        // the change: a workspace taken out, or put elsewhere among the
        // display's, is a change of its profile.
        let mut fewer = base.clone();
        fewer.desktop.workspaces.push(gevrek_model::settings::WorkspaceProfile {
            name: "another".to_string(),
            ..Default::default()
        });
        let steps = ConfigDiff::between(&fewer, &base).steps();
        let at = |step: ReloadStep| steps.iter().position(|one| *one == step);
        assert!(at(ReapplyProfiles).is_some());
        assert!(at(ReapplyProfiles) < at(DynamicRules));
        assert!(at(DynamicRules) < at(ReleaseIgnored));

        // A graph change runs the graph instead of the dynamic rules.
        let mut graph = base.clone();
        graph
            .node_graph
            .add_node(gevrek_model::graph::NodeType::OnStartup, [900.0, 900.0]);
        let steps = ConfigDiff::between(&base, &graph).steps();
        assert!(steps.contains(&Graph) && !steps.contains(&DynamicRules));

        // A setting only one backend applies (the backdrop) is an
        // appearance change: it is applied again, and so are the profiles.
        let mut backdrop = base.clone();
        backdrop.desktop.platform.windows.backdrop =
            gevrek_model::platform::windows::SystemBackdrop::Acrylic;
        let diff = ConfigDiff::between(&base, &backdrop);
        assert!(diff.appearance);
        let steps = diff.steps();
        let at = |step| {
            steps
                .iter()
                .position(|other| *other == step)
                .expect("step present")
        };
        assert!(at(SnapshotBorders) < at(Appearance));
        assert!(at(Appearance) < at(ReapplyProfiles));
        assert!(at(ReapplyProfiles) < at(RefreshBorders));
        assert!(at(RefreshBorders) < at(MarkLayouts));
    }
}
