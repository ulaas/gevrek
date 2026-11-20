//! Nodes that change the layout of the focused workspace.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_layout_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            NodeType::Retile { layout } => {
                // Get workspace index from input pin (required)
                if let Some(Value::Number(workspace_num)) = self.get_input_value(node_id, 1) {
                    let workspace_idx = workspace_num as usize;

                    if let Some(monitor_handle) = monitor_manager.get_primary_monitor_handle() {
                        // Get monitor info for work area
                        let work_area = if let Some(monitor_info) =
                            monitor_manager.get_monitor(monitor_handle)
                        {
                            monitor_manager
                                .get_adjusted_work_area(monitor_handle, window_manager)
                                .unwrap_or_else(|| monitor_info.work_area_rect())
                        } else {
                            log::warn!("Could not get monitor info for retiling");
                            self.trigger_output_execution(
                                window_manager,
                                monitor_manager,
                                node_id,
                                0,
                                visited,
                            )?;
                            return Ok(NodeDone::Early);
                        };

                        // Get the workspaces ring for the monitor
                        if let Some(workspaces) = monitor_manager.get_workspaces_mut(monitor_handle)
                        {
                            // Get the workspace at the specified index
                            if let Some(workspace) = workspaces.get_mut(workspace_idx) {
                                // Set the new layout
                                workspace.layout = *layout;
                                log::trace!(
                                    "Changed workspace {} layout to {:?}",
                                    workspace.name,
                                    layout
                                );

                                self.apply_workspace_layout_and_position(
                                    workspace,
                                    work_area,
                                    window_manager,
                                )?;

                                log::trace!(
                                    "Retiled workspace {} with layout {:?}",
                                    workspace.name,
                                    layout
                                );
                            } else {
                                log::warn!("Workspace index {} does not exist", workspace_idx);
                            }
                        }
                    }
                } else {
                    log::warn!("Retile node missing workspace number input");
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Layout Configuration Nodes
            NodeType::SetLayoutRatio { ratio } => {
                let ratio = match self.get_input_value(node_id, 1) {
                    Some(Value::Number(value)) => value as f32,
                    _ => *ratio,
                };
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::SetLayoutRatio { ratio },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::IncrementLayoutRatio { step } => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::AdjustLayoutRatio { delta: *step },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::DecrementLayoutRatio { step } => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::AdjustLayoutRatio { delta: -*step },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetMasterCount { count } => {
                let count = match self.get_input_value(node_id, 1) {
                    Some(Value::Number(value)) => value as usize,
                    _ => *count,
                };
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::SetMasterCount { count },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::IncrementMasterCount => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::AdjustMasterCount { delta: 1 },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::DecrementMasterCount => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::AdjustMasterCount { delta: -1 },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetLayoutGap { gap } => {
                let gap = match self.get_input_value(node_id, 1) {
                    Some(Value::Number(value)) => value as i32,
                    _ => *gap,
                };
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::SetWorkspacePadding { padding: gap },
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::CycleNextLayout => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::CycleNextLayout,
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::CyclePreviousLayout => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::CyclePreviousLayout,
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::RotateLayout => {
                self.graph_layout_command(
                    window_manager,
                    monitor_manager,
                    DispatchCommand::RotateLayout,
                );

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
