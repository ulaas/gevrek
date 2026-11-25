//! The executor's tests that need real windows and the Win32 window
//! manager. The executor itself lives in `gevrek_engine::executor`.

// What the executor's own module had in scope when these tests lived in it.
#![allow(unused_imports)]
use crate::dwm::layouts::{EdgeDirection, LayoutAxis, Sizing};
use crate::dwm::HwndExt;
use crate::dwm::MonitorManagerWin32;
use crate::dwm::{
    stackbar_content_rect, FocusManager, MonitorHandle, MonitorManager, RuleResolution,
    RuleResolutionScope, SnapZoneManager, StackbarContainerState, WindowEvent, WindowHandle,
    WindowManager,
};
use crate::node_editor::{
    AnimationType, BorderStyle, LogEntry, NodeGraph, NodeId, NodeType, PinId, RuleDefinition,
    TileLayout, WindowMode,
};
use crate::persistence::PersistentWindowSize;
use crate::runtime::{
    Direction, DispatchCommand, MonitorTarget, WindowSelector, WorkspaceSelector,
};
use crate::settings::{
    AppearanceProfile, LaunchProfile, MonitorSelector, ScratchpadProfile, WorkspaceProfile,
};
use anyhow::Result;
use gevrek_engine::constraints::{self, resolve_rule_rect, PersistentSizes};
use gevrek_engine::contract::{Desktop, Effect};
use gevrek_engine::executor::*;
use gevrek_engine::rule_state::{
    DragState, DynamicRuleBaseline, DynamicRuleState, PersistentSizeKey, RuleSize, WindowStateMap,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod manual_trigger_tests {
    use super::*;
    use crate::dwm::RuleEngine;
    use crate::dwm::RulePosition;
    use crate::settings::RuleMeasure;
    use gevrek_engine::constraints::constrain_rule_size;

    fn test_executor() -> Executor {
        Executor::new(
            NodeGraph::new(),
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(Mutex::new(RuleEngine::new())),
        )
    }

    #[test]
    fn persistent_size_cache_normalizes_filters_and_round_trips() {
        let mut executor = test_executor();
        executor.set_persistent_window_sizes(vec![
            PersistentWindowSize {
                exe: "Editor.EXE".to_string(),
                initial_class: "EditorWindow".to_string(),
                initial_title: "Project Alpha".to_string(),
                width: 1100,
                height: 720,
            },
            PersistentWindowSize {
                exe: "invalid.exe".to_string(),
                initial_class: "Invalid".to_string(),
                initial_title: "Invalid".to_string(),
                width: 0,
                height: 720,
            },
        ]);

        assert_eq!(
            executor.persistent_window_sizes(),
            vec![PersistentWindowSize {
                exe: "editor.exe".to_string(),
                initial_class: "editorwindow".to_string(),
                initial_title: "project alpha".to_string(),
                width: 1100,
                height: 720,
            }]
        );
    }

    #[test]
    fn dynamic_persistent_size_setting_overrides_static_source() {
        let mut executor = test_executor();
        let handle = WindowHandle(42);
        executor
            .window_state
            .get_mut(handle)
            .persistent_size
            .static_value = Some(true);
        assert!(executor.window_state.get(handle).persistent_size.enabled());

        executor
            .window_state
            .get_mut(handle)
            .persistent_size
            .dynamic_value = Some(false);
        assert!(!executor.window_state.get(handle).persistent_size.enabled());

        executor
            .window_state
            .get_mut(handle)
            .persistent_size
            .dynamic_value = None;
        assert!(executor.window_state.get(handle).persistent_size.enabled());
    }

    #[test]
    fn dynamic_initial_focus_setting_overrides_static_and_yields_to_stay_focused() {
        let mut executor = test_executor();
        let handle = WindowHandle(42);
        executor
            .window_state
            .get_mut(handle)
            .no_initial_focus
            .static_value = Some(true);
        assert!(executor.no_initial_focus_enabled(handle));

        executor
            .window_state
            .get_mut(handle)
            .no_initial_focus
            .dynamic_value = Some(false);
        assert!(!executor.no_initial_focus_enabled(handle));

        executor
            .window_state
            .get_mut(handle)
            .no_initial_focus
            .dynamic_value = Some(true);
        executor
            .window_state
            .get_mut(handle)
            .stay_focused
            .dynamic_value = Some(true);
        assert!(!executor.no_initial_focus_enabled(handle));
    }

    #[test]
    fn focus_policy_overrides_are_dynamic_and_stay_focused_wins_conflicts() {
        let mut executor = test_executor();
        let handle = WindowHandle(42);
        executor.window_state.get_mut(handle).no_focus.static_value = Some(true);
        executor
            .window_state
            .get_mut(handle)
            .no_follow_mouse
            .static_value = Some(true);
        assert!(executor.no_focus_enabled(handle));
        assert!(!executor.allows_follow_mouse_focus(handle));

        executor.window_state.get_mut(handle).no_focus.dynamic_value = Some(false);
        executor
            .window_state
            .get_mut(handle)
            .no_follow_mouse
            .dynamic_value = Some(false);
        assert!(!executor.no_focus_enabled(handle));
        assert!(executor.allows_follow_mouse_focus(handle));

        executor.window_state.get_mut(handle).no_focus.dynamic_value = Some(true);
        executor
            .window_state
            .get_mut(handle)
            .stay_focused
            .dynamic_value = Some(true);
        assert!(!executor.no_focus_enabled(handle));
        assert!(executor.allows_follow_mouse_focus(handle));
    }

    #[test]
    fn floating_size_constraints_resolve_percentages_and_reconcile_conflicts() {
        let work_area = crate::dwm::Rect::new(100, 50, 1600, 900);
        let min_size = (RuleMeasure::Pixels(400), RuleMeasure::Percent(30));
        let max_size = (RuleMeasure::Percent(75), RuleMeasure::Pixels(700));

        assert_eq!(
            constrain_rule_size(
                200,
                1000,
                work_area,
                Some(&min_size),
                Some(&max_size),
                None,
                None
            ),
            (400, 700)
        );
        let conflicting_min = (RuleMeasure::Pixels(900), RuleMeasure::Pixels(700));
        let conflicting_max = (RuleMeasure::Pixels(600), RuleMeasure::Pixels(400));
        assert_eq!(
            constrain_rule_size(
                500,
                300,
                work_area,
                Some(&conflicting_min),
                Some(&conflicting_max),
                None,
                None
            ),
            (900, 700)
        );
    }

    #[test]
    fn dynamic_size_constraints_override_static_sources_per_axis_pair() {
        let mut executor = test_executor();
        let handle = WindowHandle(42);
        executor.window_state.get_mut(handle).min_size.static_value =
            Some((RuleMeasure::Pixels(300), RuleMeasure::Pixels(200)));
        assert_eq!(
            executor
                .window_state
                .get(handle)
                .min_size
                .effective()
                .cloned(),
            Some((RuleMeasure::Pixels(300), RuleMeasure::Pixels(200)))
        );
        executor.window_state.get_mut(handle).min_size.dynamic_value =
            Some((RuleMeasure::Pixels(500), RuleMeasure::Pixels(400)));
        assert_eq!(
            executor
                .window_state
                .get(handle)
                .min_size
                .effective()
                .cloned(),
            Some((RuleMeasure::Pixels(500), RuleMeasure::Pixels(400)))
        );
    }

    #[test]
    fn aspect_ratio_uses_dominant_axis_and_preserves_bounds() {
        let work_area = crate::dwm::Rect::new(0, 0, 1920, 1080);
        let min_size = (RuleMeasure::Pixels(320), RuleMeasure::Pixels(180));
        let max_size = (RuleMeasure::Pixels(1280), RuleMeasure::Pixels(720));
        let ratio = Some((16, 9));

        assert_eq!(
            constrain_rule_size(
                1000,
                400,
                work_area,
                Some(&min_size),
                Some(&max_size),
                ratio,
                Some((800, 450))
            ),
            (1000, 563)
        );
        assert_eq!(
            constrain_rule_size(
                900,
                675,
                work_area,
                Some(&min_size),
                Some(&max_size),
                ratio,
                Some((900, 506))
            ),
            (1200, 675)
        );
        assert_eq!(
            constrain_rule_size(
                1800,
                1000,
                work_area,
                Some(&min_size),
                Some(&max_size),
                ratio,
                Some((1200, 675))
            ),
            (1280, 720)
        );
    }

    #[test]
    fn dynamic_aspect_setting_overrides_static_source() {
        let mut executor = test_executor();
        let handle = WindowHandle(42);
        executor
            .window_state
            .get_mut(handle)
            .keep_aspect_ratio
            .static_value = Some(true);
        assert!(executor
            .window_state
            .get(handle)
            .keep_aspect_ratio
            .enabled());
        executor
            .window_state
            .get_mut(handle)
            .keep_aspect_ratio
            .dynamic_value = Some(false);
        assert!(!executor
            .window_state
            .get(handle)
            .keep_aspect_ratio
            .enabled());
    }

    #[test]
    fn directional_scoring_rejects_wrong_side_and_prefers_alignment() {
        let current = crate::dwm::Rect::new(100, 100, 100, 100);
        let aligned_right = crate::dwm::Rect::new(250, 100, 100, 100);
        let diagonal_right = crate::dwm::Rect::new(220, 300, 100, 100);
        let left = crate::dwm::Rect::new(-50, 100, 100, 100);

        let aligned =
            Executor::directional_score(current, aligned_right, Direction::Right).unwrap();
        let diagonal =
            Executor::directional_score(current, diagonal_right, Direction::Right).unwrap();
        assert!(aligned < diagonal);
        assert!(Executor::directional_score(current, left, Direction::Right).is_none());
    }

    #[test]
    fn rule_geometry_resolves_percentages_centers_and_restores_independent_axes() {
        let current = crate::dwm::Rect::new(20, 30, 300, 200);
        let work_area = crate::dwm::Rect::new(100, 50, 1600, 900);
        let size = (RuleMeasure::Percent(50), RuleMeasure::Percent(40));
        let centered = resolve_rule_rect(
            current,
            work_area,
            Some(&RulePosition::Centered),
            Some(&size),
            None,
            None,
        );
        assert_eq!(centered, crate::dwm::Rect::new(500, 320, 800, 360));

        let moved = RulePosition::Coordinates {
            x: RuleMeasure::Percent(75),
            y: RuleMeasure::Pixels(25),
        };
        let positioned = resolve_rule_rect(
            current,
            work_area,
            Some(&moved),
            None,
            None,
            Some((640, 480)),
        );
        assert_eq!(positioned, crate::dwm::Rect::new(1300, 75, 640, 480));

        let restored = resolve_rule_rect(
            current,
            work_area,
            None,
            None,
            Some((10, 15)),
            Some((700, 500)),
        );
        assert_eq!(restored, crate::dwm::Rect::new(10, 15, 700, 500));
    }

    /// Run one action node, triggered manually, on a two-monitor desktop and
    /// return the primary monitor's visible workspace afterwards.
    fn workspace_after_node(node_type: NodeType) -> crate::dwm::Workspace {
        let mut graph = NodeGraph::new();
        let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
        let node = graph.add_node(node_type, [100.0, 0.0]);
        let exec_input = graph.nodes[&node].inputs[0].name.clone();
        graph
            .add_connection_by_name(trigger, "Trigger", node, &exec_input)
            .expect("the trigger should connect to the node");
        let mut executor = test_executor();
        executor.set_graph(graph);
        let mut window_manager = WindowManager::new();
        let (mut monitor_manager, primary, _) = monitor_pair();
        executor
            .execute_manual_target(
                &mut window_manager,
                &mut monitor_manager,
                &trigger.to_string(),
            )
            .expect("the graph should run");
        monitor_manager
            .get_focused_workspace(primary)
            .expect("the primary monitor has a workspace")
            .clone()
    }

    #[test]
    fn layout_nodes_change_the_visible_workspace() {
        use crate::dwm::layouts::LayoutType;
        let close = |left: f32, right: f32| (left - right).abs() < 1e-6;

        let ws = workspace_after_node(NodeType::SetLayoutRatio { ratio: 0.7 });
        assert!(close(ws.master_ratio, 0.7), "{}", ws.master_ratio);
        let ws = workspace_after_node(NodeType::IncrementLayoutRatio { step: 0.1 });
        assert!(close(ws.master_ratio, 0.6), "{}", ws.master_ratio);
        let ws = workspace_after_node(NodeType::DecrementLayoutRatio { step: 0.1 });
        assert!(close(ws.master_ratio, 0.4), "{}", ws.master_ratio);

        assert_eq!(
            workspace_after_node(NodeType::SetMasterCount { count: 3 }).master_count,
            3
        );
        assert_eq!(
            workspace_after_node(NodeType::IncrementMasterCount).master_count,
            2
        );
        assert_eq!(
            workspace_after_node(NodeType::DecrementMasterCount).master_count,
            1
        );

        // The "layout gap" node sets the padding around the workspace.
        let ws = workspace_after_node(NodeType::SetLayoutGap { gap: 30 });
        assert_eq!(ws.padding, crate::dwm::Padding::uniform(30));
        assert_eq!(ws.container_padding, crate::dwm::Padding::uniform(5));

        assert_eq!(
            workspace_after_node(NodeType::RotateLayout).layout_rotation,
            1
        );
        assert_eq!(
            workspace_after_node(NodeType::CycleNextLayout).layout,
            LayoutType::Grid
        );
        // Back from the first layout is the last of them, whichever that
        // is (it was Dwindle until Floating was added after it).
        assert_eq!(
            Some(workspace_after_node(NodeType::CyclePreviousLayout).layout),
            LayoutType::all().last().copied()
        );
    }

    /// Run one node on a primary monitor with workspaces main, two, three
    /// (starting on `start`, with one window on `main` and one on `two`)
    /// and return the monitor manager afterwards.
    fn workspaces_after_node(node_type: NodeType, start: usize) -> (MonitorManager, MonitorHandle) {
        let mut graph = NodeGraph::new();
        let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
        let node = graph.add_node(node_type, [100.0, 0.0]);
        let exec_input = graph.nodes[&node].inputs[0].name.clone();
        graph
            .add_connection_by_name(trigger, "Trigger", node, &exec_input)
            .expect("the trigger should connect to the node");
        let mut executor = test_executor();
        executor.set_graph(graph);
        let mut window_manager = WindowManager::new();
        let (mut monitor_manager, primary, _) = monitor_pair();
        for name in ["two", "three"] {
            monitor_manager.create_workspace(
                primary,
                name.to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            );
        }
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(101));
        monitor_manager
            .get_workspaces_mut(primary)
            .and_then(|workspaces| workspaces.get_mut(1))
            .expect("workspace two exists")
            .add_window(WindowHandle(102), true);
        monitor_manager.focus_workspace_at(primary, start);
        executor.set_focused_monitor_for_tests(primary);
        executor
            .execute_manual_target(
                &mut window_manager,
                &mut monitor_manager,
                &trigger.to_string(),
            )
            .expect("the graph should run");
        (monitor_manager, primary)
    }

    fn visible_workspace(node_type: NodeType, start: usize) -> usize {
        let (monitor_manager, primary) = workspaces_after_node(node_type, start);
        monitor_manager
            .get_workspaces(primary)
            .expect("the primary monitor has workspaces")
            .focused_idx()
    }

    #[test]
    fn workspace_nodes_switch_the_visible_workspace() {
        assert_eq!(
            visible_workspace(NodeType::FocusWorkspace { workspace_index: 1 }, 0),
            1
        );
        assert_eq!(
            visible_workspace(NodeType::SwitchToWorkspace { workspace_index: 2 }, 0),
            2
        );
        // A workspace that does not exist is not created by a node.
        let (monitor_manager, primary) =
            workspaces_after_node(NodeType::FocusWorkspace { workspace_index: 9 }, 0);
        let workspaces = monitor_manager.get_workspaces(primary).unwrap();
        assert_eq!((workspaces.focused_idx(), workspaces.len()), (0, 3));

        assert_eq!(visible_workspace(NodeType::NextWorkspace, 0), 1);
        assert_eq!(visible_workspace(NodeType::NextWorkspace, 2), 0, "wraps");
        assert_eq!(
            visible_workspace(NodeType::PreviousWorkspace, 0),
            2,
            "wraps"
        );
        assert_eq!(visible_workspace(NodeType::PreviousWorkspace, 2), 1);
    }

    #[test]
    fn numeric_manual_target_executes_connected_flow() {
        let mut graph = NodeGraph::new();
        let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
        let log_node = graph.add_node(
            NodeType::LogMessage {
                message: "manual-flow-ran".to_string(),
                level: crate::node_editor::LogLevel::Info,
            },
            [100.0, 0.0],
        );
        graph
            .add_connection_by_name(trigger, "Trigger", log_node, "Trigger")
            .expect("manual trigger should connect to log node");

        let log_entries = Arc::new(Mutex::new(Vec::new()));
        let rules = Arc::new(Mutex::new(RuleEngine::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = Arc::clone(&events);
        let event_callback: Arc<dyn Fn(NodeExecutionEvent) + Send + Sync> =
            Arc::new(move |event| {
                event_sink
                    .lock()
                    .expect("event mutex should not be poisoned")
                    .push(event);
            });
        let mut executor = Executor::new_with_events(
            graph,
            Arc::clone(&log_entries),
            None,
            Some(event_callback),
            rules,
        );
        let mut window_manager = WindowManager::new();
        let mut monitor_manager = MonitorManager::with_workspace_count(1);

        executor
            .execute_manual_target(
                &mut window_manager,
                &mut monitor_manager,
                &trigger.to_string(),
            )
            .expect("manual flow should execute");

        let logs = log_entries
            .lock()
            .expect("log mutex should not be poisoned");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "manual-flow-ran");

        let events = events.lock().expect("event mutex should not be poisoned");
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[0],
            NodeExecutionEvent::Started { node_id, .. } if *node_id == trigger
        ));
        assert!(matches!(
            &events[1],
            NodeExecutionEvent::Started { node_id, .. } if *node_id == log_node
        ));
        assert!(matches!(
            &events[2],
            NodeExecutionEvent::Completed { node_id, .. } if *node_id == log_node
        ));
        assert!(matches!(
            &events[3],
            NodeExecutionEvent::Completed { node_id, .. } if *node_id == trigger
        ));
    }

    #[test]
    fn numeric_manual_target_rejects_non_trigger_nodes() {
        let mut graph = NodeGraph::new();
        let node = graph.add_node(NodeType::NumberValue { value: 1.0 }, [0.0, 0.0]);
        let log_entries = Arc::new(Mutex::new(Vec::new()));
        let rules = Arc::new(Mutex::new(RuleEngine::new()));
        let mut executor = Executor::new(graph, log_entries, rules);
        let mut window_manager = WindowManager::new();
        let mut monitor_manager = MonitorManager::with_workspace_count(1);

        let error = executor
            .execute_manual_target(&mut window_manager, &mut monitor_manager, &node.to_string())
            .expect_err("non-trigger node must be rejected");

        assert!(error.to_string().contains("not a Manual Trigger"));
    }

    #[test]
    fn workspace_rename_requires_existing_unique_target_and_emits_event() {
        let monitor = MonitorHandle(77);
        let mut monitor_manager = MonitorManager::with_workspace_count(0);
        monitor_manager.add_test_monitor(monitor, "Test monitor");
        monitor_manager
            .create_workspace(
                monitor,
                "main".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        monitor_manager
            .create_workspace(
                monitor,
                "web".to_string(),
                crate::dwm::layouts::LayoutType::Columns,
            )
            .unwrap();

        let events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = Arc::clone(&events);
        let mut executor = Executor::new_with_events(
            NodeGraph::new(),
            Arc::new(Mutex::new(Vec::new())),
            None,
            Some(Arc::new(move |event| {
                event_sink.lock().unwrap().push(event);
            })),
            Arc::new(Mutex::new(RuleEngine::new())),
        );
        executor.focused_monitor = Some(monitor);
        let mut window_manager = WindowManager::new();

        executor
            .rename_workspace(
                &mut window_manager,
                &mut monitor_manager,
                WorkspaceSelector::Name("web".to_string()),
                "research".to_string(),
            )
            .unwrap();
        assert_eq!(
            monitor_manager
                .get_workspaces(monitor)
                .unwrap()
                .get(1)
                .map(|workspace| (workspace.name.as_str(), workspace.label())),
            // (What it is known by to keys and rules stays; it is shown
            // by its title.)
            Some(("web", "research"))
        );
        let events = events.lock().unwrap();
        assert!(matches!(
            events.last(),
            Some(NodeExecutionEvent::Runtime(RuntimeExtensionEvent {
                kind: RuntimeEventKind::WorkspaceRenamed,
                fields,
                ..
            })) if fields.get("old_name").map(String::as_str) == Some("web")
                && fields.get("new_name").map(String::as_str) == Some("research")
        ));
        drop(events);

        let error = executor
            .rename_workspace(
                &mut window_manager,
                &mut monitor_manager,
                WorkspaceSelector::Name("research".to_string()),
                "main".to_string(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert!(executor
            .rename_workspace(
                &mut window_manager,
                &mut monitor_manager,
                WorkspaceSelector::Name("missing".to_string()),
                "new".to_string(),
            )
            .is_err());
    }

    #[test]
    fn named_non_persistent_workspace_is_lazy_profiled_and_pruned_when_left_empty() {
        let monitor = MonitorHandle(99);
        let mut monitor_manager = MonitorManager::with_workspace_count(0);
        monitor_manager.add_test_monitor(monitor, "Test monitor");
        let base = monitor_manager
            .create_workspace(
                monitor,
                "Main".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        assert_eq!(base, 0);

        let mut executor = test_executor();
        executor.set_workspace_profiles(
            vec![WorkspaceProfile {
                name: "Notes".to_string(),
                persistent: false,
                layout: crate::dwm::layouts::LayoutType::Monocle,
                gaps_in: 17,
                gaps_out: 23,
                ..WorkspaceProfile::default()
            }],
            AppearanceProfile::default(),
        );

        let notes = executor
            .resolve_workspace_selector(
                &mut monitor_manager,
                monitor,
                &WorkspaceSelector::Name("notes".to_string()),
                false,
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(monitor)
            .unwrap()
            .get(notes)
            .unwrap();
        assert_eq!(workspace.name, "Notes");
        assert_eq!(workspace.layout, crate::dwm::layouts::LayoutType::Monocle);
        assert_eq!(workspace.container_padding.left, 17);
        assert_eq!(workspace.padding.left, 23);

        // (One of the configuration's comes before one it does not have.)
        let base = monitor_manager
            .get_workspaces(monitor)
            .unwrap()
            .iter()
            .position(|workspace| workspace.name == "Main")
            .unwrap();
        monitor_manager.focus_workspace_at(monitor, notes);
        monitor_manager.focus_workspace_at(monitor, base);
        executor.prune_non_persistent_workspace(&mut monitor_manager, monitor, notes);
        assert!(monitor_manager
            .get_workspaces(monitor)
            .unwrap()
            .elements()
            .iter()
            .all(|workspace| !workspace.name.eq_ignore_ascii_case("notes")));
    }

    fn monitor_pair() -> (MonitorManager, MonitorHandle, MonitorHandle) {
        let mut monitor_manager = MonitorManager::with_workspace_count(0);
        let primary = MonitorHandle(11);
        let secondary = MonitorHandle(22);
        monitor_manager.add_test_monitor(primary, "Primary");
        monitor_manager.add_test_monitor(secondary, "Secondary");
        for monitor in [primary, secondary] {
            monitor_manager
                .create_workspace(
                    monitor,
                    "main".to_string(),
                    crate::dwm::layouts::LayoutType::BSP,
                )
                .unwrap();
        }
        (monitor_manager, primary, secondary)
    }

    fn add_tiled_window(
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
        window: WindowHandle,
    ) {
        monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(0))
            .expect("test monitor should have a workspace")
            .add_window(window, true);
    }

    fn first_container_rect(
        monitor_manager: &MonitorManager,
        monitor: MonitorHandle,
    ) -> crate::dwm::Rect {
        monitor_manager
            .get_workspaces(monitor)
            .and_then(|workspaces| workspaces.get(0))
            .and_then(|workspace| workspace.containers.get(0))
            .map(|container| container.rect)
            .expect("test workspace should have a container")
    }

    fn zero_first_container_rect(monitor_manager: &mut MonitorManager, monitor: MonitorHandle) {
        if let Some(container) = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(0))
            .and_then(|workspace| workspace.containers.get_mut(0))
        {
            container.rect = crate::dwm::Rect::default();
        }
    }

    #[test]
    fn move_window_to_monitor_moves_membership_silently_and_with_follow() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let window = WindowHandle(7);
        add_tiled_window(&mut monitor_manager, primary, window);
        executor.focused_monitor = Some(primary);

        // Silent move: membership changes, logical monitor focus stays put.
        executor
            .move_window_to_monitor(
                &mut window_manager,
                &mut monitor_manager,
                window,
                MonitorTarget::Next,
                false,
            )
            .unwrap();
        assert_eq!(monitor_manager.find_window(window), Some((secondary, 0)));
        assert_eq!(executor.focused_monitor, Some(primary));
        assert!(!monitor_manager.is_window_floating(window));

        // Follow move: logical monitor focus transfers to the destination.
        executor.focused_monitor = Some(secondary);
        executor
            .move_window_to_monitor(
                &mut window_manager,
                &mut monitor_manager,
                window,
                MonitorTarget::Index(0),
                true,
            )
            .unwrap();
        assert_eq!(monitor_manager.find_window(window), Some((primary, 0)));
        assert_eq!(executor.focused_monitor, Some(primary));

        // Moving to the monitor the window is already on is a no-op.
        executor
            .move_window_to_monitor(
                &mut window_manager,
                &mut monitor_manager,
                window,
                MonitorTarget::Index(0),
                true,
            )
            .unwrap();
        assert_eq!(monitor_manager.find_window(window), Some((primary, 0)));

        // Floating windows stay floating across monitors.
        monitor_manager.find_and_remove_window(window);
        monitor_manager
            .get_workspaces_mut(primary)
            .and_then(|workspaces| workspaces.get_mut(0))
            .unwrap()
            .add_floating_window(window);
        executor
            .move_window_to_monitor(
                &mut window_manager,
                &mut monitor_manager,
                window,
                MonitorTarget::Index(1),
                false,
            )
            .unwrap();
        assert_eq!(monitor_manager.find_window(window), Some((secondary, 0)));
        assert!(monitor_manager.is_window_floating(window));
    }

    #[test]
    fn toggle_tiling_suspends_layout_until_retile_or_resume() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        executor.focused_monitor = Some(primary);

        // Baseline: a dirty flush applies the layout to container rects.
        executor.mark_layout_dirty(primary);
        executor
            .flush_pending_layouts(&mut window_manager, &mut monitor_manager)
            .unwrap();
        assert!(first_container_rect(&monitor_manager, primary).width > 0);

        // Suspend globally: flushes leave container rects untouched.
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleTiling,
            )
            .unwrap();
        assert!(executor.tiling_suspended());
        zero_first_container_rect(&mut monitor_manager, primary);
        executor.mark_layout_dirty(primary);
        executor
            .flush_pending_layouts(&mut window_manager, &mut monitor_manager)
            .unwrap();
        assert_eq!(first_container_rect(&monitor_manager, primary).width, 0);

        // Retile forces the layout even while suspended, without resuming.
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::Retile,
            )
            .unwrap();
        assert!(executor.tiling_suspended());
        assert!(first_container_rect(&monitor_manager, primary).width > 0);

        // Resuming re-tiles immediately.
        zero_first_container_rect(&mut monitor_manager, primary);
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleTiling,
            )
            .unwrap();
        assert!(!executor.tiling_suspended());
        assert!(first_container_rect(&monitor_manager, primary).width > 0);
    }

    #[test]
    fn toggle_workspace_tiling_freezes_and_resumes_one_workspace() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        add_tiled_window(&mut monitor_manager, secondary, WindowHandle(8));
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleWorkspaceTiling {
                    selector: WorkspaceSelector::Index(0),
                },
            )
            .unwrap();
        assert!(
            !monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap()
                .tiling_enabled
        );

        // The disabled workspace is frozen; other monitors keep tiling.
        zero_first_container_rect(&mut monitor_manager, primary);
        executor.mark_layout_dirty(primary);
        executor.mark_layout_dirty(secondary);
        executor
            .flush_pending_layouts(&mut window_manager, &mut monitor_manager)
            .unwrap();
        assert_eq!(first_container_rect(&monitor_manager, primary).width, 0);
        assert!(first_container_rect(&monitor_manager, secondary).width > 0);

        // Re-enabling applies the layout immediately.
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleWorkspaceTiling {
                    selector: WorkspaceSelector::Index(0),
                },
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert!(workspace.tiling_enabled);
        assert!(first_container_rect(&monitor_manager, primary).width > 0);
    }

    #[test]
    fn toggle_workspace_float_override_floats_and_retiles_through_dispatch() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(8));
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleWorkspaceFloatOverride {
                    selector: WorkspaceSelector::Index(0),
                },
            )
            .unwrap();
        {
            let workspace = monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap();
            assert!(workspace.float_override);
            assert_eq!(workspace.container_count(), 0);
            assert_eq!(workspace.floating_windows.len(), 2);
        }

        // Windows arriving while the override is active float as well.
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(9));
        assert!(monitor_manager.is_window_floating(WindowHandle(9)));

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleWorkspaceFloatOverride {
                    selector: WorkspaceSelector::Index(0),
                },
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert!(!workspace.float_override);
        assert_eq!(workspace.floating_windows.len(), 0);
        for window in [WindowHandle(7), WindowHandle(8), WindowHandle(9)] {
            assert!(workspace.find_container(window).is_some());
        }
    }

    #[test]
    fn adopt_window_by_default_manages_unmanaged_windows_once() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);

        // A window the graph never routed is adopted into the active workspace.
        // (The Created-event path marks the window known before adoption, so
        // initial rule evaluation has already happened at this point.)
        executor.window_state.get_mut(WindowHandle(42)).known = true;
        assert!(executor
            .adopt_window_by_default(&mut window_manager, &mut monitor_manager, WindowHandle(42))
            .unwrap());
        assert_eq!(
            monitor_manager.find_window(WindowHandle(42)),
            Some((primary, 0))
        );

        // Already-managed windows are left alone.
        assert!(!executor
            .adopt_window_by_default(&mut window_manager, &mut monitor_manager, WindowHandle(42))
            .unwrap());

        // Session-unmanaged windows stay unmanaged until `manage`.
        monitor_manager.find_and_remove_window(WindowHandle(42));
        executor
            .window_state
            .get_mut(WindowHandle(42))
            .session_unmanaged = true;
        assert!(!executor
            .adopt_window_by_default(&mut window_manager, &mut monitor_manager, WindowHandle(42))
            .unwrap());
        assert_eq!(monitor_manager.find_window(WindowHandle(42)), None);
    }

    #[test]
    fn focus_monitor_workspace_switches_only_the_target_monitor() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        monitor_manager
            .create_workspace(
                secondary,
                "web".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::FocusMonitorWorkspace {
                    target: MonitorTarget::Index(1),
                    workspace: WorkspaceSelector::Index(1),
                },
            )
            .unwrap();

        assert_eq!(executor.focused_monitor, Some(secondary));
        assert_eq!(
            monitor_manager
                .get_workspaces(secondary)
                .unwrap()
                .focused_idx(),
            1
        );
        // The other monitor's active workspace is untouched.
        assert_eq!(
            monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .focused_idx(),
            0
        );

        // Relative workspace selectors resolve on the target monitor's ring.
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::FocusMonitorWorkspace {
                    target: MonitorTarget::Index(0),
                    workspace: WorkspaceSelector::Index(0),
                },
            )
            .unwrap();
        assert_eq!(executor.focused_monitor, Some(primary));
        assert_eq!(
            monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .focused_idx(),
            0
        );
    }

    #[test]
    fn a_workspace_asked_for_by_name_is_gone_to_on_the_monitor_that_has_it() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        monitor_manager
            .create_workspace(
                secondary,
                "web".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        executor.focused_monitor = Some(primary);
        let count = |monitor_manager: &MonitorManager, monitor| {
            monitor_manager.get_workspaces(monitor).unwrap().len()
        };
        let mut switch =
            |executor: &mut Executor, monitor_manager: &mut MonitorManager, name: &str| {
                executor
                    .execute_dispatch(
                        &mut window_manager,
                        monitor_manager,
                        &mut snap_zone_manager,
                        &mut focus_manager,
                        DispatchCommand::SwitchWorkspaceSelector {
                            selector: WorkspaceSelector::Name(name.to_string()),
                        },
                    )
                    .unwrap();
            };

        // Only the other monitor has "web": it is shown there, and no
        // second "web" is made here.
        switch(&mut executor, &mut monitor_manager, "web");
        assert_eq!(executor.focused_monitor, Some(secondary));
        assert_eq!(
            monitor_manager
                .get_workspaces(secondary)
                .unwrap()
                .focused_idx(),
            1
        );
        assert_eq!(count(&monitor_manager, primary), 1);

        // Both have "main": the one of the monitor with the focus.
        switch(&mut executor, &mut monitor_manager, "main");
        assert_eq!(executor.focused_monitor, Some(secondary));
        assert_eq!(
            monitor_manager
                .get_workspaces(secondary)
                .unwrap()
                .focused_idx(),
            0
        );

        // A name nobody has: a new workspace, on the monitor with the focus.
        switch(&mut executor, &mut monitor_manager, "mail");
        assert_eq!(executor.focused_monitor, Some(secondary));
        assert_eq!(count(&monitor_manager, primary), 1);
        assert!(monitor_manager
            .get_workspaces(secondary)
            .unwrap()
            .elements()
            .iter()
            .any(|workspace| workspace.name == "mail"));
    }

    /// A selector-targeted focus reveals the
    /// window even when it lives on an inactive workspace of another monitor.
    #[test]
    fn focus_window_reveals_windows_on_other_monitors_and_inactive_workspaces() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        monitor_manager
            .create_workspace(
                secondary,
                "web".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        let window = WindowHandle(7);
        monitor_manager
            .get_workspaces_mut(secondary)
            .and_then(|workspaces| workspaces.get_mut(1))
            .unwrap()
            .add_window(window, true);
        executor.focused_monitor = Some(primary);

        executor
            .focus_managed_window(&mut window_manager, &mut monitor_manager, window)
            .unwrap();

        assert_eq!(executor.focused_monitor, Some(secondary));
        assert_eq!(monitor_manager.find_window(window), Some((secondary, 1)));
        assert_eq!(
            monitor_manager
                .get_workspaces(secondary)
                .unwrap()
                .focused_idx(),
            1
        );
    }

    #[test]
    fn force_focus_bypasses_the_no_focus_rule_for_one_explicit_action() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let window = WindowHandle(7);
        add_tiled_window(&mut monitor_manager, primary, window);
        executor.focused_monitor = Some(primary);
        executor.window_state.get_mut(window).no_focus.static_value = Some(true);

        // Ordinary focus honors the rule; force-focus overrides it once.
        assert!(executor
            .focus_managed_window(&mut window_manager, &mut monitor_manager, window)
            .is_err());
        executor
            .focus_managed_window_with_rule_bypass(
                &mut window_manager,
                &mut monitor_manager,
                window,
                true,
            )
            .unwrap();
        // The rule keeps applying to future focus decisions.
        assert!(executor.no_focus_enabled(window));
        assert!(executor
            .focus_managed_window(&mut window_manager, &mut monitor_manager, window)
            .is_err());
    }

    #[test]
    fn new_workspace_materializes_named_and_indexed_workspaces_without_switching() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);

        executor
            .new_workspace(&mut window_manager, &mut monitor_manager, None)
            .unwrap();
        let workspaces = monitor_manager.get_workspaces(primary).unwrap();
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces.get(1).unwrap().name, "2");
        // The view stays on the original workspace.
        assert_eq!(workspaces.focused_idx(), 0);

        executor
            .new_workspace(
                &mut window_manager,
                &mut monitor_manager,
                Some("mail".to_string()),
            )
            .unwrap();
        let workspaces = monitor_manager.get_workspaces(primary).unwrap();
        assert_eq!(workspaces.len(), 3);
        assert_eq!(workspaces.get(2).unwrap().name, "mail");
        assert_eq!(workspaces.focused_idx(), 0);

        assert!(executor
            .new_workspace(
                &mut window_manager,
                &mut monitor_manager,
                Some("   ".to_string()),
            )
            .is_err());
    }

    #[test]
    fn close_workspace_removes_only_an_empty_focused_workspace() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        monitor_manager
            .create_workspace(
                primary,
                "scratch".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        executor.focused_monitor = Some(primary);
        let window = WindowHandle(7);
        add_tiled_window(&mut monitor_manager, primary, window);

        // An occupied workspace refuses loudly.
        assert!(executor
            .close_workspace(&mut window_manager, &mut monitor_manager)
            .is_err());
        assert_eq!(monitor_manager.get_workspaces(primary).unwrap().len(), 2);

        // Once empty, closing removes it and focus advances in the ring.
        monitor_manager.find_and_remove_window(window);
        executor
            .close_workspace(&mut window_manager, &mut monitor_manager)
            .unwrap();
        let workspaces = monitor_manager.get_workspaces(primary).unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces.focused().unwrap().name, "scratch");

        // The last workspace on a monitor cannot be closed.
        assert!(executor
            .close_workspace(&mut window_manager, &mut monitor_manager)
            .is_err());
        assert_eq!(monitor_manager.get_workspaces(primary).unwrap().len(), 1);
    }

    #[test]
    fn ensure_workspaces_creates_only_the_missing_ones() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);

        executor
            .ensure_workspaces(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Index(0),
                3,
            )
            .unwrap();
        let workspaces = monitor_manager.get_workspaces(primary).unwrap();
        assert_eq!(workspaces.len(), 3);
        assert_eq!(workspaces.get(1).unwrap().name, "2");
        assert_eq!(workspaces.get(2).unwrap().name, "3");
        // The other monitor is untouched; rerunning is a no-op.
        assert_eq!(monitor_manager.get_workspaces(secondary).unwrap().len(), 1);
        executor
            .ensure_workspaces(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                3,
            )
            .unwrap();
        assert_eq!(monitor_manager.get_workspaces(primary).unwrap().len(), 3);

        // Invalid counts fail loudly.
        assert!(executor
            .ensure_workspaces(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                0,
            )
            .is_err());
        assert!(executor
            .ensure_workspaces(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                129,
            )
            .is_err());
    }

    #[test]
    fn ensure_named_workspace_is_idempotent_per_monitor() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);

        executor
            .ensure_named_workspace(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                "chat".to_string(),
                None,
            )
            .unwrap();
        assert_eq!(monitor_manager.get_workspaces(primary).unwrap().len(), 2);
        // A second ensure is a no-op (case-insensitive).
        executor
            .ensure_named_workspace(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                "CHAT".to_string(),
                None,
            )
            .unwrap();
        assert_eq!(monitor_manager.get_workspaces(primary).unwrap().len(), 2);
        use crate::dwm::layouts::LayoutType;
        // With a layout, the workspace that is there has it from now on,
        // whichever workspace is shown; and one made with a layout has it.
        let layout_of = |manager: &MonitorManager, name: &str| {
            manager
                .get_workspaces(primary)
                .unwrap()
                .iter()
                .find(|workspace| workspace.name == name)
                .map(|workspace| workspace.layout)
        };
        executor
            .ensure_named_workspace(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                "chat".to_string(),
                Some(LayoutType::Floating),
            )
            .unwrap();
        assert_eq!(
            layout_of(&monitor_manager, "chat"),
            Some(LayoutType::Floating)
        );
        executor
            .ensure_named_workspace(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                "notes".to_string(),
                Some(LayoutType::Columns),
            )
            .unwrap();
        assert_eq!(
            layout_of(&monitor_manager, "notes"),
            Some(LayoutType::Columns)
        );
        assert_eq!(monitor_manager.get_workspaces(primary).unwrap().len(), 3);
        // The same name on another monitor is a separate workspace.
        executor
            .ensure_named_workspace(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Index(1),
                "chat".to_string(),
                None,
            )
            .unwrap();
        assert_eq!(monitor_manager.get_workspaces(secondary).unwrap().len(), 2);
        assert!(executor
            .ensure_named_workspace(
                &mut window_manager,
                &mut monitor_manager,
                MonitorTarget::Primary,
                "  ".to_string(),
                None,
            )
            .is_err());
    }

    #[test]
    fn container_behaviour_toggle_stacks_new_windows_through_dispatch() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleWindowContainerBehaviour,
            )
            .unwrap();
        assert_eq!(
            monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap()
                .window_container_behaviour,
            crate::dwm::WindowContainerBehaviour::Append
        );

        // Append: a newly added window joins the focused container.
        monitor_manager
            .get_workspaces_mut(primary)
            .and_then(|workspaces| workspaces.get_mut(0))
            .unwrap()
            .add_window(WindowHandle(8), true);
        assert_eq!(
            monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap()
                .container_count(),
            1
        );

        // The selector form toggles it back to Column.
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleWorkspaceWindowContainerBehaviour {
                    selector: WorkspaceSelector::Index(0),
                },
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces_mut(primary)
            .and_then(|workspaces| workspaces.get_mut(0))
            .unwrap();
        assert_eq!(
            workspace.window_container_behaviour,
            crate::dwm::WindowContainerBehaviour::Column
        );
        workspace.add_window(WindowHandle(9), true);
        assert_eq!(workspace.container_count(), 2);
    }

    #[test]
    fn unmanage_releases_and_manage_readopts_a_window() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(8));
        executor.focused_monitor = Some(primary);
        // A window adopted at creation time is already known; unmanage does
        // not forget it, so re-managing skips initial rule evaluation.
        executor.window_state.get_mut(WindowHandle(7)).known = true;

        executor
            .unmanage_window(&mut window_manager, &mut monitor_manager, WindowHandle(7))
            .unwrap();
        assert_eq!(monitor_manager.find_window(WindowHandle(7)), None);
        assert!(executor.window_state.get(WindowHandle(7)).session_unmanaged);
        // The remaining window still tiles; the unmanaged one is gone from layout.
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.container_count(), 1);
        assert!(workspace.find_container(WindowHandle(8)).is_some());

        // Unmanaging twice or unmanaging an unknown window fails loudly.
        assert!(executor
            .unmanage_window(&mut window_manager, &mut monitor_manager, WindowHandle(7))
            .is_err());
        assert!(executor
            .unmanage_window(&mut window_manager, &mut monitor_manager, WindowHandle(99))
            .is_err());

        executor
            .manage_window(&mut window_manager, &mut monitor_manager, WindowHandle(7))
            .unwrap();
        assert_eq!(
            monitor_manager.find_window(WindowHandle(7)),
            Some((primary, 0))
        );
        assert!(!executor.window_state.get(WindowHandle(7)).session_unmanaged);
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.container_count(), 2);

        // Managing an already-managed window fails loudly.
        assert!(executor
            .manage_window(&mut window_manager, &mut monitor_manager, WindowHandle(7))
            .is_err());
    }

    /// A real, never-shown top-level window: alive for `IsWindow`, absent
    /// from the visible-only window snapshot.
    struct HiddenTestWindow(WindowHandle);

    impl HiddenTestWindow {
        fn create() -> Self {
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, WINDOW_EX_STYLE, WINDOW_STYLE,
            };
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    windows::core::w!("STATIC"),
                    windows::core::w!("gevrek hidden test window"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("create hidden test window");
            Self(WindowHandle(hwnd.0 as isize))
        }
    }

    impl Drop for HiddenTestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0.to_hwnd());
            }
        }
    }

    fn send(
        executor: &mut Executor,
        window_manager: &mut WindowManager,
        monitor_manager: &mut MonitorManager,
        event: WindowEvent,
    ) {
        let mut snap_zone_manager = SnapZoneManager::new(Default::default());
        let mut focus_manager = FocusManager::new();
        executor
            .execute_window_event(
                window_manager,
                monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                event,
            )
            .unwrap();
    }

    #[test]
    fn a_minimized_tiled_window_leaves_the_layout_and_returns_on_restore() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);
        let window = HiddenTestWindow::create();
        let neighbour = HiddenTestWindow::create();
        for handle in [window.0, neighbour.0] {
            monitor_manager
                .get_focused_workspace_mut(primary)
                .unwrap()
                .add_window(handle, true);
        }

        send(
            &mut executor,
            &mut window_manager,
            &mut monitor_manager,
            WindowEvent::Minimized(window.0),
        );
        assert_eq!(
            monitor_manager.find_window(window.0),
            None,
            "still in the layout"
        );
        assert_eq!(monitor_manager.find_window(neighbour.0), Some((primary, 0)));
        assert!(executor.pending_layout_monitors.contains(&primary));

        send(
            &mut executor,
            &mut window_manager,
            &mut monitor_manager,
            WindowEvent::Restored(window.0),
        );
        assert_eq!(monitor_manager.find_window(window.0), Some((primary, 0)));
        assert!(executor
            .window_state
            .ids_where(|state| state.minimized_origin.is_some())
            .is_empty());
    }

    #[test]
    fn a_window_restored_after_its_workspace_was_switched_away_joins_the_visible_one() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        monitor_manager
            .create_workspace(
                primary,
                "other".to_string(),
                crate::dwm::layouts::LayoutType::BSP,
            )
            .unwrap();
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);
        let window = HiddenTestWindow::create();
        monitor_manager
            .get_focused_workspace_mut(primary)
            .unwrap()
            .add_window(window.0, true);

        send(
            &mut executor,
            &mut window_manager,
            &mut monitor_manager,
            WindowEvent::Minimized(window.0),
        );
        monitor_manager.focus_workspace_at(primary, 1);
        send(
            &mut executor,
            &mut window_manager,
            &mut monitor_manager,
            WindowEvent::Restored(window.0),
        );
        // Never put back onto a hidden workspace, which would hide it again.
        assert_eq!(monitor_manager.find_window(window.0), Some((primary, 1)));
    }

    #[test]
    fn monitor_changes_carry_executor_state_to_the_new_handle() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, secondary) = monitor_pair();
        let replacement = MonitorHandle(99);
        monitor_manager.add_test_monitor(replacement, "Primary again");
        let mut window_manager = WindowManager::new();
        executor.focused_monitor = Some(primary);
        executor.locked_groups.insert((primary, 0, 1));
        let returning = HiddenTestWindow::create();

        let changes = crate::dwm::MonitorChanges {
            remapped: vec![(primary, replacement)],
            windows_to_hide: vec![returning.0],
            ..Default::default()
        };
        executor.apply_monitor_changes(&mut window_manager, &monitor_manager, &changes);

        assert_eq!(executor.focused_monitor, Some(replacement));
        assert!(executor.locked_groups.contains(&(replacement, 0, 1)));
        // A window sent back to a hidden workspace is tracked as WM-hidden.
        assert!(executor.window_state.get(returning.0).hidden_by_wm);
        assert!(crate::dwm::hidden_journal::is_hidden(returning.0));
        // Every attached monitor is laid out again.
        assert!(executor.pending_layout_monitors.contains(&secondary));
        assert!(executor.pending_layout_monitors.contains(&replacement));
        let _ = window_manager.show_window(returning.0);
    }

    #[test]
    fn an_app_hidden_window_leaves_the_layout_but_wm_hides_do_not() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::new(Default::default());
        let mut focus_manager = FocusManager::new();
        let tray_app = HiddenTestWindow::create();
        let other_workspace = HiddenTestWindow::create();
        for window in [tray_app.0, other_workspace.0] {
            monitor_manager
                .get_focused_workspace_mut(primary)
                .unwrap()
                .add_window(window, true);
            executor.window_state.get_mut(window).known = true;
        }
        // The WM hid this one itself (workspace switch).
        window_manager.hide_window(other_workspace.0).unwrap();

        for window in [tray_app.0, other_workspace.0] {
            executor
                .execute_window_event(
                    &mut window_manager,
                    &mut monitor_manager,
                    &mut snap_zone_manager,
                    &mut focus_manager,
                    WindowEvent::Unmapped(window),
                )
                .unwrap();
        }

        assert_eq!(monitor_manager.find_window(tray_app.0), None);
        assert!(!executor.window_state.get(tray_app.0).known);
        assert!(executor.pending_layout_monitors.contains(&primary));
        assert_eq!(
            monitor_manager.find_window(other_workspace.0),
            Some((primary, 0))
        );
        let _ = window_manager.show_window(other_workspace.0);
    }

    #[test]
    fn cleanup_stale_tracking_keeps_hidden_live_windows() {
        let hidden = HiddenTestWindow::create();
        let dead = WindowHandle(7);
        let mut executor = test_executor();
        let window_manager = WindowManager::new();
        assert!(!window_manager
            .get_all_windows()
            .iter()
            .any(|w| w.handle == hidden.0));

        for handle in [hidden.0, dead] {
            executor.window_state.get_mut(handle).known = true;
            executor.window_state.get_mut(handle).hidden_by_wm = true;
        }
        executor.scratchpads.bind("term", hidden.0, false);
        executor.scratchpads.bind("gone", dead, false);

        executor.cleanup_stale_tracking(&window_manager);

        assert!(executor.window_state.get(hidden.0).known);
        assert!(!executor.window_state.get(dead).known);
        assert!(executor.window_state.get(hidden.0).hidden_by_wm);
        assert!(!executor.window_state.get(dead).hidden_by_wm);
        assert!(executor.scratchpads.window("term").is_some());
        assert!(executor.scratchpads.window("gone").is_none());
    }

    #[test]
    fn restore_hidden_windows_clears_tracked_hidden_state() {
        // (It shows whatever the process's journal of hidden windows
        // names: no other test's windows may be in it meanwhile.)
        let _alone = crate::runtime::test_support::alone_with_the_journal();
        let mut executor = test_executor();
        let mut window_manager = WindowManager::new();
        // Handles no window can have (a window handle fits in 32 bits): a
        // small number is now and then a real window of the desktop the
        // tests run on, which was then shown and counted.
        let (hidden, pad) = (
            WindowHandle(0x7FFF_0000_0000_0007),
            WindowHandle(0x7FFF_0000_0000_0008),
        );
        executor.window_state.get_mut(hidden).hidden_by_wm = true;
        executor.scratchpads.bind("term", pad, false);

        // The handles name no window, so nothing is shown, but the
        // tracking state is still reconciled.
        let restored = executor.restore_hidden_windows(&mut window_manager);
        assert_eq!(restored, 0);
        assert!(executor
            .window_state
            .ids_where(|state| state.hidden_by_wm)
            .is_empty());
        assert!(executor.scratchpads.hidden_windows().is_empty());
    }

    #[test]
    fn stack_all_merges_the_workspace_and_unstack_all_restores_it() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        for window in [WindowHandle(7), WindowHandle(8), WindowHandle(9)] {
            add_tiled_window(&mut monitor_manager, primary, window);
        }
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::StackAll,
            )
            .unwrap();
        {
            let workspace = monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap();
            assert_eq!(workspace.container_count(), 1);
            let container = workspace.containers.get(0).unwrap();
            assert_eq!(container.windows.len(), 3);
            // The focused window keeps its visibility inside the group.
            assert_eq!(workspace.focused_window(), Some(WindowHandle(7)));
        }

        // Stacking an already fully stacked workspace is a no-op error.
        assert!(executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::StackAll,
            )
            .is_err());

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::UnstackAll,
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.container_count(), 3);
        for window in [WindowHandle(7), WindowHandle(8), WindowHandle(9)] {
            assert!(workspace.find_container(window).is_some());
        }

        // Unstacking a plain container fails loudly.
        assert!(executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::UnstackAll,
            )
            .is_err());
    }

    #[test]
    fn toggle_monocle_switches_and_restores_the_layout_through_dispatch() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleMonocle,
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.layout, crate::dwm::layouts::LayoutType::Monocle);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleMonocle,
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.layout, crate::dwm::layouts::LayoutType::BSP);
        assert!(workspace.monocle_restore_layout.is_none());
    }

    #[test]
    fn toggle_layout_lock_flips_the_workspace_flag() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleLayoutLock,
            )
            .unwrap();
        assert!(
            monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap()
                .layout_locked
        );

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::ToggleLayoutLock,
            )
            .unwrap();
        assert!(
            !monitor_manager
                .get_workspaces(primary)
                .unwrap()
                .get(0)
                .unwrap()
                .layout_locked
        );
    }

    #[test]
    fn padding_and_work_area_offset_dispatchers_update_the_focused_workspace() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        executor.focused_monitor = Some(primary);

        for command in [
            DispatchCommand::SetWorkspacePadding { padding: 30 },
            DispatchCommand::AdjustWorkspacePadding { delta: -10 },
            DispatchCommand::SetContainerPadding { padding: 15 },
            DispatchCommand::AdjustContainerPadding { delta: 5 },
            DispatchCommand::SetWorkspaceWorkAreaOffset {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            },
        ] {
            executor
                .execute_dispatch(
                    &mut window_manager,
                    &mut monitor_manager,
                    &mut snap_zone_manager,
                    &mut focus_manager,
                    command,
                )
                .unwrap();
        }

        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.padding.left, 20);
        assert_eq!(workspace.padding.right, 20);
        assert_eq!(workspace.container_padding.left, 20);
        assert_eq!(
            workspace.work_area_offset,
            crate::settings::ReservedArea {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            }
        );

        // Negative and oversized values clamp instead of failing.
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::AdjustWorkspacePadding { delta: -100 },
            )
            .unwrap();
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::SetWorkspaceWorkAreaOffset {
                    left: -5,
                    top: 20_000,
                    right: 0,
                    bottom: 0,
                },
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(workspace.padding.left, 0);
        assert_eq!(workspace.work_area_offset.left, 0);
        assert_eq!(workspace.work_area_offset.top, 10_000);
    }

    #[test]
    fn flip_layout_toggles_the_workspace_mirror_state() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        add_tiled_window(&mut monitor_manager, primary, WindowHandle(7));
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::FlipLayout {
                    axis: LayoutAxis::Horizontal,
                },
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert!(workspace.layout_flip_horizontal);
        assert!(!workspace.layout_flip_vertical);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::FlipLayout {
                    axis: LayoutAxis::HorizontalAndVertical,
                },
            )
            .unwrap();
        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert!(!workspace.layout_flip_horizontal);
        assert!(workspace.layout_flip_vertical);
    }

    #[test]
    fn set_resize_delta_updates_and_validates_the_session_delta() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        executor.focused_monitor = Some(primary);
        assert_eq!(executor.resize_delta(), 50);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::SetResizeDelta { pixels: 120 },
            )
            .unwrap();
        assert_eq!(executor.resize_delta(), 120);

        assert!(executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::SetResizeDelta { pixels: 0 },
            )
            .is_err());
        assert_eq!(executor.resize_delta(), 120);
    }

    #[test]
    fn save_and_load_resize_round_trips_workspace_ratios_and_delta() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        executor.focused_monitor = Some(primary);

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::SetResizeDelta { pixels: 120 },
            )
            .unwrap();
        {
            let workspace = monitor_manager
                .get_workspaces_mut(primary)
                .unwrap()
                .get_mut(0)
                .unwrap();
            workspace.layout = crate::dwm::layouts::LayoutType::MasterStack;
            workspace.master_ratio = 0.7;
            workspace.master_count = 2;
            workspace.bsp_split_ratios = vec![0.65, 0.4];
        }

        let path = std::env::temp_dir().join(format!(
            "gevrek-save-resize-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let path_string = path.to_string_lossy().to_string();
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::SaveResize {
                    path: path_string.clone(),
                },
            )
            .unwrap();

        // Mutate the live state away from the snapshot.
        {
            let workspace = monitor_manager
                .get_workspaces_mut(primary)
                .unwrap()
                .get_mut(0)
                .unwrap();
            workspace.layout = crate::dwm::layouts::LayoutType::BSP;
            workspace.master_ratio = 0.5;
            workspace.master_count = 1;
            workspace.bsp_split_ratios.clear();
        }
        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::SetResizeDelta { pixels: 60 },
            )
            .unwrap();

        executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::LoadResize { path: path_string },
            )
            .unwrap();
        let _ = std::fs::remove_file(&path);

        let workspace = monitor_manager
            .get_workspaces(primary)
            .unwrap()
            .get(0)
            .unwrap();
        assert_eq!(
            workspace.layout,
            crate::dwm::layouts::LayoutType::MasterStack
        );
        assert_eq!(workspace.master_ratio, 0.7);
        assert_eq!(workspace.master_count, 2);
        assert_eq!(workspace.bsp_split_ratios, vec![0.65, 0.4]);
        assert_eq!(executor.resize_delta(), 120);
    }

    #[test]
    fn load_resize_rejects_a_missing_file_and_out_of_range_delta() {
        let mut executor = test_executor();
        let (mut monitor_manager, primary, _secondary) = monitor_pair();
        let mut window_manager = WindowManager::new();
        let mut snap_zone_manager = SnapZoneManager::default();
        let mut focus_manager = FocusManager::new();
        executor.focused_monitor = Some(primary);

        let missing = std::env::temp_dir().join(format!(
            "gevrek-load-resize-missing-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);
        assert!(executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::LoadResize {
                    path: missing.to_string_lossy().to_string(),
                },
            )
            .is_err());

        let path = std::env::temp_dir().join(format!(
            "gevrek-load-resize-bad-delta-{}.json",
            std::process::id()
        ));
        crate::persistence::ResizeDimensionsState::new(0)
            .save(&path)
            .unwrap();
        assert!(executor
            .execute_dispatch(
                &mut window_manager,
                &mut monitor_manager,
                &mut snap_zone_manager,
                &mut focus_manager,
                DispatchCommand::LoadResize {
                    path: path.to_string_lossy().to_string(),
                },
            )
            .is_err());
        let _ = std::fs::remove_file(&path);
        assert_eq!(executor.resize_delta(), 50);
    }
}
