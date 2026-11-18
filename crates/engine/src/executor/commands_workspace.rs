//! Workspace and monitor commands.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        _focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::SwitchWorkspaceSelector { selector } => {
                self.switch_workspace_wherever_it_is(window_manager, monitor_manager, selector)
            }
            DispatchCommand::MoveToWorkspaceSelector { selector } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.move_window_to_workspace_selector(
                    window_manager,
                    monitor_manager,
                    active,
                    selector,
                    false,
                )?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, active)
            }
            DispatchCommand::MoveToWorkspaceAndFollow { selector } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.move_window_to_workspace_selector(
                    window_manager,
                    monitor_manager,
                    active,
                    selector,
                    true,
                )?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, active)
            }
            DispatchCommand::MoveWindowToWorkspace { window, workspace } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &window)?;
                self.move_window_to_workspace_selector(
                    window_manager,
                    monitor_manager,
                    target,
                    workspace,
                    false,
                )?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, target)
            }
            DispatchCommand::MoveWindowToWorkspaceAndFollow { window, workspace } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &window)?;
                self.move_window_to_workspace_selector(
                    window_manager,
                    monitor_manager,
                    target,
                    workspace,
                    true,
                )?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, target)
            }
            DispatchCommand::RenameWorkspace { selector, name } => {
                self.rename_workspace(window_manager, monitor_manager, selector, name)
            }
            DispatchCommand::FocusMonitor { target } => {
                self.focus_monitor(window_manager, monitor_manager, target)
            }
            DispatchCommand::FocusMonitorAtCursor => {
                self.focus_monitor_at_cursor(window_manager, monitor_manager)
            }
            DispatchCommand::FocusMonitorWorkspace { target, workspace } => {
                self.focus_monitor_workspace(window_manager, monitor_manager, target, workspace)
            }
            DispatchCommand::MoveWorkspaceToMonitor { target } => self.move_workspace_to_monitor(
                window_manager,
                monitor_manager,
                zone_manager,
                target,
            ),
            DispatchCommand::SwapActiveWorkspaces { target } => {
                self.swap_active_workspaces(window_manager, monitor_manager, zone_manager, target)
            }
            DispatchCommand::ToggleWindowCentered { window, target } => {
                let window =
                    self.resolve_window_selector(window_manager, monitor_manager, &window)?;
                let (_, display) =
                    self.resolve_monitor_target(window_manager, monitor_manager, &target)?;
                self.toggle_window_centered(window_manager, monitor_manager, window, display)
            }
            DispatchCommand::MoveWindowToMonitor { window, target } => {
                let window =
                    self.resolve_window_selector(window_manager, monitor_manager, &window)?;
                self.move_window_to_monitor(
                    window_manager,
                    monitor_manager,
                    window,
                    target,
                    false,
                )?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, window)
            }
            DispatchCommand::MoveWindowToMonitorAndFollow { window, target } => {
                let window =
                    self.resolve_window_selector(window_manager, monitor_manager, &window)?;
                self.move_window_to_monitor(window_manager, monitor_manager, window, target, true)?;
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, window)
            }
            DispatchCommand::NewWorkspace { name } => {
                self.new_workspace(window_manager, monitor_manager, name)
            }
            DispatchCommand::CloseWorkspace => {
                self.close_workspace(window_manager, monitor_manager)
            }
            DispatchCommand::EnsureWorkspaces { target, count } => {
                self.ensure_workspaces(window_manager, monitor_manager, target, count)
            }
            DispatchCommand::EnsureNamedWorkspace { target, name, layout } => {
                self.ensure_named_workspace(window_manager, monitor_manager, target, name, layout)
            }
            DispatchCommand::ToggleWindowContainerBehaviour => {
                self.toggle_window_container_behaviour(window_manager, monitor_manager)
            }
            DispatchCommand::ToggleWorkspaceWindowContainerBehaviour { selector } => self
                .toggle_workspace_window_container_behaviour(
                    window_manager,
                    monitor_manager,
                    selector,
                ),
            DispatchCommand::ToggleWorkspaceFloatOverride { selector } => {
                self.toggle_workspace_float_override(window_manager, monitor_manager, selector)
            }
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
