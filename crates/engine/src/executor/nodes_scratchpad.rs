//! Scratchpad nodes.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_scratchpad_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Scratchpad nodes
            NodeType::ToggleScratchpad {
                scratchpad_id,
                pattern,
                match_kind,
                match_strategy,
                spawn_command,
                width_percent,
                height_percent,
                position,
            } => {
                let profile = ScratchpadProfile {
                    id: scratchpad_id.clone(),
                    enabled: true,
                    match_kind: Self::shared_match_kind(match_kind),
                    match_strategy: Self::shared_match_strategy(match_strategy),
                    pattern: pattern.clone(),
                    launch: spawn_command
                        .as_deref()
                        .and_then(Self::legacy_launch_profile),
                    monitor: MonitorSelector::Any,
                    width_percent: *width_percent,
                    height_percent: *height_percent,
                    position: *position,
                    ..ScratchpadProfile::default()
                };
                let normalized = self.scratchpads.insert_profile(profile);
                self.toggle_scratchpad(window_manager, monitor_manager, &normalized)?;

                let outputs = self.get_node_output_pins(node_id);
                if let Some((handle, visible)) = self.scratchpads.window(&normalized) {
                    if outputs.len() > 1 {
                        self.pin_values
                            .insert((node_id, outputs[1]), Value::WindowHandle(handle));
                    }
                    if outputs.len() > 2 {
                        self.pin_values
                            .insert((node_id, outputs[2]), Value::Bool(visible));
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

            NodeType::ShowScratchpad { scratchpad_id } => {
                self.show_scratchpad(window_manager, monitor_manager, scratchpad_id)?;
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::HideScratchpad { scratchpad_id } => {
                self.hide_scratchpad(window_manager, scratchpad_id)?;
                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::HideAllScratchpads => {
                self.hide_all_scratchpads(window_manager);
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
