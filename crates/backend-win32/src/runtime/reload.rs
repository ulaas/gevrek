//! Hot reload as a diff against the running configuration.
//!
//! A reload used to reset every workspace, rebuild the executor, re-run all
//! startup automation, and put windows back by workspace name. Anything not
//! explicitly carried over was lost (resize ratios, focus inside containers,
//! learned state), unchanged startup automation fired again on every reload,
//! and every workspace on every monitor was laid out.
//!
//! Now each part of the configuration is applied only when it changed:
//! workspaces and their windows stay where they are, the executor keeps its
//! state and only swaps its graph, and only visible workspaces are laid out.
//! A configuration identical to the running one changes nothing.
//!
//! What changed is computed by `gevrek_engine::reload` (`ConfigDiff`), and
//! the steps are carried out by the host every backend shares
//! (`gevrek_host::wm::reload`); what only Windows has is in the runtime's
//! hooks (`reload_system` and the others, in `runtime/mod.rs`). This file
//! keeps the tests of the reload on Windows.

use super::Runtime;

impl Runtime {
    #[cfg(test)]
    fn executor_pending_layouts_is_empty(&self) -> bool {
        self.core.executor.pending_layouts_is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::dwm::layouts::LayoutType;
    use crate::node_editor::{BorderStyle, NodeGraph, NodeType};
    use crate::runtime::test_support::{finish, run_startup, test_runtime, TestWindow, MONITOR};
    use crate::settings::WorkspaceProfile;
    use gevrek_engine::reload::{changed_profile_names, same};
    use gevrek_host::wm::reload::Reload as _;

    /// Everything a reload must not touch unless the config says so.
    fn workspace_state(runtime: &Runtime) -> Vec<String> {
        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces(MONITOR)
            .unwrap();
        let mut state = vec![format!("focused {}", workspaces.focused_idx())];
        state.extend(workspaces.iter().map(|workspace| {
            format!(
                "{} id={} layout={:?} ratio={} windows={:?} floating={:?} focus={:?}",
                workspace.name,
                workspace.id,
                workspace.layout,
                workspace.master_ratio,
                workspace
                    .containers
                    .iter()
                    .map(|c| c.windows.clone())
                    .collect::<Vec<_>>(),
                workspace.floating_windows.iter().collect::<Vec<_>>(),
                workspace.focused_window(),
            )
        }));
        state
    }

    fn profile_config() -> Config {
        let mut config = Config::new();
        for (name, layout) in [("web", LayoutType::BSP), ("chat", LayoutType::Columns)] {
            config.desktop.workspaces.push(WorkspaceProfile {
                name: name.to_string(),
                layout,
                persistent: true,
                ..WorkspaceProfile::default()
            });
        }
        config
    }

    /// Two profile workspaces, windows on both, the second one visible, and
    /// a user resize tweak on the first.
    fn populated(runtime: &mut Runtime, windows: &[TestWindow]) {
        runtime.apply_workspace_profiles();
        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces_mut(MONITOR)
            .unwrap();
        workspaces
            .get_mut(0)
            .unwrap()
            .add_window(windows[0].0, true);
        workspaces
            .get_mut(0)
            .unwrap()
            .add_window(windows[1].0, true);
        workspaces.get_mut(0).unwrap().master_ratio = 0.7;
        workspaces
            .get_mut(1)
            .unwrap()
            .add_floating_window(windows[2].0);
        runtime.core.monitor_manager.focus_workspace_at(MONITOR, 1);
    }

    #[test]
    fn a_configuration_with_workspaces_of_its_own_takes_over_the_windows() {
        // The WM runs without any workspace profile: one workspace of its
        // own making, with a tiled and a floating window.
        let windows: Vec<_> = (0..2).map(|_| TestWindow::create()).collect();
        let mut runtime = test_runtime(Config::new());
        runtime.ensure_default_workspaces().unwrap();
        {
            let workspace = runtime
                .core
                .monitor_manager
                .get_focused_workspace_mut(MONITOR)
                .unwrap();
            workspace.add_window(windows[0].0, true);
            workspace.add_floating_window(windows[1].0);
        }
        let old_name = runtime
            .core
            .monitor_manager
            .get_focused_workspace_mut(MONITOR)
            .unwrap()
            .name
            .clone();

        // A configuration that names its workspaces is loaded (an example).
        let mut config = profile_config();
        config.desktop.workspaces[1].default = true;
        runtime.reload_configuration(config);

        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces(MONITOR)
            .unwrap();
        let names: Vec<&str> = workspaces
            .iter()
            .map(|workspace| workspace.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["web", "chat"],
            "the old workspace '{old_name}' is gone"
        );
        let shown = workspaces.focused().unwrap();
        assert_eq!(
            shown.name, "chat",
            "the default workspace of the configuration is shown"
        );
        assert_eq!(shown.all_windows().len(), 2, "with the windows");
        assert!(
            shown
                .floating_windows
                .iter()
                .any(|window| *window == windows[1].0),
            "the floating one still floats"
        );
        assert!(
            shown.find_container(windows[0].0).is_some(),
            "the tiled one is tiled"
        );

        // A workspace the user opens by hand afterwards is theirs: a later
        // change of the configuration leaves it and its windows alone.
        let own = runtime
            .core
            .monitor_manager
            .create_workspace(MONITOR, "Workspace 3".to_string(), LayoutType::BSP)
            .unwrap();
        let third = TestWindow::create();
        runtime
            .core
            .monitor_manager
            .get_workspaces_mut(MONITOR)
            .unwrap()
            .get_mut(own)
            .unwrap()
            .add_window(third.0, true);
        runtime
            .core
            .monitor_manager
            .focus_workspace_at(MONITOR, own);
        let mut recoloured = profile_config();
        recoloured.desktop.workspaces[1].default = true;
        recoloured.desktop.appearance.active_border_color = (1, 2, 3);
        runtime.reload_configuration(recoloured);
        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces(MONITOR)
            .unwrap();
        let shown = workspaces.focused().unwrap();
        assert_eq!(
            shown.name, "Workspace 3",
            "the user's own workspace is still shown"
        );
        assert_eq!(shown.all_windows(), vec![third.0]);
        runtime.core.monitor_manager.focus_workspace_at(MONITOR, 1);
        runtime
            .core
            .monitor_manager
            .get_workspaces_mut(MONITOR)
            .unwrap()
            .get_mut(own)
            .unwrap()
            .remove_window(third.0);
        runtime
            .core
            .monitor_manager
            .remove_inactive_empty_workspace(MONITOR, own);
        let mut back = profile_config();
        back.desktop.workspaces[1].default = true;
        runtime.reload_configuration(back);

        // Loading it again changes nothing more.
        let before = workspace_state(&runtime);
        let mut again = profile_config();
        again.desktop.workspaces[1].default = true;
        runtime.reload_configuration(again);
        assert_eq!(workspace_state(&runtime), before);
        finish(runtime);
    }

    #[test]
    fn reloading_the_running_configuration_changes_nothing() {
        let windows: Vec<_> = (0..3).map(|_| TestWindow::create()).collect();
        let mut runtime = test_runtime(profile_config());
        populated(&mut runtime, &windows);
        let before = workspace_state(&runtime);

        runtime.reload_configuration(profile_config());

        assert_eq!(workspace_state(&runtime), before);
        assert!(runtime.executor_pending_layouts_is_empty());
        finish(runtime);
    }

    #[test]
    fn an_unrelated_setting_leaves_every_workspace_alone() {
        let windows: Vec<_> = (0..3).map(|_| TestWindow::create()).collect();
        let mut runtime = test_runtime(profile_config());
        populated(&mut runtime, &windows);
        let before = workspace_state(&runtime);

        let mut edited = profile_config();
        edited.desktop.focus.history_size += 7;
        runtime.reload_configuration(edited);

        assert_eq!(workspace_state(&runtime), before);
        assert_eq!(
            runtime.config.desktop.focus.history_size,
            profile_config().desktop.focus.history_size + 7
        );
        finish(runtime);
    }

    #[test]
    fn editing_one_profile_changes_only_that_workspace() {
        let windows: Vec<_> = (0..3).map(|_| TestWindow::create()).collect();
        let mut runtime = test_runtime(profile_config());
        populated(&mut runtime, &windows);
        let before = workspace_state(&runtime);

        let mut edited = profile_config();
        edited.desktop.workspaces[1].layout = LayoutType::Rows;
        edited.desktop.workspaces[1].default = true;
        runtime.reload_configuration(edited);

        let after = workspace_state(&runtime);
        // "web" keeps its windows and the user's resize ratio; the visible
        // workspace does not switch even though "chat" became the default.
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], before[1]);
        assert!(after[2].contains("layout=Rows"), "{}", after[2]);
        assert_eq!(after[2].replace("layout=Rows", "layout=Columns"), before[2]);
        finish(runtime);
    }

    /// OnStartup -> GetMonitors -> ForEach -> CreateWorkspace("dev").
    fn workspace_graph(layout: LayoutType) -> NodeGraph {
        let mut graph = NodeGraph::new();
        let startup = graph.add_node(NodeType::OnStartup, [0.0, 0.0]);
        let monitors = graph.add_node(NodeType::GetMonitors, [100.0, 0.0]);
        let each = graph.add_node(NodeType::ForEach, [200.0, 0.0]);
        let create = graph.add_node(
            NodeType::CreateWorkspace {
                workspace_name: "dev".to_string(),
                layout,
                padding_x: 4,
                padding_y: 4,
                workspace_padding_x: 8,
                workspace_padding_y: 8,
                border_enabled: false,
                border_width: 2,
                border_offset: 0,
                border_style: BorderStyle::Square,
                active_border_color: (255, 255, 255),
                inactive_border_color: (0, 0, 0),
                active_border_alpha: 255,
                inactive_border_alpha: 255,
            },
            [300.0, 0.0],
        );
        graph
            .add_connection_by_name(startup, "Event", monitors, "Trigger")
            .unwrap();
        graph
            .add_connection_by_name(monitors, "Then", each, "Trigger")
            .unwrap();
        graph
            .add_connection_by_name(monitors, "Monitors", each, "List")
            .unwrap();
        graph
            .add_connection_by_name(each, "Loop Body", create, "Trigger")
            .unwrap();
        graph
            .add_connection_by_name(each, "Item", create, "Monitor")
            .unwrap();
        graph
    }

    fn graph_config(layout: LayoutType) -> Config {
        let mut config = Config::new();
        config.node_graph = workspace_graph(layout);
        config
    }

    #[test]
    fn a_graph_edit_updates_its_workspace_in_place() {
        let window = TestWindow::create();
        let mut runtime = test_runtime(graph_config(LayoutType::BSP));
        run_startup(&mut runtime);
        {
            let workspaces = runtime
                .core
                .monitor_manager
                .get_workspaces_mut(MONITOR)
                .unwrap();
            assert_eq!(workspaces.len(), 1);
            workspaces.get_mut(0).unwrap().add_window(window.0, true);
        }

        // Moving a node is a graph change, but no CreateWorkspace changed:
        // no duplicate workspace and no settings reset.
        let mut moved = graph_config(LayoutType::BSP);
        let first = *moved.node_graph.nodes.keys().min().unwrap();
        moved.node_graph.nodes.get_mut(&first).unwrap().position = [5.0, 5.0];
        runtime
            .core
            .monitor_manager
            .get_workspaces_mut(MONITOR)
            .unwrap()
            .get_mut(0)
            .unwrap()
            .master_ratio = 0.65;
        runtime.reload_configuration(moved);
        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces(MONITOR)
            .unwrap();
        assert_eq!(workspaces.len(), 1, "reload duplicated the workspace");
        assert_eq!(workspaces.get(0).unwrap().master_ratio, 0.65);
        assert_eq!(workspaces.get(0).unwrap().layout, LayoutType::BSP);

        // Editing the node updates the existing workspace and keeps its window.
        let mut edited = graph_config(LayoutType::Columns);
        edited.node_graph.nodes.get_mut(&first).unwrap().position = [5.0, 5.0];
        runtime.reload_configuration(edited);
        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces(MONITOR)
            .unwrap();
        assert_eq!(workspaces.len(), 1);
        let dev = workspaces.get(0).unwrap();
        assert_eq!(dev.layout, LayoutType::Columns);
        assert_eq!(dev.all_windows(), vec![window.0]);
        finish(runtime);
    }

    #[test]
    fn a_workspace_dropped_from_the_graph_is_removed_only_when_unused() {
        let window = TestWindow::create();
        let mut config = graph_config(LayoutType::BSP);
        config.desktop.workspaces.push(WorkspaceProfile {
            name: "main".to_string(),
            persistent: true,
            ..WorkspaceProfile::default()
        });
        let mut runtime = test_runtime(config.clone());
        runtime.apply_workspace_profiles();
        run_startup(&mut runtime);
        let names = |runtime: &Runtime| -> Vec<String> {
            runtime
                .core
                .monitor_manager
                .get_workspaces(MONITOR)
                .unwrap()
                .iter()
                .map(|workspace| workspace.name.clone())
                .collect()
        };
        assert_eq!(names(&runtime), vec!["main", "dev"]);

        // While "dev" holds a window it survives the graph losing it.
        runtime
            .core
            .monitor_manager
            .get_workspaces_mut(MONITOR)
            .unwrap()
            .get_mut(1)
            .unwrap()
            .add_window(window.0, true);
        let mut without_dev = config.clone();
        without_dev.node_graph = NodeGraph::new();
        runtime.reload_configuration(without_dev.clone());
        assert_eq!(names(&runtime), vec!["main", "dev"]);

        // Put the graph back, empty "dev", and drop it again: now it goes.
        runtime.reload_configuration(config);
        runtime
            .core
            .monitor_manager
            .find_and_remove_window(window.0);
        runtime.reload_configuration(without_dev);
        assert_eq!(names(&runtime), vec!["main"]);
        finish(runtime);
    }

    fn profile(name: &str) -> WorkspaceProfile {
        WorkspaceProfile {
            name: name.to_string(),
            ..WorkspaceProfile::default()
        }
    }

    #[test]
    fn only_new_or_edited_profiles_are_reapplied() {
        let old = vec![profile("web"), profile("chat")];
        let mut edited = profile("chat");
        edited.persistent = !edited.persistent;
        let new = vec![profile("web"), edited, profile("code")];
        let mut changed: Vec<_> = changed_profile_names(&old, &new).into_iter().collect();
        changed.sort();
        assert_eq!(changed, vec!["chat".to_string(), "code".to_string()]);
    }

    #[test]
    fn structural_equality_sees_through_serialization() {
        let config = Config::with_default_graph();
        assert!(same(&config, &config.clone()));
        let mut changed = config.clone();
        changed.desktop.appearance.border_width += 1;
        assert!(!same(&config, &changed));
    }
}
