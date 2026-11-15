//! Event nodes: they only pass execution on.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_events_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Event nodes
            NodeType::OnStartup => {
                // Trigger connected execution nodes
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::OnStartupCompleted => {
                // Trigger connected execution nodes (fires after all OnStartup nodes complete)
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::ManualTrigger => {
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::OnWindowCreated | NodeType::OnWindowDestroyed | NodeType::OnWindowFocused => {
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::OnWindowMoved | NodeType::OnWindowResized => {
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::OnKeyPress { .. } => {
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
