//! Window tag and rule commands.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_rules(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        _zone_manager: &mut ZoneManager,
        _focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::ClearWindowTags => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.rule_metadata.clear_tags(active);
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, active)
            }
            DispatchCommand::ClearSelectedWindowTags { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.rule_metadata.clear_tags(target);
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, target)
            }
            DispatchCommand::AddWindowTag { tag } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.add_window_tag(window_manager, active, &tag)?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, active)
            }
            DispatchCommand::AddSelectedWindowTag { selector, tag } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.add_window_tag(window_manager, target, &tag)?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, target)
            }
            DispatchCommand::RemoveWindowTag { tag } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.rule_metadata.remove_tag(active, &tag);
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, active)
            }
            DispatchCommand::RemoveSelectedWindowTag { selector, tag } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.rule_metadata.remove_tag(target, &tag);
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, target)
            }
            DispatchCommand::ToggleWindowTag { tag } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                if !self.rule_metadata.remove_tag(active, &tag) {
                    self.add_window_tag(window_manager, active, &tag)?;
                }
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, active)
            }
            DispatchCommand::ToggleSelectedWindowTag { selector, tag } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                if !self.rule_metadata.remove_tag(target, &tag) {
                    self.add_window_tag(window_manager, target, &tag)?;
                }
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, target)
            }
            DispatchCommand::EnableRule { name } => {
                self.rule_engine
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Rule engine lock is poisoned"))?
                    .set_rule_enabled(&name, true)
                    .map_err(anyhow::Error::msg)?;
                self.reevaluate_all_dynamic_rules(window_manager, monitor_manager)
            }
            DispatchCommand::DisableRule { name } => {
                self.rule_engine
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Rule engine lock is poisoned"))?
                    .set_rule_enabled(&name, false)
                    .map_err(anyhow::Error::msg)?;
                self.reevaluate_all_dynamic_rules(window_manager, monitor_manager)
            }
            DispatchCommand::ToggleRule { name } => {
                self.rule_engine
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Rule engine lock is poisoned"))?
                    .toggle_rule(&name)
                    .map_err(anyhow::Error::msg)?;
                self.reevaluate_all_dynamic_rules(window_manager, monitor_manager)
            }
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
