//! Nodes that move focus or windows within a workspace.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_focus_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Window Navigation Nodes
            NodeType::FocusNextWindow => {
                self.action_focus_next_window(window_manager, monitor_manager)?;

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::FocusPreviousWindow => {
                self.action_focus_previous_window(window_manager, monitor_manager)?;

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::CycleFocusNext => {
                // Cycle to the next window in focus history (Alt-Tab style)
                // This sets a pending operation that will be applied by the Runtime using FocusManager
                self.pending_focus_op = Some(PendingFocusOp::CycleNext);
                log::trace!("Cycle focus next requested");

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::CycleFocusPrevious => {
                // Cycle to the previous window in focus history (Alt-Shift-Tab style)
                // This sets a pending operation that will be applied by the Runtime using FocusManager
                self.pending_focus_op = Some(PendingFocusOp::CyclePrevious);
                log::trace!("Cycle focus previous requested");

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SwapWithNext => {
                self.action_swap_with_next(window_manager, monitor_manager)?;

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SwapWithPrevious => {
                self.action_swap_with_previous(window_manager, monitor_manager)?;

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::ToggleFloating => {
                self.action_toggle_floating(window_manager, monitor_manager)?;

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::ToggleExposeMode => {
                self.action_toggle_expose(window_manager, monitor_manager)?;

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
