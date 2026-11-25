//! Carrying out commands (bindings, the CLI, gRPC, the editor), and resolving the window
//! and monitor selectors they name.

use super::*;

impl Executor {
    /// Run a layout-setting command on the workspace it targets and lay
    /// that workspace out.
    pub(super) fn apply_layout_command(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        command: &DispatchCommand,
    ) -> Result<()> {
        let update = |workspace: &mut crate::workspace::Workspace| {
            commands::apply_layout_command(workspace, command)
                .map(|_| ())
                .map_err(anyhow::Error::msg)
        };
        match commands::layout_scope(command) {
            Some(LayoutScope::ActiveWindow) => {
                self.update_active_workspace_layout(window_manager, monitor_manager, update)
            }
            Some(LayoutScope::FocusedWorkspace) => {
                self.update_focused_workspace_layout(window_manager, monitor_manager, update)
            }
            None => anyhow::bail!("{} is not a layout command", command.display_name()),
        }
    }

    /// A layout node of the graph: the same command a binding would send,
    /// applied to the visible workspace of the active monitor (a graph may
    /// run with no window focused, e.g. at startup). A node that cannot act
    /// is logged and the graph goes on.
    /// The commands that act on one window chosen by a selector. The CLI,
    /// bindings and gRPC reach this through `execute_dispatch`; the graph's
    /// window nodes call `apply_window_command_to` with the window they were
    /// given. One implementation for both.
    pub(super) fn apply_window_command(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        command: &DispatchCommand,
    ) -> Result<()> {
        let selector = match command {
            DispatchCommand::MoveWindow { selector, .. }
            | DispatchCommand::ResizeWindow { selector, .. }
            | DispatchCommand::CloseWindow { selector }
            | DispatchCommand::RaiseWindow { selector }
            | DispatchCommand::LowerWindow { selector } => selector,
            other => anyhow::bail!("'{}' is not a window command", other.display_name()),
        };
        let target = self.resolve_window_selector(window_manager, monitor_manager, selector)?;
        self.apply_window_command_to(window_manager, monitor_manager, target, command)
    }

    /// `command` applied to `target` (its selector is not consulted).
    pub(super) fn apply_window_command_to(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: WindowHandle,
        command: &DispatchCommand,
    ) -> Result<()> {
        match command {
            DispatchCommand::MoveWindow { x, y, relative, .. } => {
                // A tiled window would be put back by the next layout.
                self.set_window_floating(window_manager, monitor_manager, target, true)?;
                let rect = queries(window_manager)
                    .rect(target)
                    .map_err(anyhow::Error::msg)?;
                let (x, y) = if *relative {
                    (rect.x.saturating_add(*x), rect.y.saturating_add(*y))
                } else {
                    (*x, *y)
                };
                effect(
                    window_manager,
                    Effect::Configure {
                        id: target,
                        rect: gevrek_model::geometry::Rect {
                            x,
                            y,
                            width: rect.width,
                            height: rect.height,
                        },
                        states: WindowStates::default(),
                    },
                )?;
                try_effect(
                    window_manager,
                    Effect::Border {
                        id: target,
                        change: BorderChange::Follow,
                    },
                );
                Ok(())
            }
            DispatchCommand::ResizeWindow {
                width,
                height,
                relative,
                ..
            } => {
                self.set_window_floating(window_manager, monitor_manager, target, true)?;
                let rect = queries(window_manager)
                    .rect(target)
                    .map_err(anyhow::Error::msg)?;
                let (width, height) = if *relative {
                    (
                        rect.width.saturating_add(*width),
                        rect.height.saturating_add(*height),
                    )
                } else {
                    (*width, *height)
                };
                effect(
                    window_manager,
                    Effect::Configure {
                        id: target,
                        rect: gevrek_model::geometry::Rect {
                            x: rect.x,
                            y: rect.y,
                            width: width.max(100),
                            height: height.max(100),
                        },
                        states: WindowStates::default(),
                    },
                )?;
                try_effect(
                    window_manager,
                    Effect::Border {
                        id: target,
                        change: BorderChange::Follow,
                    },
                );
                Ok(())
            }
            DispatchCommand::CloseWindow { .. } => window_manager.apply(&Effect::Close(target)),
            DispatchCommand::RaiseWindow { .. } => window_manager.apply(&Effect::Stack {
                id: target,
                op: StackOp::Raise,
            }),
            DispatchCommand::LowerWindow { .. } => window_manager.apply(&Effect::Stack {
                id: target,
                op: StackOp::Lower,
            }),
            other => anyhow::bail!("'{}' is not a window command", other.display_name()),
        }
    }

    pub(super) fn resolve_monitor_target(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        target: &MonitorTarget,
    ) -> Result<(MonitorHandle, MonitorHandle)> {
        let current = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the focused monitor"))?;
        let resolved = Self::resolve_monitor_target_from(monitor_manager, current, target)?;
        Ok((current, resolved))
    }

    /// Resolve a monitor selector relative to a specific reference monitor.
    pub(super) fn resolve_monitor_target_from(
        monitor_manager: &MonitorManager,
        current: MonitorHandle,
        target: &MonitorTarget,
    ) -> Result<MonitorHandle> {
        Ok(workspaces::resolve_monitor_target(
            monitor_manager,
            current,
            target,
        )?)
    }

    pub(super) fn resolve_window_selector(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        selector: &WindowSelector,
    ) -> Result<WindowHandle> {
        fn text_matches(
            window_manager: &dyn Desktop,
            pattern: &str,
            kind: crate::rules::MatchKind,
        ) -> Vec<WindowHandle> {
            let (pattern, strategy) = pattern
                .strip_prefix("re:")
                .map(|pattern| (pattern, crate::rules::MatchStrategy::Regex))
                .unwrap_or((pattern, crate::rules::MatchStrategy::Contains));
            crate::matching::find_windows_by_match(window_manager, pattern, &kind, &strategy, false)
        }

        if matches!(selector, WindowSelector::Active) {
            let active = queries(window_manager)
                .active_window()
                .filter(|handle| queries(window_manager).is_tracked(*handle))
                .ok_or_else(|| anyhow::anyhow!("There is no active managed window"))?;
            return Ok(active);
        }
        if let WindowSelector::Handle(raw_handle) = selector {
            let handle = isize::try_from(*raw_handle)
                .map(WindowHandle)
                .map_err(|_| anyhow::anyhow!("Window handle {raw_handle} is out of range"))?;
            // The value is a window id (as `state` prints it) or a plain
            // HWND (as other tools print it); for a reused HWND they differ.
            let by_hwnd = window_manager.window_for_native_handle(handle.0);
            for candidate in [handle, by_hwnd] {
                if queries(window_manager).exists(candidate)
                    && queries(window_manager).is_tracked(candidate)
                {
                    return Ok(candidate);
                }
            }
            anyhow::bail!("No managed window has handle {raw_handle}");
        }

        let mut matches = match selector {
            WindowSelector::Active => unreachable!(),
            WindowSelector::Floating => queries(window_manager)
                .windows()
                .into_iter()
                .filter(|window| monitor_manager.is_window_floating(window.handle))
                .map(|window| window.handle)
                .collect(),
            WindowSelector::Tiled => queries(window_manager)
                .windows()
                .into_iter()
                .filter(|window| {
                    monitor_manager.find_window(window.handle).is_some()
                        && !monitor_manager.is_window_floating(window.handle)
                })
                .map(|window| window.handle)
                .collect(),
            WindowSelector::ProcessId(pid) => queries(window_manager)
                .windows()
                .into_iter()
                .filter(|window| queries(window_manager).pid(window.handle) == Some(*pid))
                .map(|window| window.handle)
                .collect(),
            WindowSelector::Title(pattern) => {
                text_matches(window_manager, pattern, crate::rules::MatchKind::Title)
            }
            WindowSelector::InitialTitle(pattern) => {
                let (pattern, strategy) = pattern
                    .strip_prefix("re:")
                    .map(|pattern| (pattern, crate::rules::MatchStrategy::Regex))
                    .unwrap_or((pattern.as_str(), crate::rules::MatchStrategy::Contains));
                crate::matching::find_windows_by_initial_match(
                    window_manager,
                    pattern,
                    false,
                    &strategy,
                    false,
                )
            }
            WindowSelector::Class(pattern) => {
                text_matches(window_manager, pattern, crate::rules::MatchKind::Class)
            }
            WindowSelector::InitialClass(pattern) => {
                let (pattern, strategy) = pattern
                    .strip_prefix("re:")
                    .map(|pattern| (pattern, crate::rules::MatchStrategy::Regex))
                    .unwrap_or((pattern.as_str(), crate::rules::MatchStrategy::Contains));
                crate::matching::find_windows_by_initial_match(
                    window_manager,
                    pattern,
                    true,
                    &strategy,
                    false,
                )
            }
            WindowSelector::Executable(pattern) => {
                text_matches(window_manager, pattern, crate::rules::MatchKind::Exe)
            }
            WindowSelector::Path(pattern) => {
                text_matches(window_manager, pattern, crate::rules::MatchKind::Path)
            }
            WindowSelector::Tag(tag) => {
                let tag = crate::rules::normalize_tag(tag);
                queries(window_manager)
                    .windows()
                    .into_iter()
                    .filter(|window| self.rule_metadata.tags(window.handle).contains(&tag))
                    .map(|window| window.handle)
                    .collect()
            }
            WindowSelector::Handle(_) => unreachable!(),
        };
        matches.sort_by_key(|handle| handle.0);
        matches
            .into_iter()
            .find(|handle| queries(window_manager).exists(*handle))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No managed window matches selector '{}'",
                    selector.expression()
                )
            })
    }

    pub fn execute_dispatch(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        snap_zone_manager: &mut SnapZoneManager,
        focus_manager: &mut FocusManager,
        command: DispatchCommand,
    ) -> Result<()> {
        log::debug!("Dispatching {}", command.display_name());
        self.release_placed_scratchpads(monitor_manager);
        let focus_before = queries(window_manager).active_window();
        // Pick the family by the command's kind, then hand the command over.
        #[derive(Clone, Copy)]
        enum Family {
            Focus,
            Window,
            Rules,
            Workspace,
            Layout,
            Group,
            Other,
        }
        let family = match &command {
            DispatchCommand::Unknown(_) => anyhow::bail!(
                "This command is not known to this version of gevrek: {}",
                serde_json::to_string(&command).unwrap_or_default()
            ),
            DispatchCommand::FocusNext
            | DispatchCommand::FocusPrevious
            | DispatchCommand::FocusLast
            | DispatchCommand::FocusWindow { .. }
            | DispatchCommand::ForceFocusWindow { .. }
            | DispatchCommand::FocusDirection { .. }
            | DispatchCommand::SwapNext
            | DispatchCommand::SwapPrevious
            | DispatchCommand::SwapDirection { .. } => Family::Focus,
            DispatchCommand::MoveActive { .. }
            | DispatchCommand::MoveWindow { .. }
            | DispatchCommand::ResizeWindow { .. }
            | DispatchCommand::CloseWindow { .. }
            | DispatchCommand::RaiseWindow { .. }
            | DispatchCommand::LowerWindow { .. }
            | DispatchCommand::ResizeActive { .. }
            | DispatchCommand::CloseActive
            | DispatchCommand::MinimizeActive
            | DispatchCommand::ToggleMaximized
            | DispatchCommand::ToggleFullscreen
            | DispatchCommand::ToggleWindowFullscreen { .. }
            | DispatchCommand::TogglePinned
            | DispatchCommand::ToggleWindowPinned { .. }
            | DispatchCommand::ToggleFloating
            | DispatchCommand::ToggleWindowFloating { .. }
            | DispatchCommand::SetPinned { .. }
            | DispatchCommand::SetWindowPinned { .. }
            | DispatchCommand::SetFloating { .. }
            | DispatchCommand::SetWindowFloating { .. }
            | DispatchCommand::CenterActive
            | DispatchCommand::ToggleFloatingAside
            | DispatchCommand::CenterWindow { .. }
            | DispatchCommand::RaiseActive
            | DispatchCommand::LowerActive
            | DispatchCommand::ToggleTitleBar
            | DispatchCommand::ToggleWindowTitleBar { .. }
            | DispatchCommand::ManageWindow { .. }
            | DispatchCommand::UnmanageWindow { .. }
            | DispatchCommand::RestoreWindows
            | DispatchCommand::TogglePseudotile
            | DispatchCommand::ToggleWindowPseudotile { .. } => Family::Window,
            DispatchCommand::ClearWindowTags
            | DispatchCommand::ClearSelectedWindowTags { .. }
            | DispatchCommand::AddWindowTag { .. }
            | DispatchCommand::AddSelectedWindowTag { .. }
            | DispatchCommand::RemoveWindowTag { .. }
            | DispatchCommand::RemoveSelectedWindowTag { .. }
            | DispatchCommand::ToggleWindowTag { .. }
            | DispatchCommand::ToggleSelectedWindowTag { .. }
            | DispatchCommand::EnableRule { .. }
            | DispatchCommand::DisableRule { .. }
            | DispatchCommand::ToggleRule { .. } => Family::Rules,
            DispatchCommand::SwitchWorkspaceSelector { .. }
            | DispatchCommand::MoveToWorkspaceSelector { .. }
            | DispatchCommand::MoveToWorkspaceAndFollow { .. }
            | DispatchCommand::MoveWindowToWorkspace { .. }
            | DispatchCommand::MoveWindowToWorkspaceAndFollow { .. }
            | DispatchCommand::RenameWorkspace { .. }
            | DispatchCommand::FocusMonitor { .. }
            | DispatchCommand::FocusMonitorAtCursor
            | DispatchCommand::FocusMonitorWorkspace { .. }
            | DispatchCommand::MoveWorkspaceToMonitor { .. }
            | DispatchCommand::SwapActiveWorkspaces { .. }
            | DispatchCommand::MoveWindowToMonitor { .. }
            | DispatchCommand::ToggleWindowCentered { .. }
            | DispatchCommand::MoveWindowToMonitorAndFollow { .. }
            | DispatchCommand::NewWorkspace { .. }
            | DispatchCommand::CloseWorkspace
            | DispatchCommand::EnsureWorkspaces { .. }
            | DispatchCommand::EnsureNamedWorkspace { .. }
            | DispatchCommand::ToggleWindowContainerBehaviour
            | DispatchCommand::ToggleWorkspaceWindowContainerBehaviour { .. }
            | DispatchCommand::ToggleWorkspaceFloatOverride { .. }
            | DispatchCommand::ShowDesktop { .. } => Family::Workspace,
            DispatchCommand::Retile
            | DispatchCommand::ToggleTiling
            | DispatchCommand::ToggleWorkspaceTiling { .. }
            | DispatchCommand::CycleNextLayout
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
            | DispatchCommand::ToggleDwindlePreserveSplit
            | DispatchCommand::ResizeEdge { .. }
            | DispatchCommand::ResizeAxis { .. }
            | DispatchCommand::SetResizeDelta { .. }
            | DispatchCommand::SaveResize { .. }
            | DispatchCommand::LoadResize { .. }
            | DispatchCommand::QuickSaveResize
            | DispatchCommand::QuickLoadResize
            | DispatchCommand::PromoteToMaster
            | DispatchCommand::Promote
            | DispatchCommand::PromoteFocus
            | DispatchCommand::PromoteWindow { .. } => Family::Layout,
            DispatchCommand::ToggleGroup
            | DispatchCommand::ToggleWindowGroup { .. }
            | DispatchCommand::ToggleGroupLock
            | DispatchCommand::ToggleWindowGroupLock { .. }
            | DispatchCommand::CycleGroupNext
            | DispatchCommand::CycleGroupPrevious
            | DispatchCommand::CycleWindowGroup { .. }
            | DispatchCommand::FocusGroupIndex { .. }
            | DispatchCommand::FocusWindowGroupIndex { .. }
            | DispatchCommand::MoveGroupWindow { .. }
            | DispatchCommand::MoveWindowInGroup { .. }
            | DispatchCommand::MergeGroupNext
            | DispatchCommand::MergeGroupPrevious
            | DispatchCommand::UngroupActive
            | DispatchCommand::StackAll
            | DispatchCommand::UnstackAll => Family::Group,
            DispatchCommand::ToggleExpose
            | DispatchCommand::ToggleScratchpad { .. }
            | DispatchCommand::SnapActiveToZone { .. }
            | DispatchCommand::CycleActiveZoneNext
            | DispatchCommand::CycleActiveZonePrevious
            | DispatchCommand::ScrollView { .. }
            | DispatchCommand::SetBindingSubmap { .. }
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
            | DispatchCommand::Zones { .. }
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
            | DispatchCommand::ToggleMouseFollowsFocus
            | DispatchCommand::ShowShortcuts => Family::Other,
        };
        // A command about a window or a display to work with, given while
        // the desktop is shown, brings the windows back first, and the
        // keyboard to where it was, for the command to start from.
        if self.desktop_shown() && Self::ends_shown_desktop(&command) {
            self.end_show_desktop(
                window_manager,
                monitor_manager,
                None,
                true,
                desktop::DesktopEnds::Focus,
            );
        }
        let showing_desktop = matches!(command, DispatchCommand::ShowDesktop { .. });
        let result = match family {
            Family::Focus => self.dispatch_focus(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
            Family::Window => self.dispatch_window(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
            Family::Rules => self.dispatch_rules(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
            Family::Workspace => self.dispatch_workspace(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
            Family::Layout => self.dispatch_layout(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
            Family::Group => self.dispatch_group(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
            Family::Other => self.dispatch_other(
                window_manager,
                monitor_manager,
                snap_zone_manager,
                focus_manager,
                command,
            ),
        };
        // A command that left a window of a workspace with the keyboard
        // (one gevrek does not know as a focus command: a graph's, a
        // scratchpad's window that is a workspace's now) is the person
        // working with the windows again.
        if self.desktop_shown() && !showing_desktop {
            let given = queries(window_manager)
                .active_window()
                .filter(|window| Some(*window) != focus_before);
            if given.is_some_and(|window| monitor_manager.find_window(window).is_some()) {
                self.end_show_desktop(
                    window_manager,
                    monitor_manager,
                    None,
                    false,
                    desktop::DesktopEnds::Focus,
                );
            }
        }
        // Mouse-follows-focus: when a
        // dispatch moved keyboard focus to a different window, warp the
        // cursor to its center. Focus changes caused by pointer movement
        // (focus-follows-mouse) never reach this path, so the two options
        // do not fight each other.
        if result.is_ok() && focus_manager.mouse_follows_focus() {
            let focus_after = queries(window_manager).active_window();
            if focus_after != focus_before {
                if let Some(window) = focus_after {
                    if let Ok(rect) = queries(window_manager).rect(window) {
                        try_effect(
                            window_manager,
                            Effect::WarpPointer(gevrek_model::geometry::Point {
                                x: rect.x + rect.width.div_euclid(2),
                                y: rect.y + rect.height.div_euclid(2),
                            }),
                        );
                    }
                }
            }
        }
        result
    }
}
