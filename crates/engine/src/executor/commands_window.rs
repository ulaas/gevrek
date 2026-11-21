//! Commands on one window: moving, resizing, closing, its states, floating, managing.

use super::*;

impl Executor {
    /// Carry out one command of this family (`execute_dispatch` picks the family).
    pub(super) fn dispatch_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        _snap_zone_manager: &mut SnapZoneManager,
        _focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::MoveActive { delta_x, delta_y } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                let rect = queries(window_manager)
                    .rect(active)
                    .map_err(anyhow::Error::msg)?;
                effect(
                    window_manager,
                    Effect::Configure {
                        id: active,
                        rect: gevrek_model::geometry::Rect {
                            x: rect.x.saturating_add(delta_x),
                            y: rect.y.saturating_add(delta_y),
                            width: rect.width,
                            height: rect.height,
                        },
                        states: WindowStates::default(),
                    },
                )
            }
            command @ (DispatchCommand::MoveWindow { .. }
            | DispatchCommand::ResizeWindow { .. }
            | DispatchCommand::CloseWindow { .. }
            | DispatchCommand::RaiseWindow { .. }
            | DispatchCommand::LowerWindow { .. }) => {
                self.apply_window_command(window_manager, monitor_manager, &command)
            }
            DispatchCommand::ResizeActive {
                delta_width,
                delta_height,
            } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                let rect = queries(window_manager)
                    .rect(active)
                    .map_err(anyhow::Error::msg)?;
                effect(
                    window_manager,
                    Effect::Configure {
                        id: active,
                        rect: gevrek_model::geometry::Rect {
                            x: rect.x,
                            y: rect.y,
                            width: rect.width.saturating_add(delta_width).max(100),
                            height: rect.height.saturating_add(delta_height).max(100),
                        },
                        states: WindowStates::default(),
                    },
                )
            }
            DispatchCommand::CloseActive => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                effect(window_manager, Effect::Close(active))
            }
            DispatchCommand::MinimizeActive => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                effect(
                    window_manager,
                    Effect::State {
                        id: active,
                        change: StateChange::Minimized(true),
                    },
                )?;
                self.window_state.get_mut(active).minimized = true;
                Ok(())
            }
            DispatchCommand::ToggleMaximized => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                let maximized = !queries(window_manager).is_maximized(active);
                effect(
                    window_manager,
                    Effect::State {
                        id: active,
                        change: StateChange::Maximized(maximized),
                    },
                )
            }
            DispatchCommand::ToggleFullscreen => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                let fullscreen = !queries(window_manager).is_fullscreen(active);
                effect(
                    window_manager,
                    Effect::State {
                        id: active,
                        change: StateChange::Fullscreen(fullscreen),
                    },
                )?;
                if !fullscreen {
                    if let Some(monitor) = monitor_of(window_manager, monitor_manager, active) {
                        self.mark_layout_dirty(monitor.handle);
                    }
                }
                Ok(())
            }
            DispatchCommand::ToggleWindowFullscreen { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                let fullscreen = !queries(window_manager).is_fullscreen(target);
                effect(
                    window_manager,
                    Effect::State {
                        id: target,
                        change: StateChange::Fullscreen(fullscreen),
                    },
                )?;
                if !fullscreen && monitor_manager.find_window(target).is_some() {
                    self.update_window_workspace_layout(
                        window_manager,
                        monitor_manager,
                        target,
                        |_| Ok(()),
                    )?;
                }
                Ok(())
            }
            DispatchCommand::TogglePinned => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                let on_top = !queries(window_manager).is_always_on_top(active);
                self.set_window_on_top(window_manager, monitor_manager, active, on_top)
            }
            DispatchCommand::ToggleWindowPinned { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                let on_top = !queries(window_manager).is_always_on_top(target);
                self.set_window_on_top(window_manager, monitor_manager, target, on_top)
            }
            DispatchCommand::ToggleFloating => {
                self.action_toggle_floating(window_manager, monitor_manager)
            }
            DispatchCommand::ToggleWindowFloating { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                if monitor_manager.find_window(target).is_none() {
                    anyhow::bail!("The selected window is not managed by gevrek");
                }
                let enabled = !monitor_manager.is_window_floating(target);
                self.set_window_floating(window_manager, monitor_manager, target, enabled)
            }
            DispatchCommand::SetPinned { enabled } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.set_window_on_top(window_manager, monitor_manager, active, enabled)
            }
            DispatchCommand::SetWindowPinned { selector, enabled } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.set_window_on_top(window_manager, monitor_manager, target, enabled)
            }
            DispatchCommand::SetFloating { enabled } => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                if monitor_manager.find_window(active).is_none() {
                    anyhow::bail!("The active window is not managed by gevrek");
                }
                self.set_window_floating(window_manager, monitor_manager, active, enabled)
            }
            DispatchCommand::SetWindowFloating { selector, enabled } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                if monitor_manager.find_window(target).is_none() {
                    anyhow::bail!("The selected window is not managed by gevrek");
                }
                self.set_window_floating(window_manager, monitor_manager, target, enabled)
            }
            DispatchCommand::ToggleFloatingAside => {
                self.toggle_floating_aside(window_manager, monitor_manager)
            }
            DispatchCommand::CenterActive => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.center_window(window_manager, monitor_manager, active)
            }
            DispatchCommand::CenterWindow { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.focus_managed_window(window_manager, monitor_manager, target)?;
                self.center_window(window_manager, monitor_manager, target)
            }
            DispatchCommand::RaiseActive => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                effect(
                    window_manager,
                    Effect::Stack {
                        id: active,
                        op: StackOp::Raise,
                    },
                )
            }
            DispatchCommand::LowerActive => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                effect(
                    window_manager,
                    Effect::Stack {
                        id: active,
                        op: StackOp::Lower,
                    },
                )
            }
            DispatchCommand::ToggleTitleBar => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                window_manager.toggle_title_bar(active)?;
                Ok(())
            }
            DispatchCommand::ToggleWindowTitleBar { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                window_manager.toggle_title_bar(target)?;
                Ok(())
            }
            DispatchCommand::ManageWindow { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.manage_window(window_manager, monitor_manager, target)
            }
            DispatchCommand::UnmanageWindow { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.unmanage_window(window_manager, monitor_manager, target)
            }
            DispatchCommand::RestoreWindows => {
                let restored = self.restore_hidden_windows(window_manager);
                log::debug!("Restored {} hidden window(s)", restored);
                Ok(())
            }
            DispatchCommand::TogglePseudotile => {
                let active = queries(window_manager)
                    .active_window()
                    .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
                self.toggle_window_pseudotile(window_manager, monitor_manager, active)
            }
            DispatchCommand::ToggleWindowPseudotile { selector } => {
                let target =
                    self.resolve_window_selector(window_manager, monitor_manager, &selector)?;
                self.toggle_window_pseudotile(window_manager, monitor_manager, target)
            }
            other => anyhow::bail!(
                "'{}' is not handled by this command family",
                other.display_name()
            ),
        }
    }
}
