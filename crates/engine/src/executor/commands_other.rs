//! Expose, scratchpads, zones, and the commands only the host can carry out.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_other(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        snap_zone_manager: &mut SnapZoneManager,
        _focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::ToggleExpose => {
                self.action_toggle_expose(window_manager, monitor_manager)
            }
            DispatchCommand::ToggleScratchpad { id } => {
                self.toggle_scratchpad(window_manager, monitor_manager, &id)
            }
            DispatchCommand::SnapActiveToZone { index } => {
                self.snap_active_to_zone(window_manager, monitor_manager, snap_zone_manager, index)
            }
            DispatchCommand::CycleActiveZoneNext => {
                self.cycle_active_zone(window_manager, monitor_manager, snap_zone_manager, false)
            }
            DispatchCommand::CycleActiveZonePrevious => {
                self.cycle_active_zone(window_manager, monitor_manager, snap_zone_manager, true)
            }
            DispatchCommand::ScrollView { pixels } => {
                self.scroll_view(window_manager, monitor_manager, pixels);
                Ok(())
            }
            DispatchCommand::SetBindingSubmap { .. }
            | DispatchCommand::ResetBindingSubmap
            | DispatchCommand::Quit
            | DispatchCommand::KeyboardLayout { .. }
            | DispatchCommand::ExpectWindow { .. }
            | DispatchCommand::WorkspaceOverview
            | DispatchCommand::PickWindow
            | DispatchCommand::NightLightPreview { .. }
            | DispatchCommand::ShortcutsInhibit
            | DispatchCommand::OutputTrial { .. }
            | DispatchCommand::Shell { .. }
            | DispatchCommand::Notify { .. }
            | DispatchCommand::AppShortcut { .. }
            | DispatchCommand::Osd { .. }
            | DispatchCommand::Exec { .. }
            | DispatchCommand::LaunchStartup { .. }
            | DispatchCommand::StopStartup { .. }
            | DispatchCommand::RestartStartup { .. }
            | DispatchCommand::ToggleStackbar
            | DispatchCommand::WatchConfiguration { .. }
            | DispatchCommand::ToggleTransparency
            | DispatchCommand::SetTransparencyAlpha { .. }
            | DispatchCommand::SetGlobalWorkAreaOffset { .. }
            | DispatchCommand::ShowShortcuts
            | DispatchCommand::ToggleMouseFollowsFocus => {
                anyhow::bail!("This command must be handled by the runtime")
            }
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
