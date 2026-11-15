//! Window group (stack) commands.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_group(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        _zone_manager: &mut ZoneManager,
        _focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::ToggleGroup => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.toggle_group(window_manager, monitor_manager, active)
            }
            DispatchCommand::ToggleWindowGroup { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.toggle_group(window_manager, monitor_manager, target)
            }
            DispatchCommand::ToggleGroupLock => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.toggle_group_lock(monitor_manager, active)
            }
            DispatchCommand::ToggleWindowGroupLock { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.toggle_group_lock(monitor_manager, target)
            }
            DispatchCommand::CycleGroupNext => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.cycle_group(window_manager, monitor_manager, active, true)
            }
            DispatchCommand::CycleGroupPrevious => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.cycle_group(window_manager, monitor_manager, active, false)
            }
            DispatchCommand::CycleWindowGroup { selector, forward } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.cycle_group(window_manager, monitor_manager, target, forward)
            }
            DispatchCommand::FocusGroupIndex { index } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.focus_group_index(window_manager, monitor_manager, active, index)
            }
            DispatchCommand::FocusWindowGroupIndex { selector, index } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.focus_group_index(window_manager, monitor_manager, target, index)
            }
            DispatchCommand::MoveGroupWindow { forward } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.move_group_window(monitor_manager, active, forward)
            }
            DispatchCommand::MoveWindowInGroup { selector, forward } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.move_group_window(monitor_manager, target, forward)
            }
            DispatchCommand::MergeGroupNext => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.merge_group(window_manager, monitor_manager, active, true)
            }
            DispatchCommand::MergeGroupPrevious => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.merge_group(window_manager, monitor_manager, active, false)
            }
            DispatchCommand::UngroupActive => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.ungroup_window(window_manager, monitor_manager, active)
            }
            DispatchCommand::StackAll => self.stack_all(window_manager, monitor_manager),
            DispatchCommand::UnstackAll => self.unstack_all(window_manager, monitor_manager),
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
