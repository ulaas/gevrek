//! The automation graph's interpreter: running nodes, values on pins, hotkey and manual
//! triggers, the startup run.

use super::*;

/// How many value nodes are asked one behind the other, at the most.
/// (The longest row in gevrek's own graphs is six nodes; a row is cut
/// where a debug build on a thread of the smallest kind still has room.)
const DEEPEST_VALUE: usize = 32;
/// How many nodes are run one after the other in one row, at the most.
const DEEPEST_ROW: usize = 64;
/// How many nodes are run for one thing that happened, at the most (a
/// loop runs what is behind it ten thousand times; loops that run each
/// other would not come to an end).
const MOST_NODES: usize = 250_000;

impl Executor {
    /// Swap in a new automation graph. The executor keeps every piece of
    /// per-window and per-workspace state; only graph-derived values reset.
    pub fn set_graph(&mut self, graph: NodeGraph) {
        self.graph = graph;
        self.pin_values.clear();
    }

    /// Re-run startup automation of a changed graph against the live
    /// workspaces (see `reload_previous_graph`). Returns the workspaces the
    /// previous graph created and the new one no longer does.
    pub fn rerun_startup_for_reload(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<Vec<(MonitorHandle, String)>> {
        self.reload_previous_workspaces = Some(std::mem::take(&mut self.graph_workspaces));
        let result = self.execute_startup_workspaces_only(window_manager, monitor_manager);
        let previous = self.reload_previous_workspaces.take().unwrap_or_default();
        if let Err(error) = result {
            for (key, settings) in previous {
                self.graph_workspaces.entry(key).or_insert(settings);
            }
            return Err(error);
        }
        Ok(previous
            .into_keys()
            .filter(|key| !self.graph_workspaces.contains_key(key))
            .collect())
    }

    /// Execute startup for hot-reload - creates workspaces only
    /// Skips OnStartupCompleted to avoid re-arranging existing windows during hot-reload
    pub fn execute_startup_workspaces_only(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        log::info!("Executing startup events (workspaces only, skipping OnStartupCompleted for hot-reload)");

        // Set startup phase flag
        self.is_startup_phase = true;

        // Find all OnStartup event nodes
        let mut startup_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnStartup))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        startup_nodes.sort_unstable();
        let _run = GraphRun::start("OnStartup", startup_nodes.len(), false);

        for node_id in startup_nodes {
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }

        log::info!("✅ OnStartup nodes completed (OnStartupCompleted skipped for hot-reload)");

        // Clear startup phase flag
        self.is_startup_phase = false;
        self.reevaluate_all_dynamic_rules(window_manager, monitor_manager)?;

        // Clear suppressions
        self.suppressed_windows.clear();

        Ok(())
    }

    pub fn execute_startup(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        log::info!("Executing startup events");

        // Set startup phase flag - affects monitor detection logic
        self.is_startup_phase = true;

        // Find all OnStartup event nodes
        let mut startup_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnStartup))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        startup_nodes.sort_unstable();
        let _run = GraphRun::start("OnStartup", startup_nodes.len(), false);

        for node_id in startup_nodes {
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }

        log::info!("✅ OnStartup nodes completed, triggering OnStartupCompleted nodes");

        // Find and execute all OnStartupCompleted event nodes
        let mut startup_completed_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnStartupCompleted))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        startup_completed_nodes.sort_unstable();
        let _run = GraphRun::start("OnStartupCompleted", startup_completed_nodes.len(), false);

        for node_id in startup_completed_nodes {
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }

        // Existing windows do not emit Created events when the WM starts. Adopt
        // every window the startup graph placed and run the same canonical rule
        // lifecycle used for newly created windows.
        let mut existing_windows = Vec::new();
        for (monitor, _) in monitor_manager.get_monitors_with_handles() {
            if let Some(workspaces) = monitor_manager.get_workspaces(monitor) {
                for workspace in workspaces.iter() {
                    for container in workspace.containers.iter() {
                        existing_windows.extend(container.all_windows());
                    }
                    existing_windows.extend(workspace.floating_windows.iter().copied());
                }
            }
        }
        existing_windows.sort_by_key(|handle| handle.0);
        existing_windows.dedup();
        for handle in existing_windows {
            self.window_state.get_mut(handle).known = true;
            self.apply_initial_window_rules(window_manager, monitor_manager, handle)?;
        }

        // Clear startup phase flag
        self.is_startup_phase = false;

        // Log final window distribution summary
        log::info!("═══════════════════════════════════════════════════════════");
        log::info!("📊 STARTUP SUMMARY - Window Distribution:");
        for (monitor_handle, monitor_info) in monitor_manager.get_monitors_with_handles() {
            log::info!("   Monitor '{}' ({:?}):", monitor_info.name, monitor_handle);
            log::info!(
                "      Work area: ({},{}) {}x{}",
                monitor_info.work_x,
                monitor_info.work_y,
                monitor_info.work_width,
                monitor_info.work_height
            );

            if let Some(workspaces) = monitor_manager.get_workspaces(monitor_handle) {
                for (ws_idx, workspace) in workspaces.iter().enumerate() {
                    let tiled_count = workspace.containers.len();
                    let floating_count = workspace.floating_windows.len();
                    log::info!(
                        "      Workspace '{}' [{}]: {} tiled, {} floating",
                        workspace.name,
                        ws_idx,
                        tiled_count,
                        floating_count
                    );

                    // Show each window
                    for (c_idx, container) in workspace.containers.iter().enumerate() {
                        if let Some(wh) = container.focused_window() {
                            let title = queries(window_manager)
                                .windows()
                                .iter()
                                .find(|w| w.handle == wh)
                                .map(|w| w.title.clone())
                                .unwrap_or_else(|| format!("{:?}", wh));
                            log::info!(
                                "         [{}] rect=({},{} {}x{}) '{}'",
                                c_idx,
                                container.rect.x,
                                container.rect.y,
                                container.rect.width,
                                container.rect.height,
                                title
                            );
                        }
                    }
                }
            }
        }
        log::info!("═══════════════════════════════════════════════════════════");

        // Clear suppressions after startup to allow normal event processing
        // Wait a bit for any pending events to settle
        std::thread::sleep(std::time::Duration::from_millis(500));
        self.suppressed_windows.clear();
        log::info!("Startup complete, cleared event suppressions");

        Ok(())
    }

    pub(super) fn execute_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        visited: &mut HashSet<NodeId>,
    ) -> Result<()> {
        let Some(node_type) = self
            .graph
            .nodes
            .get(&node_id)
            .map(|node| node.node_type.clone())
        else {
            return Ok(());
        };

        // A graph is a file anyone may have written. A row longer than
        // anybody draws is not run to its end (each node run is room
        // that is not given back until the row ends; and a loop begins
        // its row anew, so two loops that run each other have no end of
        // their own); neither are more nodes than any graph needs.
        if self.running_deep == 0 {
            self.nodes_run = 0;
        }
        if self.nodes_run >= MOST_NODES {
            if self.nodes_run == MOST_NODES {
                log::warn!("More than {MOST_NODES} nodes were run for one event: no more are");
                self.nodes_run += 1;
            }
            return Ok(());
        }
        // (One that is not run for being too deep is counted all the
        // same: a loop down there turns for nothing otherwise.)
        self.nodes_run += 1;
        if self.running_deep >= DEEPEST_ROW {
            log::warn!("A row of more than {DEEPEST_ROW} nodes is not run further");
            return Ok(());
        }
        self.running_deep += 1;
        let result = self.execute_node_counted(
            window_manager,
            monitor_manager,
            node_id,
            node_type,
            visited,
        );
        self.running_deep -= 1;
        result
    }

    /// Whether as many nodes were run for what happened as are run at
    /// the most: a loop turns no further then.
    pub(super) fn nodes_spent(&self) -> bool {
        self.nodes_run >= MOST_NODES
    }

    fn execute_node_counted(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node_type: NodeType,
        visited: &mut HashSet<NodeId>,
    ) -> Result<()> {
        let started = Instant::now();
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        if let Some(broadcast) = &self.node_event_broadcast {
            broadcast(NodeExecutionEvent::Started {
                node_id,
                node_type: node_type.clone(),
                timestamp_ms,
            });
        }

        let result = self.execute_node_inner(window_manager, monitor_manager, node_id, visited);

        if let Some(broadcast) = &self.node_event_broadcast {
            let finished_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            match &result {
                Ok(()) => broadcast(NodeExecutionEvent::Completed {
                    node_id,
                    node_type,
                    timestamp_ms: finished_at,
                    duration_ms: started.elapsed().as_millis() as i64,
                }),
                Err(error) => broadcast(NodeExecutionEvent::Failed {
                    node_id,
                    node_type,
                    timestamp_ms: finished_at,
                    error: error.to_string(),
                }),
            }
        }

        result
    }

    pub(super) fn execute_node_inner(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        visited: &mut HashSet<NodeId>,
    ) -> Result<()> {
        if !visited.insert(node_id) {
            log::warn!(
                "Cycle detected in graph execution at node {}. Halting this path.",
                node_id
            );
            return Ok(());
        }

        let node = match self.graph.nodes.get(&node_id).cloned() {
            Some(n) => n,
            None => {
                visited.remove(&node_id);
                return Ok(());
            }
        };

        log::trace!("🔹 Executing: {}", node.node_type.display_name());

        let done = match &node.node_type {
            NodeType::Unknown(_) => {
                log::warn!("A node this version of gevrek does not know was skipped");
                NodeDone::Early
            }
            NodeType::OnStartup
            | NodeType::OnStartupCompleted
            | NodeType::ManualTrigger
            | NodeType::OnWindowCreated
            | NodeType::OnWindowDestroyed
            | NodeType::OnWindowFocused
            | NodeType::OnWindowMoved
            | NodeType::OnWindowResized
            | NodeType::OnKeyPress { .. } => {
                self.run_events_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::LaunchProcess { .. }
            | NodeType::MoveWindow { .. }
            | NodeType::ResizeWindow { .. }
            | NodeType::CloseWindow
            | NodeType::SetWindowTitle { .. }
            | NodeType::AnimateWindow { .. }
            | NodeType::SetWindowArrangement { .. }
            | NodeType::BringToFront
            | NodeType::SendToBack
            | NodeType::SetAlwaysOnTop { .. }
            | NodeType::CloakWindow { .. }
            | NodeType::SetNativeBorderColor { .. }
            | NodeType::SetCaptionColor { .. }
            | NodeType::SetSystemBackdrop { .. }
            | NodeType::SetWindowCorner { .. } => {
                self.run_window_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::CreateFloatRule { .. }
            | NodeType::CreateIgnoreRule { .. }
            | NodeType::CreateManageRule { .. }
            | NodeType::CreateRule { .. }
            | NodeType::CombineRules
            | NodeType::ApplySystemFilters { .. } => {
                self.run_rules_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::WindowExists
            | NodeType::WindowHasTitle { .. }
            | NodeType::MatchWindow { .. }
            | NodeType::WindowModeSelector { .. }
            | NodeType::GetWindowPosition
            | NodeType::GetWindowSize
            | NodeType::GetActiveWindow
            | NodeType::GetAllWindows
            | NodeType::GetMonitorForWindow
            | NodeType::GetMonitors
            | NodeType::GetMonitorInfo
            | NodeType::GetWindowInfo => {
                self.run_query_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::CreateWorkspace { .. }
            | NodeType::SwitchToWorkspace { .. }
            | NodeType::FocusWorkspace { .. }
            | NodeType::NextWorkspace
            | NodeType::PreviousWorkspace
            | NodeType::GetCurrentWorkspace
            | NodeType::MoveWindowToWorkspace { .. }
            | NodeType::GetWorkspace { .. }
            | NodeType::MoveToNextWorkspace
            | NodeType::MoveToPreviousWorkspace => {
                self.run_workspace_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::Add
            | NodeType::Subtract
            | NodeType::Multiply
            | NodeType::Divide
            | NodeType::NumberValue { .. }
            | NodeType::StringValue { .. }
            | NodeType::BoolValue { .. }
            | NodeType::ColorValue { .. }
            | NodeType::BorderStyleValue { .. }
            | NodeType::ToString
            | NodeType::FromString
            | NodeType::LogMessage { .. }
            | NodeType::ForEach
            | NodeType::ForLoop { .. }
            | NodeType::WindowListToSingle { .. } => {
                self.run_value_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::Retile { .. }
            | NodeType::SetLayoutRatio { .. }
            | NodeType::IncrementLayoutRatio { .. }
            | NodeType::DecrementLayoutRatio { .. }
            | NodeType::SetMasterCount { .. }
            | NodeType::IncrementMasterCount
            | NodeType::DecrementMasterCount
            | NodeType::SetLayoutGap { .. }
            | NodeType::CycleNextLayout
            | NodeType::CyclePreviousLayout
            | NodeType::RotateLayout => {
                self.run_layout_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::FocusNextWindow
            | NodeType::FocusPreviousWindow
            | NodeType::CycleFocusNext
            | NodeType::CycleFocusPrevious
            | NodeType::SwapWithNext
            | NodeType::SwapWithPrevious
            | NodeType::ToggleFloating
            | NodeType::ToggleExposeMode => {
                self.run_focus_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
            NodeType::ToggleScratchpad { .. }
            | NodeType::ShowScratchpad { .. }
            | NodeType::HideScratchpad { .. }
            | NodeType::HideAllScratchpads => {
                self.run_scratchpad_node(window_manager, monitor_manager, node_id, &node, visited)?
            }
        };
        // A node that stopped early stays marked as visited, as it always did.
        if done == NodeDone::Early {
            return Ok(());
        }

        visited.remove(&node_id);
        Ok(())
    }

    /// Get output pin IDs for a node (for setting output values).
    pub(super) fn get_node_output_pins(&self, node_id: NodeId) -> Vec<PinId> {
        self.graph
            .nodes
            .get(&node_id)
            .map(|n| n.outputs.iter().map(|p| p.id).collect())
            .unwrap_or_default()
    }

    pub(super) fn shared_match_kind(
        kind: &gevrek_model::graph::RuleMatchKind,
    ) -> crate::rules::MatchKind {
        match kind {
            gevrek_model::graph::RuleMatchKind::Title => crate::rules::MatchKind::Title,
            gevrek_model::graph::RuleMatchKind::Class => crate::rules::MatchKind::Class,
            gevrek_model::graph::RuleMatchKind::Exe => crate::rules::MatchKind::Exe,
            gevrek_model::graph::RuleMatchKind::Path => crate::rules::MatchKind::Path,
            gevrek_model::graph::RuleMatchKind::AppId => crate::rules::MatchKind::AppId,
        }
    }

    pub(super) fn shared_match_strategy(
        strategy: &gevrek_model::graph::RuleMatchStrategy,
    ) -> crate::rules::MatchStrategy {
        match strategy {
            gevrek_model::graph::RuleMatchStrategy::Equals => crate::rules::MatchStrategy::Equals,
            gevrek_model::graph::RuleMatchStrategy::Contains => {
                crate::rules::MatchStrategy::Contains
            }
            gevrek_model::graph::RuleMatchStrategy::StartsWith => {
                crate::rules::MatchStrategy::StartsWith
            }
            gevrek_model::graph::RuleMatchStrategy::EndsWith => {
                crate::rules::MatchStrategy::EndsWith
            }
            gevrek_model::graph::RuleMatchStrategy::Regex => crate::rules::MatchStrategy::Regex,
        }
    }

    pub(super) fn trigger_output_execution(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        output_index: usize,
        visited: &mut HashSet<NodeId>,
    ) -> Result<()> {
        // Find all connections from this node's output
        let node = match self.graph.nodes.get(&node_id) {
            Some(n) => n,
            None => return Ok(()),
        };

        if output_index >= node.outputs.len() {
            return Ok(());
        }

        let output_pin_id = node.outputs[output_index].id;

        let connected_nodes: Vec<(NodeId, PinId)> = self
            .graph
            .connections
            .iter()
            .filter(|c| c.from_node == node_id && c.from_pin == output_pin_id)
            .map(|c| (c.to_node, c.to_pin))
            .collect();

        for (next_node_id, _) in connected_nodes {
            self.execute_node(window_manager, monitor_manager, next_node_id, visited)?;
        }

        Ok(())
    }

    pub(super) fn get_input_value(&mut self, node_id: NodeId, input_index: usize) -> Option<Value> {
        let node = self.graph.nodes.get(&node_id)?;

        if input_index >= node.inputs.len() {
            log::trace!(
                "get_input_value: node {} input_index {} out of bounds (len={})",
                node_id,
                input_index,
                node.inputs.len()
            );
            return None;
        }

        let input_pin_id = node.inputs[input_index].id;

        // Find the connection to this input
        let connection = self
            .graph
            .connections
            .iter()
            .find(|c| c.to_node == node_id && c.to_pin == input_pin_id);

        let connection = match connection {
            Some(c) => c,
            None => {
                log::trace!(
                    "get_input_value: no connection found for node {} input {} (pin_id {})",
                    node_id,
                    input_index,
                    input_pin_id
                );
                return None;
            }
        };

        // Extract values we need before any mutable borrows
        let from_node_id = connection.from_node;
        let from_pin_id = connection.from_pin;

        // Check if source is a pure data node that needs evaluation
        let source_node = self.graph.nodes.get(&from_node_id)?;
        let is_pure_data_node = matches!(
            source_node.node_type,
            NodeType::NumberValue { .. }
                | NodeType::StringValue { .. }
                | NodeType::BoolValue { .. }
                | NodeType::ColorValue { .. }
                | NodeType::BorderStyleValue { .. }
                | NodeType::Add
                | NodeType::Subtract
                | NodeType::Multiply
                | NodeType::Divide
                | NodeType::ToString
                | NodeType::FromString
        );

        if is_pure_data_node {
            // Always re-evaluate pure data nodes to ensure they reflect current input values
            // This is important for nodes like ToString that depend on values that change during loops
            // (Not one that is being worked out already, where the wires
            // go round, and not behind more nodes than anybody draws: a
            // graph is a file anyone may have written, and each node
            // asked is room that is not given back until it answers.
            // What such a node said last stands.)
            if self.evaluating.len() >= DEEPEST_VALUE {
                log::warn!("A value behind more than {DEEPEST_VALUE} nodes is not worked out");
            } else if self.evaluating.insert(from_node_id) {
                self.evaluate_data_node(from_node_id);
                self.evaluating.remove(&from_node_id);
            } else {
                log::warn!("Node {from_node_id} is asked for its own value: the wires go round");
            }
            return self.pin_values.get(&(from_node_id, from_pin_id)).cloned();
        }

        // For non-data nodes, just return the cached value
        let result = self.pin_values.get(&(from_node_id, from_pin_id)).cloned();
        if result.is_none() {
            log::trace!("get_input_value: pin_values has no value for (node={}, pin={}). Connection: from_node={}, from_pin={}",
                node_id, input_index, from_node_id, from_pin_id);
        }
        result
    }

    /// Evaluate a pure data node (nodes that compute outputs from inputs without side effects)
    pub(super) fn evaluate_data_node(&mut self, node_id: NodeId) {
        let node = match self.graph.nodes.get(&node_id) {
            Some(n) => n,
            None => return,
        };

        match &node.node_type {
            NodeType::NumberValue { value } => {
                self.set_output_value(node_id, 0, Value::Number(*value));
            }
            NodeType::StringValue { value } => {
                self.set_output_value(node_id, 0, Value::String(value.clone()));
            }
            NodeType::BoolValue { value } => {
                self.set_output_value(node_id, 0, Value::Bool(*value));
            }
            NodeType::ColorValue { r, g, b } => {
                self.set_output_value(node_id, 0, Value::Color(*r, *g, *b));
            }
            NodeType::BorderStyleValue { value } => {
                self.set_output_value(node_id, 0, Value::BorderStyle(*value));
            }
            NodeType::Add => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                self.set_output_value(node_id, 0, Value::Number(a + b));
            }
            NodeType::Subtract => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                self.set_output_value(node_id, 0, Value::Number(a - b));
            }
            NodeType::Multiply => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                self.set_output_value(node_id, 0, Value::Number(a * b));
            }
            NodeType::Divide => {
                let a = self
                    .get_input_value(node_id, 0)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0);
                let b = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::Number(n) = v {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(1.0);
                self.set_output_value(node_id, 0, Value::Number(a / b));
            }
            NodeType::ToString => {
                if let Some(value) = self.get_input_value(node_id, 0) {
                    let string_value = match value {
                        Value::Number(n) => {
                            if n.fract() == 0.0 {
                                format!("{}", n as i64)
                            } else {
                                format!("{}", n)
                            }
                        }
                        Value::Bool(b) => format!("{}", b),
                        Value::String(s) => s,
                        _ => format!("{:?}", value),
                    };
                    self.set_output_value(node_id, 0, Value::String(string_value));
                }
            }
            NodeType::FromString => {
                if let Some(Value::String(s)) = self.get_input_value(node_id, 0) {
                    if let Ok(n) = s.parse::<f64>() {
                        self.set_output_value(node_id, 0, Value::Number(n));
                    } else {
                        self.set_output_value(node_id, 0, Value::Number(0.0));
                    }
                }
            }
            _ => {
                // Not a pure data node, shouldn't be called
            }
        }
    }

    pub(super) fn set_output_value(&mut self, node_id: NodeId, output_index: usize, value: Value) {
        if let Some(node) = self.graph.nodes.get(&node_id) {
            if output_index < node.outputs.len() {
                let output_pin_id = node.outputs[output_index].id;
                log::trace!(
                    "set_output_value: node={}, output_index={}, pin_id={}, value={:?}",
                    node_id,
                    output_index,
                    output_pin_id,
                    value
                );
                self.pin_values.insert((node_id, output_pin_id), value);
            } else {
                log::warn!(
                    "set_output_value: output_index {} out of bounds for node {} (len={})",
                    output_index,
                    node_id,
                    node.outputs.len()
                );
            }
        } else {
            log::warn!("set_output_value: node {} not found in graph", node_id);
        }
    }

    /// Execute hotkey event - triggers matching OnKeyPress nodes
    pub fn execute_hotkey(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        _focus_manager: &mut FocusManager,
        hotkey: &crate::input::HotkeyEvent,
    ) -> Result<()> {
        self.release_placed_scratchpads(monitor_manager);
        // Find all OnKeyPress nodes that match this hotkey (collect first to avoid borrow issues)
        let mut matching_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter_map(|(node_id, node)| {
                if let NodeType::OnKeyPress {
                    key,
                    ctrl,
                    alt,
                    shift,
                    win,
                } = &node.node_type
                {
                    if gevrek_model::keys::key_name_to_vk(key)
                        == gevrek_model::keys::key_name_to_vk(&hotkey.key)
                        && ctrl == &hotkey.ctrl
                        && alt == &hotkey.alt
                        && shift == &hotkey.shift
                        && win == &hotkey.win
                    {
                        return Some(*node_id);
                    }
                }
                None
            })
            .collect();
        // (In the order of the nodes: the same on every start.)
        matching_nodes.sort_unstable();
        let _run = GraphRun::start("OnKeyPress", matching_nodes.len(), false);

        // Execute matching nodes
        for node_id in matching_nodes {
            log::trace!(
                "the node {} runs for the shortcut {}",
                node_id,
                hotkey.key
            );

            // Set output values for the hotkey data
            self.set_output_value(node_id, 1, Value::String(hotkey.key.clone()));
            self.set_output_value(node_id, 2, Value::Bool(hotkey.ctrl));
            self.set_output_value(node_id, 3, Value::Bool(hotkey.alt));
            self.set_output_value(node_id, 4, Value::Bool(hotkey.shift));
            self.set_output_value(node_id, 5, Value::Bool(hotkey.win));

            // Trigger execution
            let mut visited = HashSet::new();
            self.trigger_output_execution(
                window_manager,
                monitor_manager,
                node_id,
                0,
                &mut visited,
            )?;
        }

        Ok(())
    }

    /// Focus the next window of the active window's workspace. Shared by the graph node and the dispatch command.
    pub(super) fn action_focus_next_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // Get active window to determine which monitor to operate on
        if let Some(active_window) = queries(window_manager).active_window() {
            if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, active_window) {
                let monitor_handle = monitor_info.handle;

                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    if let Some(window_handle) = Self::focus_workspace_window_by_offset(
                        workspace,
                        window_manager,
                        active_window,
                        1,
                    )? {
                        log::debug!(
                            "Focused next window {:?} on monitor: {}",
                            window_handle,
                            monitor_info.name
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Focus the previous window of the active window's workspace. Shared by the graph node and the dispatch command.
    pub(super) fn action_focus_previous_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // Get active window to determine which monitor to operate on
        if let Some(active_window) = queries(window_manager).active_window() {
            if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, active_window) {
                let monitor_handle = monitor_info.handle;

                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    if let Some(window_handle) = Self::focus_workspace_window_by_offset(
                        workspace,
                        window_manager,
                        active_window,
                        -1,
                    )? {
                        log::debug!(
                            "Focused previous window {:?} on monitor: {}",
                            window_handle,
                            monitor_info.name
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Swap the focused container with the next one and re-tile. Shared by the graph node and the dispatch command.
    pub(super) fn action_swap_with_next(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // Get active window to determine which monitor to operate on
        if let Some(active_window) = queries(window_manager).active_window() {
            if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, active_window) {
                let monitor_handle = monitor_info.handle;

                // Get monitor work area for layout
                let work_area = monitor_manager
                    .get_adjusted_work_area(monitor_handle, window_manager)
                    .unwrap_or_else(|| monitor_info.work_area_rect());

                // Get mutable workspace and perform swap
                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    workspace.swap_container_with_next();
                    log::debug!("Swapped with next window on monitor: {}", monitor_info.name);

                    self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
                }
            }
        }
        Ok(())
    }

    /// Swap the focused container with the previous one and re-tile. Shared by the graph node and the dispatch command.
    pub(super) fn action_swap_with_previous(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // Get active window to determine which monitor to operate on
        if let Some(active_window) = queries(window_manager).active_window() {
            if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, active_window) {
                let monitor_handle = monitor_info.handle;

                // Get monitor work area for layout
                let work_area = monitor_manager
                    .get_adjusted_work_area(monitor_handle, window_manager)
                    .unwrap_or_else(|| monitor_info.work_area_rect());

                // Get mutable workspace and perform swap
                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    workspace.swap_container_with_previous();
                    log::debug!(
                        "Swapped with previous window on monitor: {}",
                        monitor_info.name
                    );

                    self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
                }
            }
        }
        Ok(())
    }

    /// Switch the active window between tiled and floating. Shared by the graph node and the dispatch command.
    pub(super) fn action_toggle_floating(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // Get the currently active window to determine which monitor to operate on
        if let Some(window_handle) = queries(window_manager).active_window() {
            if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, window_handle) {
                let monitor_handle = monitor_info.handle;

                // Get monitor work area for layout
                let work_area =
                    if let Some(monitor_info) = monitor_manager.get_monitor(monitor_handle) {
                        monitor_manager
                            .get_adjusted_work_area(monitor_handle, window_manager)
                            .unwrap_or_else(|| monitor_info.work_area_rect())
                    } else {
                        log::warn!("Could not get monitor info for layout");
                        return Ok(());
                    };

                // Check if window is currently floating
                let is_floating = monitor_manager.is_window_floating(window_handle);

                // A window with the keyboard that is on another workspace
                // than the one shown (it took the keyboard by itself, or
                // was given it by a command) floats or tiles where it is.
                // It used to be put on the workspace that is shown as
                // well, and was on two workspaces from then on.
                if let Some((its_monitor, its_workspace)) = monitor_manager.find_window(window_handle)
                {
                    let shown = monitor_manager
                        .get_workspaces(its_monitor)
                        .map(|workspaces| workspaces.focused_idx());
                    if its_monitor != monitor_handle || shown != Some(its_workspace) {
                        return self.set_window_floating(
                            window_manager,
                            monitor_manager,
                            window_handle,
                            !is_floating,
                        );
                    }
                }

                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    if is_floating {
                        // Convert from floating to tiled
                        workspace.remove_window(window_handle); // Remove from floating_windows
                        workspace.add_window(window_handle, true); // Add to containers as tiled
                        self.apply_workspace_layout_and_position(
                            workspace,
                            work_area,
                            window_manager,
                        )?;

                        // Remove min/max buttons for tiled windows
                        if let Err(e) = effect(
                            window_manager,
                            Effect::Platform(PlatformEffect::Windows(
                                WindowsEffect::CaptionButtons {
                                    id: window_handle,
                                    shown: false,
                                },
                            )),
                        ) {
                            log::warn!("Failed to remove min/max buttons: {}", e);
                        }

                        log::debug!(
                            "Converted window to tiled mode on monitor: {}",
                            monitor_info.name
                        );
                        // A tiled window is not kept on top (see
                        // `set_window_on_top`).
                        if queries(window_manager).is_always_on_top(window_handle) {
                            effect(
                                window_manager,
                                Effect::Stack {
                                    id: window_handle,
                                    op: StackOp::AlwaysOnTop(false),
                                },
                            )?;
                        }
                    } else {
                        // Convert from tiled to floating
                        workspace.remove_window(window_handle); // Remove from containers
                        workspace.add_floating_window(window_handle); // Add to floating_windows
                        self.apply_workspace_layout_and_position(
                            workspace,
                            work_area,
                            window_manager,
                        )?;

                        // Restore min/max buttons for floating windows
                        if let Err(e) = effect(
                            window_manager,
                            Effect::Platform(PlatformEffect::Windows(
                                WindowsEffect::CaptionButtons {
                                    id: window_handle,
                                    shown: true,
                                },
                            )),
                        ) {
                            log::warn!("Failed to restore min/max buttons: {}", e);
                        }

                        log::debug!(
                            "Converted window to floating mode on monitor: {}",
                            monitor_info.name
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Enter or leave the window overview on the active window's workspace. Shared by the graph node and the dispatch command.
    pub(super) fn action_toggle_expose(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // As pictures of the windows, where the desktop can show them so:
        // of the display with the keyboard's window, or else of the one
        // the pointer is on. Nothing is moved, and nothing is kept here.
        let shown = queries(window_manager)
            .active_window()
            .and_then(|window| monitor_of(window_manager, monitor_manager, window))
            .or_else(|| monitor_at_cursor(window_manager, monitor_manager));
        if let Some(monitor) = shown {
            let arranged = monitor_manager
                .get_focused_workspace(monitor.handle)
                .is_some_and(|workspace| self.expose_workspaces.contains(&workspace.id));
            if !arranged {
                let area = monitor_manager
                    .get_adjusted_work_area(monitor.handle, window_manager)
                    .unwrap_or_else(|| monitor.work_area_rect());
                let windows = monitor_manager
                    .get_focused_workspace(monitor.handle)
                    .map(|workspace| workspace.all_windows())
                    .unwrap_or_default();
                if let Some(up) = window_manager.toggle_window_overview(&windows, area) {
                    log::debug!(
                        "The overview of {} window(s) is {}",
                        windows.len(),
                        if up { "shown" } else { "put away" }
                    );
                    return Ok(());
                }
            }
        }
        if let Some(window_handle) = queries(window_manager).active_window() {
            if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, window_handle) {
                let monitor_handle = monitor_info.handle;

                let work_area = if let Some(mi) = monitor_manager.get_monitor(monitor_handle) {
                    monitor_manager
                        .get_adjusted_work_area(monitor_handle, window_manager)
                        .unwrap_or_else(|| mi.work_area_rect())
                } else {
                    return Ok(());
                };

                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    let ws_id = workspace.id;

                    if self.expose_workspaces.contains(&ws_id) {
                        // Deactivate expose: restore all windows to original positions
                        if expose::is_active(workspace) {
                            let ops = expose::deactivate(workspace);
                            if !ops.is_empty() {
                                Self::batch_move_resize_and_sync_borders(window_manager, &ops)?;
                                self.last_expose_transition = Instant::now();
                            }
                            log::debug!("Expose mode deactivated on workspace {}", ws_id);
                        }
                        self.expose_workspaces.remove(&ws_id);
                        // Restore borders after exiting expose
                        try_effect(window_manager, Effect::DecorationsVisible(true));
                    } else {
                        // Hide borders during expose
                        try_effect(window_manager, Effect::DecorationsVisible(false));

                        // Activate expose: capture positions and arrange in grid
                        let window_count = workspace.all_windows().len();
                        let grid = expose::activate(workspace, work_area, |window| {
                            queries(window_manager)
                                .rect(window)
                                .map_err(anyhow::Error::msg)
                                .ok()
                        });
                        if let Some(ops) = grid {
                            if !ops.is_empty() {
                                if let Err(error) =
                                    Self::batch_move_resize_and_sync_borders(window_manager, &ops)
                                {
                                    // As before: a failed arrangement leaves
                                    // expose off.
                                    workspace.expose_state = None;
                                    return Err(error);
                                }
                                self.last_expose_transition = Instant::now();
                            }

                            self.expose_workspaces.insert(ws_id);
                            log::debug!(
                                "Expose mode activated on workspace {} with {} windows",
                                ws_id,
                                window_count
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// A workspace-switch node of the graph, run by the same code as the
    /// switch command. A node never creates a workspace: an index that does
    /// not exist is logged and the graph goes on.
    pub(super) fn graph_switch_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        monitor: Option<MonitorHandle>,
        selector: WorkspaceSelector,
    ) {
        let Some(monitor) = monitor else {
            return;
        };
        let result = self
            .resolve_existing_workspace_selector(monitor_manager, monitor, &selector)
            .and_then(|index| {
                self.switch_workspace_on(
                    window_manager,
                    monitor_manager,
                    monitor,
                    WorkspaceSelector::Index(index),
                )
            });
        if let Err(error) = result {
            log::warn!(
                "Graph node could not switch to workspace '{}': {}",
                selector.expression(),
                error
            );
        }
    }

    /// The monitor a workspace node acts on: the active window's, else the
    /// primary one.
    pub(super) fn graph_node_monitor(
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
    ) -> Option<MonitorHandle> {
        queries(window_manager)
            .active_window()
            .and_then(|window| monitor_of(window_manager, monitor_manager, window))
            .map(|monitor| monitor.handle)
            .or_else(|| monitor_manager.get_primary_monitor_handle())
    }

    pub(super) fn graph_layout_command(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        command: DispatchCommand,
    ) {
        let result =
            self.update_focused_workspace_layout(window_manager, monitor_manager, |workspace| {
                commands::apply_layout_command(workspace, &command)
                    .map(|_| ())
                    .map_err(anyhow::Error::msg)
            });
        if let Err(error) = result {
            log::warn!(
                "Graph node '{}' did nothing: {}",
                command.display_name(),
                error
            );
        }
    }

    /// Execute a manual graph trigger or one of the built-in CLI actions.
    ///
    /// Numeric targets address a `ManualTrigger` node in the active graph. Named
    /// targets are stable aliases used by the `gevrek` satellite CLI.
    pub fn execute_manual_target(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: &str,
    ) -> Result<()> {
        if let Ok(node_id) = target.parse::<NodeId>() {
            let node =
                self.graph.nodes.get(&node_id).ok_or_else(|| {
                    anyhow::anyhow!("Manual trigger node {} does not exist", node_id)
                })?;

            if !matches!(node.node_type, NodeType::ManualTrigger) {
                anyhow::bail!("Node {} is not a Manual Trigger node", node_id);
            }

            self.clear_state();
            let mut visited = HashSet::new();
            return self.execute_node(window_manager, monitor_manager, node_id, &mut visited);
        }

        anyhow::bail!(
            "'{}' is a built-in action, not a Manual Trigger node; it runs as a command",
            target
        )
    }

    /// The command behind a built-in manual target (the stable aliases of
    /// the `gevrek` CLI). `None` for a numeric target, which names a
    /// `ManualTrigger` node of the graph.
    pub fn manual_target_command(target: &str) -> Result<Option<DispatchCommand>> {
        if target.parse::<NodeId>().is_ok() {
            return Ok(None);
        }
        let command =
            match target {
                "focus_next" => DispatchCommand::FocusNext,
                "focus_previous" => DispatchCommand::FocusPrevious,
                "move_to_next" => DispatchCommand::MoveToWorkspaceSelector {
                    selector: WorkspaceSelector::Next,
                },
                "move_to_previous" => DispatchCommand::MoveToWorkspaceSelector {
                    selector: WorkspaceSelector::Previous,
                },
                "toggle_float" => DispatchCommand::ToggleFloating,
                _ => {
                    let Some(index) = target.strip_prefix("workspace_") else {
                        anyhow::bail!("Unknown manual target '{}'", target);
                    };
                    DispatchCommand::SwitchWorkspaceSelector {
                        selector: WorkspaceSelector::Index(index.parse().map_err(|_| {
                            anyhow::anyhow!("Invalid workspace target '{}'", target)
                        })?),
                    }
                }
            };
        Ok(Some(command))
    }

    /// Get reference to the node graph
    pub fn get_graph(&self) -> &NodeGraph {
        &self.graph
    }
}

/// How long the graph took from one trigger, told as the run ends (an
/// error that ends it early too). At debug for a trigger that comes now
/// and then (a shortcut, a window that opens or closes, the start), at
/// trace for one that comes with each move of a window or change of the
/// keyboard. Nothing is told where no node was there to run.
pub(super) struct GraphRun {
    trigger: &'static str,
    nodes: usize,
    started: Instant,
    often: bool,
}

impl GraphRun {
    pub(super) fn start(trigger: &'static str, nodes: usize, often: bool) -> Self {
        Self {
            trigger,
            nodes,
            started: Instant::now(),
            often,
        }
    }
}

impl Drop for GraphRun {
    fn drop(&mut self) {
        if self.nodes == 0 {
            return;
        }
        let level = if self.often {
            log::Level::Trace
        } else {
            log::Level::Debug
        };
        log::log!(
            level,
            "the graph ran from {} {} node(s) in {:.1} ms",
            self.nodes,
            self.trigger,
            self.started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
