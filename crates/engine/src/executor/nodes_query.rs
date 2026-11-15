//! Nodes that ask about windows and monitors.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_query_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Condition nodes
            NodeType::WindowExists => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    if queries(window_manager).exists(handle) {
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?; // True
                    } else {
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            1,
                            visited,
                        )?; // False
                    }
                }
            }

            NodeType::WindowHasTitle { title } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let title = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| title.clone());

                    if queries(window_manager)
                        .has_title(handle, &title)
                        .map_err(anyhow::Error::msg)?
                    {
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?; // True
                    } else {
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            1,
                            visited,
                        )?; // False
                    }
                }
            }

            NodeType::MatchWindow {
                match_kind,
                match_strategy,
                pattern,
            } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let pattern_str = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| pattern.clone());

                    // Get window info for matching
                    let all_windows = queries(window_manager).windows();
                    if let Some(window) = all_windows.iter().find(|w| w.handle == handle) {
                        let match_target = match match_kind {
                            gevrek_model::graph::RuleMatchKind::Title => window.title.clone(),
                            gevrek_model::graph::RuleMatchKind::Class => {
                                queries(window_manager).class(handle)
                            }
                            gevrek_model::graph::RuleMatchKind::Exe => {
                                queries(window_manager).exe(handle)
                            }
                            gevrek_model::graph::RuleMatchKind::Path => {
                                queries(window_manager).path(handle)
                            }
                            gevrek_model::graph::RuleMatchKind::AppId => {
                                queries(window_manager).app_id(handle)
                            }
                        };

                        // Apply match strategy
                        let matches = match match_strategy {
                            gevrek_model::graph::RuleMatchStrategy::Equals => {
                                match_target == pattern_str
                            }
                            gevrek_model::graph::RuleMatchStrategy::Contains => {
                                match_target.contains(&pattern_str)
                            }
                            gevrek_model::graph::RuleMatchStrategy::StartsWith => {
                                match_target.starts_with(&pattern_str)
                            }
                            gevrek_model::graph::RuleMatchStrategy::EndsWith => {
                                match_target.ends_with(&pattern_str)
                            }
                            gevrek_model::graph::RuleMatchStrategy::Regex => {
                                regex::Regex::new(&pattern_str)
                                    .map(|regex| regex.is_match(&match_target))
                                    .unwrap_or(false)
                            }
                        };

                        if matches {
                            log::info!(
                                "Window matches rule: {:?} {:?} '{}'",
                                match_kind,
                                match_strategy,
                                pattern_str
                            );
                            self.trigger_output_execution(
                                window_manager,
                                monitor_manager,
                                node_id,
                                0,
                                visited,
                            )?; // Matches
                        } else {
                            self.trigger_output_execution(
                                window_manager,
                                monitor_manager,
                                node_id,
                                1,
                                visited,
                            )?; // No Match
                        }
                    } else {
                        // Window not found
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            1,
                            visited,
                        )?; // No Match
                    }
                }
            }

            NodeType::WindowModeSelector { default_mode } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let all_windows = queries(window_manager).windows();

                    let should_tile =
                        if let Some(window) = all_windows.iter().find(|w| w.handle == handle) {
                            // First check if window should be ignored entirely (Gevrek's own windows)
                            if queries(window_manager).ignored_by_rules(handle) {
                                log::debug!(
                                    "Window '{}' should be ignored, setting as FLOATING",
                                    window.title
                                );
                                false // Float ignored windows
                            } else {
                                let should_float = queries(window_manager).floats_by_rules(handle)
                                    || window.title.is_empty();

                                if should_float {
                                    log::debug!("Window '{}' matched float rule", window.title);
                                    false // Float
                                } else {
                                    // Use default mode for unmatched windows
                                    matches!(default_mode, WindowMode::Tiled)
                                }
                            }
                        } else {
                            // Window not found, use default
                            matches!(default_mode, WindowMode::Tiled)
                        };

                    if should_tile {
                        log::info!(
                            "Window mode: TILED - '{}'",
                            all_windows
                                .iter()
                                .find(|w| w.handle == handle)
                                .map(|w| w.title.as_str())
                                .unwrap_or("unknown")
                        );
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?; // Tile output
                    } else {
                        log::info!(
                            "Window mode: FLOATING - '{}'",
                            all_windows
                                .iter()
                                .find(|w| w.handle == handle)
                                .map(|w| w.title.as_str())
                                .unwrap_or("unknown")
                        );
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            1,
                            visited,
                        )?; // Float output
                    }
                }
            }

            // Data nodes
            NodeType::GetWindowPosition => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    if let Some((x, y)) = queries(window_manager).position(handle) {
                        let output_count = self
                            .graph
                            .nodes
                            .get(&node_id)
                            .map(|n| n.outputs.len())
                            .unwrap_or(0);
                        if output_count > 1 {
                            self.set_output_value(node_id, 1, Value::Number(x as f64));
                        }
                        if output_count > 2 {
                            self.set_output_value(node_id, 2, Value::Number(y as f64));
                        }
                    }
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::GetWindowSize => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    if let Some((width, height)) = queries(window_manager).size(handle) {
                        let output_count = self
                            .graph
                            .nodes
                            .get(&node_id)
                            .map(|n| n.outputs.len())
                            .unwrap_or(0);
                        if output_count > 1 {
                            self.set_output_value(node_id, 1, Value::Number(width as f64));
                        }
                        if output_count > 2 {
                            self.set_output_value(node_id, 2, Value::Number(height as f64));
                        }
                    }
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::GetActiveWindow => {
                if let Some(handle) = queries(window_manager).active_window() {
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::GetAllWindows => {
                // Filter out windows that should be ignored (like Gevrek's own windows)
                let windows = queries(window_manager)
                    .windows()
                    .iter()
                    .filter(|w| !queries(window_manager).ignored_by_rules(w.handle))
                    .map(|w| w.handle)
                    .collect::<Vec<_>>();

                log::debug!(
                    "GetAllWindows returned {} manageable windows",
                    windows.len()
                );
                self.set_output_value(node_id, 1, Value::WindowList(windows));
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::GetMonitorForWindow => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    if let Some(monitor_info) = monitor_of(window_manager, monitor_manager, handle)
                    {
                        let monitors = monitor_manager.get_all_monitors();
                        if let Some(index) = monitors
                            .iter()
                            .position(|m| m.handle == monitor_info.handle)
                        {
                            self.set_output_value(node_id, 1, Value::Monitor(monitor_info.handle));
                            self.set_output_value(node_id, 2, Value::Number(index as f64));
                        }
                    }
                }
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Monitor and Workspace Creation Nodes
            NodeType::GetMonitors => {
                let monitors = monitor_manager
                    .get_all_monitors()
                    .iter()
                    .map(|m| m.handle)
                    .collect::<Vec<_>>();

                log::debug!(
                    "GetMonitors: Found {} monitors with handles: {:?}",
                    monitors.len(),
                    monitors
                );
                self.set_output_value(node_id, 1, Value::MonitorList(monitors));
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::GetMonitorInfo => {
                if let Some(Value::Monitor(handle)) = self.get_input_value(node_id, 1) {
                    if let Some(info) = monitor_manager.get_monitor(handle) {
                        // Set all monitor info output values
                        self.set_output_value(node_id, 1, Value::String(info.name.clone()));
                        self.set_output_value(node_id, 2, Value::Number(info.x as f64));
                        self.set_output_value(node_id, 3, Value::Number(info.y as f64));
                        self.set_output_value(node_id, 4, Value::Number(info.width as f64));
                        self.set_output_value(node_id, 5, Value::Number(info.height as f64));
                        self.set_output_value(node_id, 6, Value::Number(info.work_x as f64));
                        self.set_output_value(node_id, 7, Value::Number(info.work_y as f64));
                        self.set_output_value(node_id, 8, Value::Number(info.work_width as f64));
                        self.set_output_value(node_id, 9, Value::Number(info.work_height as f64));
                        self.set_output_value(node_id, 10, Value::Bool(info.is_primary));

                        log::info!("Got monitor info: {}", info.name);
                    }
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Helper nodes
            NodeType::GetWindowInfo => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let all_windows = queries(window_manager).windows();
                    if let Some(window) = all_windows.iter().find(|w| w.handle == handle) {
                        // Set output values
                        self.set_output_value(node_id, 1, Value::String(window.title.clone()));
                        self.set_output_value(node_id, 2, Value::Number(window.x as f64));
                        self.set_output_value(node_id, 3, Value::Number(window.y as f64));
                        self.set_output_value(node_id, 4, Value::Number(window.width as f64));
                        self.set_output_value(node_id, 5, Value::Number(window.height as f64));
                        log::debug!("Got window info for '{}'", window.title);
                    }
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Other families are run by their own function.
            _ => {}
        }
        Ok(NodeDone::Normal)
    }
}
