//! Workspaces and monitors: creating, switching, renaming, closing, moving windows and
//! workspaces between them.

use super::*;

/// A workspace of a display, for an overview of all of them (see
/// [`Executor::workspaces_laid_out`]).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceView {
    /// Its place among the display's workspaces.
    pub index: usize,
    pub name: String,
    /// It is the one the display shows.
    pub shown: bool,
    /// Its windows, with their places.
    pub windows: Vec<(WindowHandle, crate::container::Rect)>,
}

/// What a change to the workspace before or after would show (see
/// [`Executor::workspace_beside`]).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceBeside {
    /// The display whose workspace would change.
    pub output: MonitorHandle,
    /// The room its windows have there.
    pub area: crate::container::Rect,
    /// 1: towards the next workspace, -1: towards the one before.
    pub direction: i32,
    /// The windows shown now, with their places.
    pub going: Vec<(WindowHandle, crate::container::Rect)>,
    /// The windows of the other workspace, with their places there.
    pub coming: Vec<(WindowHandle, crate::container::Rect)>,
}

impl Executor {
    pub(super) fn emit_workspace_event(
        &self,
        kind: RuntimeEventKind,
        monitor: MonitorHandle,
        name: &str,
        persistent: bool,
    ) {
        let Some(broadcast) = &self.node_event_broadcast else {
            return;
        };
        broadcast(NodeExecutionEvent::Runtime(RuntimeExtensionEvent {
            kind,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            window_handle: None,
            fields: [
                ("monitor".to_string(), monitor.0.to_string()),
                ("name".to_string(), name.to_string()),
                ("persistent".to_string(), persistent.to_string()),
            ]
            .into_iter()
            .collect(),
        }));
    }

    pub(super) fn emit_workspace_renamed(
        &self,
        monitor: MonitorHandle,
        workspace_id: usize,
        old_name: &str,
        new_name: &str,
    ) {
        let Some(broadcast) = &self.node_event_broadcast else {
            return;
        };
        broadcast(NodeExecutionEvent::Runtime(RuntimeExtensionEvent {
            kind: RuntimeEventKind::WorkspaceRenamed,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            window_handle: None,
            fields: [
                ("monitor".to_string(), monitor.0.to_string()),
                ("workspace_id".to_string(), workspace_id.to_string()),
                ("old_name".to_string(), old_name.to_string()),
                ("new_name".to_string(), new_name.to_string()),
            ]
            .into_iter()
            .collect(),
        }));
    }

    pub(super) fn create_workspace_for_selector(
        &mut self,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
        name: String,
        fallback_layout: crate::layouts::LayoutType,
        run_created_empty: bool,
    ) -> Result<usize> {
        let profile = self
            .workspace_profiles
            .get(&name.trim().to_ascii_lowercase())
            .cloned();
        let layout = profile
            .as_ref()
            .map(|profile| profile.layout)
            .unwrap_or(fallback_layout);
        let workspace_name = profile
            .as_ref()
            .map(|profile| profile.name.clone())
            .unwrap_or(name);
        let index = monitor_manager
            .create_workspace(monitor, workspace_name.clone(), layout)
            .ok_or_else(|| {
                anyhow::anyhow!("Could not create named workspace '{workspace_name}'")
            })?;
        if let Some(profile) = &profile {
            if let Some(workspace) = monitor_manager
                .get_workspaces_mut(monitor)
                .and_then(|workspaces| workspaces.get_mut(index))
            {
                profile.apply_to_workspace(
                    workspace,
                    &gevrek_model::theme::dressed(
                        &self.workspace_appearance,
                        self.wallpaper_accent,
                    ),
                );
            }
            if run_created_empty {
                if let Some(launch) = &profile.on_created_empty {
                    // Started at the next layout flush, where the desktop
                    // is at hand (within the same loop pass).
                    self.pending_workspace_launches
                        .push((profile.name.clone(), launch.clone()));
                }
            }
        }
        self.emit_workspace_event(
            RuntimeEventKind::WorkspaceCreated,
            monitor,
            &workspace_name,
            profile.as_ref().is_some_and(|profile| profile.persistent),
        );
        Ok(index)
    }

    /// Whether a workspace profile (config, not graph) defines `name`.
    /// Whether the graph that runs now creates a workspace of this name on
    /// this output.
    pub fn graph_creates_workspace(&self, output: MonitorHandle, name: &str) -> bool {
        self.graph_workspaces.keys().any(|(monitor, graph_name)| {
            *monitor == output && graph_name.eq_ignore_ascii_case(name)
        })
    }

    pub fn has_workspace_profile(&self, name: &str) -> bool {
        self.workspace_profiles
            .contains_key(&name.trim().to_ascii_lowercase())
    }

    pub(super) fn hide_workspace_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        handle: WindowHandle,
    ) -> Result<()> {
        let mut changes = Changes::default();
        workspaces::hide_windows(&mut self.window_state, &[handle], &mut changes);
        self.apply_changes(window_manager, changes);
        Ok(())
    }

    /// Restore every window Gevrek hid (workspace switching, group tabs,
    /// scratchpads). Used both by the shutdown path and by the
    /// `RestoreWindows` dispatch. Returns the
    /// number of windows that were actually shown again.
    pub fn restore_hidden_windows(&mut self, window_manager: &mut dyn Desktop) -> usize {
        let mut handles = self.window_state.ids_where(|state| state.hidden_by_wm);
        handles.extend(self.scratchpads.hidden_windows());
        handles.sort_by_key(|h| h.0);
        handles.dedup();

        let mut restored = 0;
        for handle in handles {
            if queries(window_manager).exists(handle)
                && effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: true,
                    },
                )
                .is_ok()
            {
                let _ = effect(
                    window_manager,
                    Effect::Border {
                        id: handle,
                        change: BorderChange::Visible(true),
                    },
                );
                restored += 1;
            }
            self.window_state.get_mut(handle).hidden_by_wm = false;
        }

        self.scratchpads.mark_all_visible();

        // Windows hidden outside the tracked set (graph workspace/cloak
        // nodes) are still in the crash-safe journal.
        restored + window_manager.restore_journaled_windows()
    }

    pub(super) fn focus_workspace_window_by_offset(
        workspace: &mut crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
        active_window: WindowHandle,
        offset: isize,
    ) -> Result<Option<WindowHandle>> {
        let windows: Vec<WindowHandle> = workspace
            .all_windows()
            .into_iter()
            .filter(|handle| {
                queries(window_manager).exists(*handle)
                    && !queries(window_manager).is_decoration(*handle)
                    && !queries(window_manager).is_minimized(*handle)
            })
            .collect();

        if windows.is_empty() {
            return Ok(None);
        }

        let current_idx = windows.iter().position(|handle| *handle == active_window);

        let next_idx = match current_idx {
            Some(idx) => {
                let len = windows.len() as isize;
                ((idx as isize + offset).rem_euclid(len)) as usize
            }
            None if offset < 0 => windows.len() - 1,
            None => 0,
        };

        let next_window = windows[next_idx];
        if let Some(container_idx) = workspace.find_container(next_window) {
            workspace.focus_container(container_idx);
        }

        effect(
            window_manager,
            Effect::SetVisible {
                id: next_window,
                visible: true,
            },
        )?;
        effect(window_manager, Effect::Focus(Some(next_window)))?;
        let _ = effect(
            window_manager,
            Effect::Border {
                id: next_window,
                change: BorderChange::Visible(true),
            },
        );
        Ok(Some(next_window))
    }

    pub(super) fn window_workspace_context(
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
    ) -> (Option<String>, Option<usize>) {
        let Some((monitor, index)) = monitor_manager.find_window(handle) else {
            return (None, None);
        };
        let name = monitor_manager
            .get_workspaces(monitor)
            .and_then(|workspaces| workspaces.get(index))
            .map(|workspace| workspace.name.clone());
        (name, Some(index))
    }

    pub(super) fn target_monitor(
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        monitor_index: Option<usize>,
    ) -> Option<MonitorHandle> {
        monitor_index
            .and_then(|index| {
                monitor_manager
                    .get_all_monitors_ordered()
                    .get(index)
                    .map(|monitor| monitor.handle)
            })
            .or_else(|| {
                monitor_manager
                    .find_window(handle)
                    .map(|(monitor, _)| monitor)
            })
            .or_else(|| monitor_manager.get_primary_monitor_handle())
    }

    pub(super) fn move_window_to_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        target_monitor: MonitorHandle,
        target_workspace: usize,
        floating: bool,
    ) -> Result<()> {
        let target_exists = monitor_manager
            .get_workspaces(target_monitor)
            .map(|workspaces| target_workspace < workspaces.len())
            .unwrap_or(false);
        if !target_exists {
            anyhow::bail!(
                "Rule target workspace {} does not exist on monitor {:?}",
                target_workspace,
                target_monitor
            );
        }
        let mut changes = Changes::default();
        let Some(moved) = workspaces::move_window(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            handle,
            target_monitor,
            target_workspace,
            Some(floating),
            &mut changes,
        )?
        else {
            return Ok(());
        };
        if let Some((source_monitor, _)) = moved.from {
            self.mark_layout_dirty(source_monitor);
        }
        // Shown on the visible workspace, hidden (as a WM hide) otherwise.
        self.apply_changes(window_manager, changes);
        if moved.visible {
            self.mark_layout_dirty(target_monitor);
        }
        Ok(())
    }

    pub(super) fn active_monitor(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
    ) -> Option<MonitorHandle> {
        focus::active_output(
            monitor_manager,
            self.focused_monitor,
            || queries(window_manager).active_window(),
            |window| {
                monitor_of(window_manager, monitor_manager, window).map(|monitor| monitor.handle)
            },
        )
    }

    /// Show the windows of the workspace a monitor shows now and lay it out
    /// (after the shown workspace was changed in the model alone).
    pub fn show_focused_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
    ) -> Result<()> {
        self.reveal_focused_workspace(window_manager, monitor_manager, monitor)
            .map(|_| ())
    }

    pub(super) fn reveal_focused_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
    ) -> Result<Option<WindowHandle>> {
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine a monitor work area"))?;
        // Show, then lay out: windows are positioned once they are visible
        // (some apps ignore or defer moves while hidden).
        let mut changes = Changes::default();
        let reveal = workspaces::reveal(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            monitor,
            &mut changes,
        )
        .map_err(|_| anyhow::anyhow!("The monitor has no active workspace"))?;
        self.apply_changes(window_manager, changes);
        for window in &reveal.hidden {
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: *window,
                    change: BorderChange::Visible(false),
                },
            );
        }
        if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor) {
            self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        }
        Ok(reveal.focus)
    }

    pub(super) fn translate_floating_windows(
        window_manager: &mut dyn Desktop,
        windows: &[WindowHandle],
        delta_x: i32,
        delta_y: i32,
    ) {
        for window in windows {
            if let Ok(rect) = queries(window_manager)
                .rect(*window)
                .map_err(anyhow::Error::msg)
            {
                let _ = effect(
                    window_manager,
                    Effect::Configure {
                        id: *window,
                        rect: gevrek_model::geometry::Rect {
                            x: rect.x + delta_x,
                            y: rect.y + delta_y,
                            width: rect.width,
                            height: rect.height,
                        },
                        states: WindowStates::default(),
                    },
                );
            }
        }
    }

    /// Give the keyboard to the window an output would focus. An output
    /// with nothing to focus (its workspace is empty) takes the keyboard
    /// away from a window of another output: the keys go where the command
    /// went, and with nobody there they are gevrek's alone. A window that
    /// belongs to no output (a launcher, a menu) keeps them.
    fn focus_on_output(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        output: MonitorHandle,
    ) -> Result<()> {
        if let Some(window) = focus::output_focus_target(monitor_manager, output) {
            return effect(window_manager, Effect::Focus(Some(window)));
        }
        let elsewhere = queries(window_manager)
            .active_window()
            .and_then(|window| monitor_manager.find_window(window))
            .is_some_and(|(other, _)| other != output);
        if elsewhere {
            effect(window_manager, Effect::Focus(None))?;
        }
        Ok(())
    }

    pub(super) fn focus_monitor(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: MonitorTarget,
    ) -> Result<()> {
        let (_, target_monitor) =
            self.resolve_monitor_target(window_manager, monitor_manager, &target)?;
        self.focused_monitor = Some(target_monitor);
        self.focus_on_output(window_manager, monitor_manager, target_monitor)?;
        log::info!("Focused monitor using selector '{}'", target.expression());
        Ok(())
    }

    /// Focus the monitor containing the mouse cursor, including the focused window of that
    /// monitor's active workspace.
    pub(super) fn focus_monitor_at_cursor(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let monitor = monitor_at_cursor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor under the cursor"))?
            .handle;
        self.focused_monitor = Some(monitor);
        self.focus_on_output(window_manager, monitor_manager, monitor)?;
        log::info!("Focused the monitor under the cursor");
        Ok(())
    }

    /// Focus a specific workspace on a specific monitor. The workspace selector resolves against
    /// the target monitor's ring; other monitors are left untouched.
    pub(super) fn focus_monitor_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: MonitorTarget,
        workspace: WorkspaceSelector,
    ) -> Result<()> {
        let (_, monitor) = self.resolve_monitor_target(window_manager, monitor_manager, &target)?;
        // switch_workspace_selector operates on the active monitor; pointing
        // the logical monitor focus at the target first makes it resolve the
        // selector on the target monitor's ring.
        self.focused_monitor = Some(monitor);
        self.switch_workspace_selector(window_manager, monitor_manager, workspace.clone())?;
        // Even when the workspace was already active, land keyboard focus on
        // its window so the command always focuses the target monitor.
        self.focus_on_output(window_manager, monitor_manager, monitor)?;
        log::info!(
            "Focused workspace '{}' on monitor '{}'",
            workspace.expression(),
            target.expression()
        );
        Ok(())
    }

    pub(super) fn move_workspace_to_monitor(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        target: MonitorTarget,
    ) -> Result<()> {
        let (source, target_monitor) =
            self.resolve_monitor_target(window_manager, monitor_manager, &target)?;
        if source == target_monitor {
            return Ok(());
        }
        let source_area = monitor_manager
            .get_configured_work_area(source)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the source monitor work area"))?;
        let target_area = monitor_manager
            .get_configured_work_area(target_monitor)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the target monitor work area"))?;
        let (workspace_id, workspace_windows, floating) = monitor_manager
            .get_focused_workspace(source)
            .map(|workspace| {
                (
                    workspace.id,
                    workspace.all_windows(),
                    workspace
                        .floating_windows
                        .iter()
                        .copied()
                        .collect::<Vec<_>>(),
                )
            })
            .ok_or_else(|| anyhow::anyhow!("The source monitor has no active workspace"))?;
        let replaced_windows = monitor_manager
            .get_focused_workspace(target_monitor)
            .map(|workspace| workspace.all_windows())
            .unwrap_or_default();
        for window in replaced_windows {
            self.hide_workspace_window(window_manager, window)?;
        }
        monitor_manager.move_focused_workspace(source, target_monitor)?;
        groups::move_workspace_locks(
            &mut self.locked_groups,
            workspace_id,
            source,
            target_monitor,
        );
        for window in workspace_windows {
            zone_manager.remove_window(window);
        }
        Self::translate_floating_windows(
            window_manager,
            &floating,
            target_area.x - source_area.x,
            target_area.y - source_area.y,
        );
        let _ = self.reveal_focused_workspace(window_manager, monitor_manager, source)?;
        let focus =
            self.reveal_focused_workspace(window_manager, monitor_manager, target_monitor)?;
        self.focused_monitor = Some(target_monitor);
        if let Some(window) = focus {
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        log::info!(
            "Moved active workspace to monitor using selector '{}'",
            target.expression()
        );
        Ok(())
    }

    pub(super) fn swap_active_workspaces(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        target: MonitorTarget,
    ) -> Result<()> {
        let (source, target_monitor) =
            self.resolve_monitor_target(window_manager, monitor_manager, &target)?;
        if source == target_monitor {
            return Ok(());
        }
        let source_area = monitor_manager
            .get_configured_work_area(source)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the source monitor work area"))?;
        let target_area = monitor_manager
            .get_configured_work_area(target_monitor)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the target monitor work area"))?;
        let (source_workspace_id, source_windows, source_floating) = monitor_manager
            .get_focused_workspace(source)
            .map(|workspace| {
                (
                    workspace.id,
                    workspace.all_windows(),
                    workspace
                        .floating_windows
                        .iter()
                        .copied()
                        .collect::<Vec<_>>(),
                )
            })
            .ok_or_else(|| anyhow::anyhow!("The source monitor has no active workspace"))?;
        let (target_workspace_id, target_windows, target_floating) = monitor_manager
            .get_focused_workspace(target_monitor)
            .map(|workspace| {
                (
                    workspace.id,
                    workspace.all_windows(),
                    workspace
                        .floating_windows
                        .iter()
                        .copied()
                        .collect::<Vec<_>>(),
                )
            })
            .ok_or_else(|| anyhow::anyhow!("The target monitor has no active workspace"))?;
        monitor_manager.swap_focused_workspaces(source, target_monitor)?;
        groups::move_workspace_locks(
            &mut self.locked_groups,
            source_workspace_id,
            source,
            target_monitor,
        );
        groups::move_workspace_locks(
            &mut self.locked_groups,
            target_workspace_id,
            target_monitor,
            source,
        );
        for window in source_windows.into_iter().chain(target_windows) {
            zone_manager.remove_window(window);
        }
        Self::translate_floating_windows(
            window_manager,
            &source_floating,
            target_area.x - source_area.x,
            target_area.y - source_area.y,
        );
        Self::translate_floating_windows(
            window_manager,
            &target_floating,
            source_area.x - target_area.x,
            source_area.y - target_area.y,
        );
        let source_focus =
            self.reveal_focused_workspace(window_manager, monitor_manager, source)?;
        let _ = self.reveal_focused_workspace(window_manager, monitor_manager, target_monitor)?;
        self.focused_monitor = Some(source);
        if let Some(window) = source_focus {
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        log::info!(
            "Swapped active workspaces using monitor selector '{}'",
            target.expression()
        );
        Ok(())
    }

    pub fn resolve_workspace_selector(
        &mut self,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
        selector: &WorkspaceSelector,
        run_created_empty: bool,
    ) -> Result<usize> {
        let (current, len) = monitor_manager
            .get_workspaces(monitor)
            .map(|workspaces| (workspaces.focused_idx(), workspaces.len()))
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no workspace ring"))?;

        let requested = match selector {
            WorkspaceSelector::Index(index) => {
                if *index >= MAX_DYNAMIC_WORKSPACES {
                    anyhow::bail!(
                        "Workspace index {} exceeds the dynamic workspace limit {}",
                        index,
                        MAX_DYNAMIC_WORKSPACES - 1
                    );
                }
                while monitor_manager
                    .get_workspaces(monitor)
                    .map(|workspaces| workspaces.len() <= *index)
                    .unwrap_or(false)
                {
                    let number = monitor_manager
                        .get_workspaces(monitor)
                        .map(|workspaces| workspaces.len() + 1)
                        .unwrap_or(1);
                    self.create_workspace_for_selector(
                        monitor_manager,
                        monitor,
                        format!("Workspace {number}"),
                        crate::layouts::LayoutType::BSP,
                        run_created_empty,
                    )?;
                }
                *index
            }
            WorkspaceSelector::Name(name) => {
                if let Some(index) =
                    monitor_manager
                        .get_workspaces(monitor)
                        .and_then(|workspaces| {
                            workspaces
                                .elements()
                                .iter()
                                .position(|workspace| workspace.name.eq_ignore_ascii_case(name))
                        })
                {
                    index
                } else {
                    self.create_workspace_for_selector(
                        monitor_manager,
                        monitor,
                        name.clone(),
                        crate::layouts::LayoutType::BSP,
                        run_created_empty,
                    )?
                }
            }
            WorkspaceSelector::Next => {
                if len == 0 {
                    anyhow::bail!("The active monitor has no workspaces");
                }
                (current + 1) % len
            }
            WorkspaceSelector::Previous => {
                if len == 0 {
                    anyhow::bail!("The active monitor has no workspaces");
                }
                (current + len - 1) % len
            }
            WorkspaceSelector::PreviousVisited => monitor_manager
                .get_previous_workspace_index(monitor)
                .filter(|index| {
                    monitor_manager
                        .get_workspaces(monitor)
                        .is_some_and(|workspaces| *index < workspaces.len())
                })
                .ok_or_else(|| anyhow::anyhow!("No previously visited workspace is available"))?,
            WorkspaceSelector::FirstEmpty => {
                if let Some(index) =
                    monitor_manager
                        .get_workspaces(monitor)
                        .and_then(|workspaces| {
                            workspaces
                                .elements()
                                .iter()
                                .position(|workspace| workspace.all_windows().is_empty())
                        })
                {
                    index
                } else {
                    let number = len + 1;
                    self.create_workspace_for_selector(
                        monitor_manager,
                        monitor,
                        format!("Workspace {number}"),
                        crate::layouts::LayoutType::BSP,
                        run_created_empty,
                    )?
                }
            }
            WorkspaceSelector::Relative(offset) => {
                if len == 0 {
                    anyhow::bail!("The active monitor has no workspaces");
                }
                (current as i64 + i64::from(*offset)).rem_euclid(len as i64) as usize
            }
        };
        Ok(requested)
    }

    pub(super) fn switch_workspace_selector(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        selector: WorkspaceSelector,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        self.switch_workspace_on(window_manager, monitor_manager, monitor, selector)
    }

    /// What a change to the workspace before or after would show, without
    /// changing anything: for fingers on a touchpad that pull that
    /// workspace into view before the change is decided. The windows of
    /// that workspace are laid out for it (they stay hidden). `None` for
    /// a selector that is not "the next", "the one before" or so many
    /// along, and where that is the workspace shown already.
    pub fn workspace_beside(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        selector: &WorkspaceSelector,
    ) -> Option<WorkspaceBeside> {
        let direction = match selector {
            WorkspaceSelector::Previous | WorkspaceSelector::Relative(i32::MIN..=-1) => -1,
            WorkspaceSelector::Next | WorkspaceSelector::Relative(1..) => 1,
            _ => return None,
        };
        let monitor = self.active_monitor(window_manager, monitor_manager)?;
        let current = monitor_manager.get_workspaces(monitor)?.focused_idx();
        let target = self
            .resolve_workspace_selector(monitor_manager, monitor, selector, false)
            .ok()?;
        if target == current {
            return None;
        }
        let work_area = monitor_manager.get_adjusted_work_area(monitor, window_manager)?;
        let places = |workspace: &crate::workspace::Workspace, window_manager: &dyn Desktop| {
            let mut places = Vec::new();
            for container in workspace.containers.iter() {
                if let Some(window) = container.focused_window() {
                    if !container.offscreen {
                        places.push((window, container.rect));
                    }
                }
            }
            for window in workspace.floating_windows.iter().copied() {
                if let Ok(rect) = queries(window_manager).rect(window) {
                    places.push((window, rect));
                }
            }
            places
        };
        let going = places(
            monitor_manager.get_focused_workspace(monitor)?,
            window_manager,
        );
        let coming = {
            let workspace = monitor_manager
                .get_workspaces_mut(monitor)?
                .get_mut(target)?;
            if let Err(error) =
                self.apply_workspace_layout_and_position(workspace, work_area, window_manager)
            {
                log::debug!("The workspace beside could not be laid out: {error}");
            }
            places(workspace, window_manager)
        };
        Some(WorkspaceBeside {
            output: monitor,
            area: work_area,
            direction,
            going,
            coming,
        })
    }

    /// Every workspace of a display with its windows at the places they
    /// have there, the hidden ones laid out for it (they stay hidden):
    /// for an overview of all of them at once.
    pub fn workspaces_laid_out(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
    ) -> Vec<WorkspaceView> {
        let Some(work_area) = monitor_manager.get_adjusted_work_area(monitor, window_manager)
        else {
            return Vec::new();
        };
        let Some((shown, count)) = monitor_manager
            .get_workspaces(monitor)
            .map(|workspaces| (workspaces.focused_idx(), workspaces.len()))
        else {
            return Vec::new();
        };
        let mut views = Vec::new();
        for index in 0..count {
            let Some(workspace) = monitor_manager
                .get_workspaces_mut(monitor)
                .and_then(|workspaces| workspaces.get_mut(index))
            else {
                continue;
            };
            if index != shown {
                if let Err(error) =
                    self.apply_workspace_layout_and_position(workspace, work_area, window_manager)
                {
                    log::debug!("A workspace could not be laid out for the overview: {error}");
                }
            }
            let mut windows = Vec::new();
            for container in workspace.containers.iter() {
                if let Some(window) = container.focused_window() {
                    if !container.offscreen {
                        windows.push((window, container.rect));
                    }
                }
            }
            for window in workspace.floating_windows.iter().copied() {
                if let Ok(rect) = queries(window_manager).rect(window) {
                    windows.push((window, rect));
                }
            }
            views.push(WorkspaceView {
                index,
                name: workspace.name.clone(),
                shown: index == shown,
                windows,
            });
        }
        views
    }

    /// Show a workspace of a display by its place among that display's
    /// workspaces, and give that display the keyboard.
    pub fn show_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
        index: usize,
    ) -> Result<()> {
        self.focused_monitor = Some(monitor);
        self.switch_workspace_on(
            window_manager,
            monitor_manager,
            monitor,
            WorkspaceSelector::Index(index),
        )?;
        self.focus_on_output(window_manager, monitor_manager, monitor)
    }

    /// The "switch workspace" command. As `switch_workspace_selector`, but
    /// a workspace asked for by name is gone to where it is: when the
    /// active monitor has none of that name and another monitor has, that
    /// monitor gets the focus and shows it. Only a name no monitor has,
    /// or one the configuration gives the active monitor too, makes a new
    /// workspace (on the active monitor).
    pub(super) fn switch_workspace_wherever_it_is(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        selector: WorkspaceSelector,
    ) -> Result<()> {
        let elsewhere = match &selector {
            WorkspaceSelector::Name(name) => {
                let has = |monitor: MonitorHandle| {
                    monitor_manager
                        .get_workspaces(monitor)
                        .is_some_and(|workspaces| {
                            workspaces
                                .elements()
                                .iter()
                                .any(|workspace| workspace.name.eq_ignore_ascii_case(name))
                        })
                };
                // A workspace of the configuration that this monitor is to
                // have, and has not made yet (one that is not persistent
                // is made when it is first asked for), is made here: it is
                // not looked for on the monitor that happens to have made
                // its own of that name already.
                let ordered = monitor_manager.get_all_monitors_ordered();
                let own_by_profile = |monitor: MonitorHandle| {
                    self.workspace_profiles
                        .get(&name.trim().to_ascii_lowercase())
                        .is_some_and(|profile| {
                            profile.enabled
                                && crate::config_apply::monitor_selector_targets(
                                    &profile.monitor,
                                    &ordered,
                                )
                                .contains(&monitor)
                        })
                };
                self.active_monitor(window_manager, monitor_manager)
                    .filter(|active| !has(*active) && !own_by_profile(*active))
                    .and_then(|active| {
                        monitor_manager
                            .get_all_monitors_ordered()
                            .into_iter()
                            .map(|monitor| monitor.handle)
                            .find(|monitor| *monitor != active && has(*monitor))
                    })
            }
            _ => None,
        };
        let Some(monitor) = elsewhere else {
            return self.switch_workspace_selector(window_manager, monitor_manager, selector);
        };
        self.focused_monitor = Some(monitor);
        self.switch_workspace_on(window_manager, monitor_manager, monitor, selector)?;
        // Also when it was that monitor's visible workspace already: the
        // keyboard goes where the command went.
        self.focus_on_output(window_manager, monitor_manager, monitor)?;
        Ok(())
    }

    /// Switch the visible workspace of `monitor`. A workspace the selector
    /// names that does not exist yet is created (dynamic workspaces).
    pub(super) fn switch_workspace_on(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
        selector: WorkspaceSelector,
    ) -> Result<()> {
        let target = self.resolve_workspace_selector(monitor_manager, monitor, &selector, true)?;
        // Floating windows that were put out of the way are where they
        // belong when the workspace is seen next.
        if monitor_manager
            .get_workspaces(monitor)
            .is_some_and(|workspaces| workspaces.focused_idx() != target)
        {
            if let Some(leaving) = monitor_manager.get_focused_workspace_mut(monitor) {
                Self::bring_floating_back(leaving, window_manager)?;
            }
        }
        // Hide the current workspace and focus the target.
        let mut changes = Changes::default();
        let Some(current) = workspaces::begin_switch(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            monitor,
            target,
            &mut changes,
        )?
        else {
            return Ok(());
        };
        // (The windows hidden now go out as the others come in.)
        window_manager.workspace_gives_way();
        self.apply_changes(window_manager, changes);

        // Lay the new workspace out while it is still hidden, then start the
        // slide from those rects and show it.
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let transition_rects = {
            let workspace = monitor_manager
                .get_focused_workspace_mut(monitor)
                .ok_or_else(|| anyhow::anyhow!("The selected workspace disappeared"))?;
            self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
            let mut transition_rects = Vec::new();
            for window in workspace.floating_windows.iter().copied() {
                if let Ok(rect) = queries(window_manager)
                    .rect(window)
                    .map_err(anyhow::Error::msg)
                {
                    transition_rects.push((window, rect));
                }
            }
            for container in workspace.containers.iter() {
                if let Some(window) = container.focused_window() {
                    transition_rects.push((window, container.rect));
                }
            }
            transition_rects
        };
        let direction = match &selector {
            WorkspaceSelector::Previous | WorkspaceSelector::Relative(i32::MIN..=-1) => -1,
            WorkspaceSelector::Next | WorkspaceSelector::Relative(1..) => 1,
            _ if target < current => -1,
            _ => 1,
        };
        let _ = window_manager.begin_workspace_transition(
            &transition_rects,
            direction * work_area.width,
            0,
        )?;
        let mut changes = Changes::default();
        let reveal = workspaces::reveal(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            monitor,
            &mut changes,
        )?;
        self.apply_changes(window_manager, changes);
        if let Some(window) = reveal.focus {
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        log::info!(
            "Switched workspace using selector '{}'",
            selector.expression()
        );
        self.prune_non_persistent_workspace(monitor_manager, monitor, current);
        Ok(())
    }

    pub(super) fn resolve_existing_workspace_selector(
        &self,
        monitor_manager: &MonitorManager,
        monitor: MonitorHandle,
        selector: &WorkspaceSelector,
    ) -> Result<usize> {
        Ok(workspaces::resolve_existing_workspace(
            monitor_manager,
            monitor,
            selector,
        )?)
    }

    pub fn rename_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        selector: WorkspaceSelector,
        name: String,
    ) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("Workspace name cannot be empty");
        }
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        // (A workspace named by its name is found on whichever display
        // has it: a panel renames the workspace of its own display, which
        // need not be the one with the keyboard.)
        let (monitor, index) =
            match self.resolve_existing_workspace_selector(monitor_manager, monitor, &selector) {
                Ok(index) => (monitor, index),
                Err(error) => {
                    let WorkspaceSelector::Name(wanted) = &selector else {
                        return Err(error);
                    };
                    monitor_manager
                        .get_all_monitors()
                        .iter()
                        .find_map(|other| {
                            let at = monitor_manager
                                .get_workspaces(other.handle)?
                                .iter()
                                .position(|workspace| {
                                    workspace.name.eq_ignore_ascii_case(wanted.trim())
                                })?;
                            Some((other.handle, at))
                        })
                        .ok_or(error)?
                }
            };
        let Some(workspaces::Renamed {
            workspace_id,
            old_name,
            windows,
        }) = workspaces::rename_workspace(monitor_manager, monitor, index, name)?
        else {
            return Ok(());
        };
        for window in windows {
            self.reevaluate_dynamic_rules(window_manager, monitor_manager, window)?;
        }
        self.emit_workspace_renamed(monitor, workspace_id, &old_name, name);
        // A workspace that every display has by one name (it is written
        // down once, for all of them) has the new name on every display:
        // else the others would keep a workspace nothing is written of.
        let others: Vec<(MonitorHandle, usize)> = monitor_manager
            .get_all_monitors()
            .iter()
            .map(|other| other.handle)
            .filter(|other| *other != monitor)
            .filter_map(|other| {
                let at = monitor_manager
                    .get_workspaces(other)?
                    .iter()
                    .position(|workspace| workspace.name.eq_ignore_ascii_case(&old_name))?;
                Some((other, at))
            })
            .collect();
        for (other, at) in others {
            match workspaces::rename_workspace(monitor_manager, other, at, name) {
                Ok(Some(renamed)) => {
                    for window in renamed.windows {
                        self.reevaluate_dynamic_rules(window_manager, monitor_manager, window)?;
                    }
                    self.emit_workspace_renamed(other, renamed.workspace_id, &old_name, name);
                }
                Ok(None) => {}
                Err(error) => log::debug!("Workspace '{old_name}' keeps its name on {other:?}: {error}"),
            }
        }
        log::info!(
            "Renamed workspace '{}' to '{}' using selector '{}'",
            old_name,
            name,
            selector.expression()
        );
        Ok(())
    }

    pub fn prune_non_persistent_workspace(
        &mut self,
        monitor_manager: &mut MonitorManager,
        monitor: MonitorHandle,
        index: usize,
    ) {
        let removable = monitor_manager
            .get_workspaces(monitor)
            .and_then(|workspaces| workspaces.get(index))
            .and_then(|workspace| {
                let profile = self
                    .workspace_profiles
                    .get(&workspace.name.trim().to_ascii_lowercase())?;
                (!profile.persistent && workspace.all_windows().is_empty())
                    .then_some(workspace.name.clone())
            });
        let Some(name) = removable else {
            return;
        };
        if monitor_manager.remove_inactive_empty_workspace(monitor, index) {
            log::info!("Removed empty non-persistent workspace '{}'", name);
            self.emit_workspace_event(RuntimeEventKind::WorkspaceDestroyed, monitor, &name, false);
        }
    }

    /// Materialize a workspace on the focused
    /// monitor now — named (honoring a matching workspace profile) when
    /// `name` is given, otherwise the next `Workspace N` index. The view
    /// does not switch to the new workspace.
    pub fn new_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        name: Option<String>,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let name = match name {
            Some(name) => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("Workspace name cannot be empty");
                }
                trimmed.to_string()
            }
            None => {
                let number = monitor_manager
                    .get_workspaces(monitor)
                    .map(|workspaces| workspaces.len() + 1)
                    .unwrap_or(1);
                format!("Workspace {number}")
            }
        };
        let index = self.create_workspace_for_selector(
            monitor_manager,
            monitor,
            name.clone(),
            crate::layouts::LayoutType::BSP,
            true,
        )?;
        log::info!("Created workspace '{}' at index {}", name, index);
        Ok(())
    }

    /// Close the focused workspace on the active
    /// monitor when it is empty, failing loudly while it still holds
    /// windows or is the monitor's last workspace.
    pub fn close_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let closed = workspaces::close_focused_workspace(monitor_manager, monitor)?;
        let persistent = self
            .workspace_profiles
            .get(&closed.trim().to_ascii_lowercase())
            .is_some_and(|profile| profile.persistent);
        self.emit_workspace_event(
            RuntimeEventKind::WorkspaceDestroyed,
            monitor,
            &closed,
            persistent,
        );
        // Reveal the workspace that shifted into the focused slot.
        let focus = self.reveal_focused_workspace(window_manager, monitor_manager, monitor)?;
        if let Some(window) = focus {
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        Ok(())
    }

    /// Create `Workspace N` workspaces on the
    /// selected monitor until it has at least `count`.
    pub fn ensure_workspaces(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: MonitorTarget,
        count: usize,
    ) -> Result<()> {
        if count == 0 || count > MAX_DYNAMIC_WORKSPACES {
            anyhow::bail!("Workspace count must be between 1 and {MAX_DYNAMIC_WORKSPACES}");
        }
        let current = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let monitor = Self::resolve_monitor_target_from(monitor_manager, current, &target)?;
        let mut created = 0usize;
        while monitor_manager
            .get_workspaces(monitor)
            .map(|workspaces| workspaces.len())
            .unwrap_or(0)
            < count
        {
            let number = monitor_manager
                .get_workspaces(monitor)
                .map(|workspaces| workspaces.len() + 1)
                .unwrap_or(1);
            self.create_workspace_for_selector(
                monitor_manager,
                monitor,
                format!("Workspace {number}"),
                crate::layouts::LayoutType::BSP,
                true,
            )?;
            created += 1;
        }
        log::info!(
            "Monitor {:?} now has at least {} workspace(s) ({} created)",
            monitor,
            count,
            created
        );
        Ok(())
    }

    /// Create a workspace named `name` on
    /// the selected monitor when none exists yet.
    pub fn ensure_named_workspace(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: MonitorTarget,
        name: String,
        layout: Option<crate::layouts::LayoutType>,
    ) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("Workspace name cannot be empty");
        }
        let current = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let monitor = Self::resolve_monitor_target_from(monitor_manager, current, &target)?;
        let exists = monitor_manager
            .get_workspaces(monitor)
            .is_some_and(|workspaces| {
                workspaces
                    .iter()
                    .any(|workspace| workspace.name.eq_ignore_ascii_case(name))
            });
        if exists {
            log::info!(
                "Workspace '{}' already exists on monitor {:?}",
                name,
                monitor
            );
            // A layout that was named is the workspace's from now on.
            if let Some(layout) = layout {
                let changed = monitor_manager
                    .get_workspaces_mut(monitor)
                    .and_then(|workspaces| {
                        workspaces
                            .iter_mut()
                            .find(|workspace| workspace.name.eq_ignore_ascii_case(name))
                    })
                    .is_some_and(|workspace| {
                        let changed = workspace.layout != layout;
                        workspace.layout = layout;
                        changed
                    });
                if changed {
                    self.retile_focused_workspaces(window_manager, monitor_manager)?;
                }
            }
            return Ok(());
        }
        self.create_workspace_for_selector(
            monitor_manager,
            monitor,
            name.to_string(),
            layout.unwrap_or(crate::layouts::LayoutType::BSP),
            true,
        )?;
        log::info!(
            "Ensured workspace '{}' exists on monitor {:?}",
            name,
            monitor
        );
        Ok(())
    }

    /// Toggle the focused
    /// workspace between Column (new windows get their own container) and
    /// Append (new windows join the focused container). Only future window
    /// adds are affected, so no re-layout is needed.
    pub(super) fn toggle_window_container_behaviour(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let workspace = monitor_manager
            .get_focused_workspace_mut(monitor)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;
        let behaviour = workspace.toggle_window_container_behaviour();
        log::info!(
            "Window container behaviour for workspace '{}' is now {}",
            workspace.name,
            behaviour.display_name()
        );
        Ok(())
    }

    /// Toggle the
    /// container behaviour of the selected workspace on the active monitor.
    pub(super) fn toggle_workspace_window_container_behaviour(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        selector: WorkspaceSelector,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let index =
            self.resolve_existing_workspace_selector(monitor_manager, monitor, &selector)?;
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(index))
            .ok_or_else(|| anyhow::anyhow!("The selected workspace no longer exists"))?;
        let behaviour = workspace.toggle_window_container_behaviour();
        log::info!(
            "Window container behaviour for workspace '{}' is now {} (selector '{}')",
            workspace.name,
            behaviour.display_name(),
            selector.expression()
        );
        Ok(())
    }

    pub(super) fn move_window_to_workspace_selector(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        selector: WorkspaceSelector,
        follow: bool,
    ) -> Result<()> {
        let (monitor, source_index) = monitor_manager
            .find_window(window)
            .ok_or_else(|| anyhow::anyhow!("The selected window is not assigned to a workspace"))?;
        let target_index =
            self.resolve_workspace_selector(monitor_manager, monitor, &selector, false)?;
        if source_index == target_index {
            return Ok(());
        }

        let mut changes = Changes::default();
        workspaces::move_window(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            window,
            monitor,
            target_index,
            None,
            &mut changes,
        )?;

        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        for index in [source_index, target_index] {
            let workspace = monitor_manager
                .get_workspaces_mut(monitor)
                .and_then(|workspaces| workspaces.get_mut(index))
                .ok_or_else(|| anyhow::anyhow!("Workspace {} is not available", index))?;
            self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        }
        // Shown on the visible workspace, hidden otherwise.
        self.apply_changes(window_manager, changes);
        log::info!(
            "Moved selected window {:?} using workspace selector '{}' ({})",
            window,
            selector.expression(),
            if follow { "follow" } else { "silent" }
        );
        if follow {
            // Switch to the exact workspace resolved before the move. Resolving
            // selectors such as `empty` again after inserting the window could
            // select (or create) a different workspace.
            self.switch_workspace_selector(
                window_manager,
                monitor_manager,
                WorkspaceSelector::Index(target_index),
            )?;
            // The window that was moved is the one the user is working
            // with: it has the keyboard there, not the window that
            // workspace had it on last (the next key would act on that
            // one, a second move would take the wrong window).
            if let Some(workspace) = monitor_manager
                .get_workspaces_mut(monitor)
                .and_then(|workspaces| workspaces.get_mut(target_index))
            {
                workspace.focus_window(window);
            }
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        Ok(())
    }

    /// Move a window to the focused workspace on another monitor. `next`/`previous` monitor
    /// selectors resolve relative to the window's own monitor. With `follow`,
    /// focus moves to the destination monitor; otherwise the current view is
    /// untouched.
    pub fn move_window_to_monitor(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        target: MonitorTarget,
        follow: bool,
    ) -> Result<()> {
        let (source, source_index) = monitor_manager
            .find_window(window)
            .ok_or_else(|| anyhow::anyhow!("The selected window is not assigned to a workspace"))?;
        let target_monitor = Self::resolve_monitor_target_from(monitor_manager, source, &target)?;
        if source == target_monitor {
            return Ok(());
        }
        let target_index = monitor_manager
            .get_workspaces(target_monitor)
            .map(|workspaces| workspaces.focused_idx())
            .ok_or_else(|| anyhow::anyhow!("The target monitor has no workspaces"))?;

        let mut changes = Changes::default();
        let moved = workspaces::move_window(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            window,
            target_monitor,
            target_index,
            None,
            &mut changes,
        )?;
        let was_floating = moved.is_some_and(|moved| moved.floating);

        // Re-layout the vacated source workspace and the receiving one.
        for (monitor, index) in [(source, source_index), (target_monitor, target_index)] {
            if let Some(work_area) = monitor_manager.get_adjusted_work_area(monitor, window_manager)
            {
                if let Some(workspace) = monitor_manager
                    .get_workspaces_mut(monitor)
                    .and_then(|workspaces| workspaces.get_mut(index))
                {
                    self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
                }
            }
        }
        // Floating windows keep their size; translate them onto the target
        // monitor the same way whole-workspace moves do.
        if was_floating {
            if let (Some(source_area), Some(target_area)) = (
                monitor_manager.get_configured_work_area(source),
                monitor_manager.get_configured_work_area(target_monitor),
            ) {
                Self::translate_floating_windows(
                    window_manager,
                    &[window],
                    target_area.x - source_area.x,
                    target_area.y - source_area.y,
                );
            }
        }
        // The target is its monitor's visible workspace: shown.
        self.apply_changes(window_manager, changes);
        if follow {
            self.focused_monitor = Some(target_monitor);
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        log::info!(
            "Moved window {:?} to monitor using selector '{}' ({})",
            window,
            target.expression(),
            if follow { "follow" } else { "silent" }
        );
        Ok(())
    }

    /// Put the windows of a saved session back on the workspaces they were
    /// on, and show the workspace each output showed. Only windows that are
    /// managed now and are still the same window (same id, class and program) are
    /// moved; a saved workspace that no longer exists is skipped. Returns
    /// how many windows were moved.
    pub fn restore_session(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        state: &crate::persistence::WindowManagerState,
    ) -> usize {
        let workspace_exists = |outputs: &MonitorManager, monitor, index: usize| {
            outputs
                .get_workspaces(monitor)
                .is_some_and(|workspaces| workspaces.get(index).is_some())
        };
        let mut moved = 0;
        for saved in &state.windows {
            let (Some(monitor), Some(saved_index)) = (saved.monitor, saved.workspace_id) else {
                continue;
            };
            // By name when one was saved: the order of workspaces may have
            // changed since. A name that is gone means the workspace is gone.
            let index = match &saved.workspace_name {
                Some(name) => {
                    let found = monitor_manager
                        .get_workspaces(monitor)
                        .and_then(|workspaces| {
                            workspaces
                                .elements()
                                .iter()
                                .position(|workspace| &workspace.name == name)
                        });
                    match found {
                        Some(index) => index,
                        None => continue,
                    }
                }
                None => saved_index,
            };
            if !queries(window_manager).exists(saved.handle)
                || queries(window_manager).class(saved.handle) != saved.class
                || queries(window_manager).exe(saved.handle) != saved.exe
            {
                continue;
            }
            let Some(current) = monitor_manager.find_window(saved.handle) else {
                continue;
            };
            if current == (monitor, index) || !workspace_exists(monitor_manager, monitor, index) {
                continue;
            }
            match self.move_window_to_workspace(
                window_manager,
                monitor_manager,
                saved.handle,
                monitor,
                index,
                saved.is_floating,
            ) {
                Ok(()) => moved += 1,
                Err(error) => log::warn!(
                    "Could not put window {:?} back on its workspace: {}",
                    saved.handle,
                    error
                ),
            }
        }
        for (monitor, index) in &state.focused_workspaces {
            let showing = monitor_manager
                .get_workspaces(*monitor)
                .map(|workspaces| workspaces.focused_idx());
            if showing.is_none()
                || showing == Some(*index)
                || !workspace_exists(monitor_manager, *monitor, *index)
            {
                continue;
            }
            if let Err(error) = self.switch_workspace_on(
                window_manager,
                monitor_manager,
                *monitor,
                WorkspaceSelector::Index(*index),
            ) {
                log::warn!(
                    "Could not show the saved workspace on {:?}: {}",
                    monitor,
                    error
                );
            }
        }
        moved
    }
}
