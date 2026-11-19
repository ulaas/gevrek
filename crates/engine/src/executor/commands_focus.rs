//! Focus and swap commands.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_focus(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        _snap_zone_manager: &mut SnapZoneManager,
        focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::FocusNext => {
                self.action_focus_next_window(window_manager, monitor_manager)
            }
            DispatchCommand::FocusPrevious => {
                self.action_focus_previous_window(window_manager, monitor_manager)
            }
            DispatchCommand::FocusLast => {
                let active = queries(window_manager).active_window();
                let target = focus_manager
                    .history()
                    .copied()
                    .find(|handle| {
                        Some(*handle) != active && queries(window_manager).exists(*handle)
                    })
                    .ok_or_else(|| anyhow::anyhow!("There is no previous managed window"))?;
                self.focus_managed_window(window_manager, monitor_manager, target)
            }
            DispatchCommand::FocusWindow { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.focus_managed_window(window_manager, monitor_manager, target)
            }
            DispatchCommand::ForceFocusWindow { selector } => {
                // Focus through a `NoFocus` rule for
                // this single explicit action.
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.focus_managed_window_with_rule_bypass(
                    window_manager,
                    monitor_manager,
                    target,
                    true,
                )
            }
            DispatchCommand::FocusDirection { direction } => {
                self.focus_direction(window_manager, monitor_manager, direction)
            }
            DispatchCommand::SwapNext => {
                self.action_swap_with_next(window_manager, monitor_manager)
            }
            DispatchCommand::SwapPrevious => {
                self.action_swap_with_previous(window_manager, monitor_manager)
            }
            DispatchCommand::SwapDirection { direction } => {
                self.swap_direction(window_manager, monitor_manager, direction)
            }
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
