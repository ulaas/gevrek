//! Layout commands: tiling on and off, resizing, promoting, the resize-state file.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_layout(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        _snap_zone_manager: &mut SnapZoneManager,
        _focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::Retile => {
                self.retile_focused_workspaces(window_manager, monitor_manager)
            }
            DispatchCommand::ToggleTiling => self.toggle_tiling(window_manager, monitor_manager),
            DispatchCommand::ToggleWorkspaceTiling { selector } => {
                self.toggle_workspace_tiling(window_manager, monitor_manager, selector)
            }
            // Layout settings of one workspace: decided by the engine's
            // command table, then laid out here.
            command @ (DispatchCommand::CycleNextLayout
            | DispatchCommand::CyclePreviousLayout
            | DispatchCommand::SetLayout { .. }
            | DispatchCommand::SetLayoutRatio { .. }
            | DispatchCommand::AdjustLayoutRatio { .. }
            | DispatchCommand::SetMasterCount { .. }
            | DispatchCommand::AdjustMasterCount { .. }
            | DispatchCommand::RotateLayout
            | DispatchCommand::FlipLayout { .. }
            | DispatchCommand::ToggleMonocle
            | DispatchCommand::ToggleLayoutLock
            | DispatchCommand::SetLayoutGap { .. }
            | DispatchCommand::AdjustLayoutGap { .. }
            | DispatchCommand::SetWorkspacePadding { .. }
            | DispatchCommand::AdjustWorkspacePadding { .. }
            | DispatchCommand::SetContainerPadding { .. }
            | DispatchCommand::AdjustContainerPadding { .. }
            | DispatchCommand::SetWorkspaceWorkAreaOffset { .. }
            | DispatchCommand::SetDwindleDirection { .. }
            | DispatchCommand::CycleDwindleDirection
            | DispatchCommand::ToggleDwindlePreserveSplit) => {
                self.apply_layout_command(window_manager, monitor_manager, &command)
            }
            DispatchCommand::ResizeEdge { edge, sizing } => {
                self.resize_edge(window_manager, monitor_manager, edge, sizing)
            }
            DispatchCommand::ResizeAxis { axis, sizing } => {
                self.resize_axis(window_manager, monitor_manager, axis, sizing)
            }
            DispatchCommand::SetResizeDelta { pixels } => {
                if !(1..=1000).contains(&pixels) {
                    anyhow::bail!("Resize delta must be between 1 and 1000 pixels");
                }
                self.resize_delta = pixels;
                log::debug!("Resize delta set to {} pixels", pixels);
                Ok(())
            }
            DispatchCommand::SaveResize { path } => self
                .save_resize_dimensions(monitor_manager, std::path::Path::new(&path))
                .map(|_| ()),
            DispatchCommand::LoadResize { path } => self
                .load_resize_dimensions(
                    window_manager,
                    monitor_manager,
                    std::path::Path::new(&path),
                )
                .map(|_| ()),
            DispatchCommand::QuickSaveResize => self
                .save_resize_dimensions(
                    monitor_manager,
                    &crate::persistence::ResizeDimensionsState::quick_path(),
                )
                .map(|_| ()),
            DispatchCommand::QuickLoadResize => self
                .load_resize_dimensions(
                    window_manager,
                    monitor_manager,
                    &crate::persistence::ResizeDimensionsState::quick_path(),
                )
                .map(|_| ()),
            DispatchCommand::PromoteToMaster => {
                self.promote_to_master(window_manager, monitor_manager)
            }
            DispatchCommand::Promote => self.promote(window_manager, monitor_manager),
            DispatchCommand::PromoteFocus => self.promote_focus(window_manager, monitor_manager),
            DispatchCommand::PromoteWindow { direction } => {
                // Focus the container in the given
                // direction, then promote it to the front of the tree.
                self.focus_direction(window_manager, monitor_manager, direction)?;
                self.promote(window_manager, monitor_manager)
            }
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
