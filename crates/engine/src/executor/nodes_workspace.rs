//! Nodes that create, switch and fill workspaces.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_workspace_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            NodeType::CreateWorkspace {
                workspace_name,
                layout,
                padding_x,
                padding_y,
                workspace_padding_x,
                workspace_padding_y,
                border_enabled,
                border_width,
                border_offset,
                border_style,
                active_border_color,
                inactive_border_color,
                active_border_alpha,
                inactive_border_alpha,
            } => {
                if let Some(Value::Monitor(monitor_handle)) = self.get_input_value(node_id, 1) {
                    // Get workspace name from input or use default
                    let name = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| workspace_name.clone());

                    // Get layout from input or use default
                    let layout_type = self
                        .get_input_value(node_id, 3)
                        .and_then(|v| {
                            if let Value::Layout(l) = v {
                                Some(l)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*layout);

                    // Get container padding values from inputs (gaps between windows)
                    let padding_x_val = self
                        .get_input_value(node_id, 4)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*padding_x);

                    let padding_y_val = self
                        .get_input_value(node_id, 5)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*padding_y);

                    // Get workspace padding values from inputs (outer margin)
                    let workspace_padding_x_val = self
                        .get_input_value(node_id, 6)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*workspace_padding_x);

                    let workspace_padding_y_val = self
                        .get_input_value(node_id, 7)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*workspace_padding_y);

                    // Get border configuration from inputs
                    let border_enabled_val = self
                        .get_input_value(node_id, 8)
                        .and_then(|v| {
                            if let Value::Bool(b) = v {
                                Some(b)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*border_enabled);

                    let border_width_val = self
                        .get_input_value(node_id, 9)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*border_width);

                    let border_offset_val = self
                        .get_input_value(node_id, 10)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*border_offset);

                    let border_style_val = self
                        .get_input_value(node_id, 11)
                        .and_then(|v| {
                            if let Value::BorderStyle(s) = v {
                                Some(s)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*border_style);

                    let active_border_color_val = self
                        .get_input_value(node_id, 12)
                        .and_then(|v| {
                            if let Value::Color(r, g, b) = v {
                                Some((r, g, b))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*active_border_color);

                    let inactive_border_color_val = self
                        .get_input_value(node_id, 13)
                        .and_then(|v| {
                            if let Value::Color(r, g, b) = v {
                                Some((r, g, b))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*inactive_border_color);

                    let active_border_alpha_val = self
                        .get_input_value(node_id, 14)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as u8)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*active_border_alpha);

                    let inactive_border_alpha_val = self
                        .get_input_value(node_id, 15)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as u8)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*inactive_border_alpha);

                    log::debug!("CreateWorkspace: Attempting to create '{}' with {:?} layout, container padding (x:{}, y:{}), and workspace padding (x:{}, y:{}) on monitor {:?}",
                        name, layout_type, padding_x_val, padding_y_val, workspace_padding_x_val, workspace_padding_y_val, monitor_handle);

                    let settings = crate::outputs::WorkspaceSettings {
                        layout: layout_type,
                        container_padding_x: padding_x_val,
                        container_padding_y: padding_y_val,
                        workspace_padding_x: workspace_padding_x_val,
                        workspace_padding_y: workspace_padding_y_val,
                        border_enabled: border_enabled_val,
                        border_width: border_width_val,
                        border_offset: border_offset_val,
                        border_style: border_style_val,
                        active_border_color: active_border_color_val,
                        inactive_border_color: inactive_border_color_val,
                        active_border_alpha: active_border_alpha_val,
                        inactive_border_alpha: inactive_border_alpha_val,
                    };
                    let key = (monitor_handle, name.to_ascii_lowercase());
                    self.graph_workspaces.insert(key.clone(), settings.clone());

                    // During a hot reload the workspace usually exists already:
                    // update it in place (keeping windows, focus and resize
                    // state), and only if the settings this node produces
                    // changed, whether in the node itself or in a value wired
                    // into it.
                    let existing = self
                        .reload_previous_workspaces
                        .is_some()
                        .then(|| monitor_manager.find_workspace_index(monitor_handle, &name))
                        .flatten();
                    let workspace_index = match existing {
                        Some(index) => {
                            let unchanged = self
                                .reload_previous_workspaces
                                .as_ref()
                                .and_then(|previous| previous.get(&key))
                                == Some(&settings);
                            if !unchanged {
                                monitor_manager.update_workspace_settings(
                                    monitor_handle,
                                    index,
                                    &settings,
                                );
                                log::debug!(
                                    "Hot reload updated workspace '{}' on monitor {:?}",
                                    name,
                                    monitor_handle
                                );
                            }
                            Some(index)
                        }
                        None => monitor_manager.create_workspace_with_padding(
                            monitor_handle,
                            name.clone(),
                            &settings,
                        ),
                    };
                    if let Some(workspace_index) = workspace_index {
                        log::debug!("Workspace '{}' at index {} on monitor {:?} with container padding (x:{}, y:{}) and workspace padding (x:{}, y:{})",
                            name, workspace_index, monitor_handle, padding_x_val, padding_y_val, workspace_padding_x_val, workspace_padding_y_val);

                        // Set output values
                        self.set_output_value(node_id, 1, Value::Number(workspace_index as f64)); // Workspace index
                        self.set_output_value(node_id, 2, Value::String(name)); // Name passthrough
                        self.set_output_value(node_id, 3, Value::Layout(layout_type));
                        // Layout passthrough
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

            // Workspace nodes
            NodeType::SwitchToWorkspace { workspace_index }
            | NodeType::FocusWorkspace { workspace_index } => {
                let index = match self.get_input_value(node_id, 1) {
                    Some(Value::Number(value)) => value as usize,
                    _ => *workspace_index,
                };
                // These nodes address the primary monitor.
                let monitor = monitor_manager.get_primary_monitor_handle();
                self.graph_switch_workspace(
                    window_manager,
                    monitor_manager,
                    monitor,
                    WorkspaceSelector::Index(index),
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::NextWorkspace => {
                let monitor = Self::graph_node_monitor(window_manager, monitor_manager);
                self.graph_switch_workspace(
                    window_manager,
                    monitor_manager,
                    monitor,
                    WorkspaceSelector::Next,
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::PreviousWorkspace => {
                let monitor = Self::graph_node_monitor(window_manager, monitor_manager);
                self.graph_switch_workspace(
                    window_manager,
                    monitor_manager,
                    monitor,
                    WorkspaceSelector::Previous,
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::GetCurrentWorkspace => {
                if let Some(monitor_handle) = monitor_manager.get_primary_monitor_handle() {
                    if let Some(workspaces) = monitor_manager.get_workspaces(monitor_handle) {
                        let current_idx = workspaces.focused_idx();
                        self.set_output_value(node_id, 1, Value::Number(current_idx as f64));
                        log::trace!("Current workspace: {}", current_idx);
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

            NodeType::MoveWindowToWorkspace { workspace_index } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let index = match self.get_input_value(node_id, 2) {
                        Some(Value::Number(value)) => value as usize,
                        _ => *workspace_index,
                    };
                    // On the window's own monitor; a window that is not
                    // managed yet goes to the primary monitor.
                    let monitor = monitor_manager
                        .find_window(handle)
                        .map(|(monitor, _)| monitor)
                        .or_else(|| monitor_manager.get_primary_monitor_handle());
                    if let Some(monitor) = monitor {
                        let floating = monitor_manager.is_window_floating(handle);
                        if let Err(error) = self.move_window_to_workspace(
                            window_manager,
                            monitor_manager,
                            handle,
                            monitor,
                            index,
                            floating,
                        ) {
                            log::warn!("Graph node could not move the window: {}", error);
                        }
                    }

                    // Pass through window for chaining
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

            NodeType::GetWorkspace { workspace_index } => {
                // Return the configured workspace index
                self.set_output_value(node_id, 1, Value::Number(*workspace_index as f64));
                log::trace!("GetWorkspace returning index: {}", workspace_index);

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::MoveToNextWorkspace => {
                // The active window moves; it is hidden with its new workspace.
                if let Some(window) = queries(window_manager).active_window() {
                    if let Err(error) = self.move_window_to_workspace_selector(
                        window_manager,
                        monitor_manager,
                        window,
                        WorkspaceSelector::Next,
                        false,
                    ) {
                        log::warn!("Graph node could not move the window: {}", error);
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

            NodeType::MoveToPreviousWorkspace => {
                // The active window moves; it is hidden with its new workspace.
                if let Some(window) = queries(window_manager).active_window() {
                    if let Err(error) = self.move_window_to_workspace_selector(
                        window_manager,
                        monitor_manager,
                        window,
                        WorkspaceSelector::Previous,
                        false,
                    ) {
                        log::warn!("Graph node could not move the window: {}", error);
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
