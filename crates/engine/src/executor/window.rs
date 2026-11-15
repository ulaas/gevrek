//! Single windows: floating, focus by direction, groups and their tabs, managing and
//! unmanaging, centering and zones.

use super::*;

impl Executor {
    pub(super) fn set_window_floating(
        &self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        floating: bool,
    ) -> Result<()> {
        if monitor_manager.is_window_floating(handle) == floating {
            return Ok(());
        }
        let Some((monitor, workspace_index)) = monitor_manager.find_window(handle) else {
            return Ok(());
        };
        let work_area = monitor_manager.get_adjusted_work_area(monitor, window_manager);
        let Some(work_area) = work_area else {
            return Ok(());
        };
        if let Some(workspace) = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(workspace_index))
        {
            workspace.remove_window(handle);
            if floating {
                workspace.add_floating_window(handle);
                effect(
                    window_manager,
                    Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                        id: handle,
                        shown: true,
                    })),
                )?;
            } else {
                workspace.add_window(handle, true);
                effect(
                    window_manager,
                    Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                        id: handle,
                        shown: false,
                    })),
                )?;
            }
            self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        }
        Ok(())
    }

    /// The most recently focused window that can take focus back from
    /// `excluded`.
    pub(super) fn previous_focus_candidate(
        &self,
        window_manager: &dyn Desktop,
        focus_manager: &FocusManager,
        excluded: WindowHandle,
    ) -> Option<WindowHandle> {
        focus::previous_candidate(&self.window_state, focus_manager, excluded, |candidate| {
            queries(window_manager).exists(candidate)
                && queries(window_manager).is_visible(candidate)
        })
    }

    /// Give focus back to `target` (chosen by the focus policy) instead of
    /// `excluded`. Returns false when there is no target.
    pub(super) fn restore_previous_focus(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        focus_manager: &mut FocusManager,
        excluded: WindowHandle,
        target: Option<WindowHandle>,
    ) -> Result<bool> {
        let Some(target) = target else {
            return Ok(false);
        };
        self.focus_managed_window(window_manager, monitor_manager, target)?;
        focus_manager.record_focus(target);
        log::info!(
            "Suppressed initial focus for {:?}; restored focus to {:?}",
            excluded,
            target
        );
        Ok(true)
    }

    pub fn directional_score(
        current: crate::container::Rect,
        candidate: crate::container::Rect,
        direction: Direction,
    ) -> Option<i64> {
        let current_x = current.x as i64 + current.width as i64 / 2;
        let current_y = current.y as i64 + current.height as i64 / 2;
        let candidate_x = candidate.x as i64 + candidate.width as i64 / 2;
        let candidate_y = candidate.y as i64 + candidate.height as i64 / 2;
        let dx = candidate_x - current_x;
        let dy = candidate_y - current_y;

        let (primary, perpendicular) = match direction {
            Direction::Left if dx < 0 => (-dx, dy.abs()),
            Direction::Right if dx > 0 => (dx, dy.abs()),
            Direction::Up if dy < 0 => (-dy, dx.abs()),
            Direction::Down if dy > 0 => (dy, dx.abs()),
            _ => return None,
        };

        // Prefer alignment on the requested axis, then physical proximity.
        Some(primary * primary + perpendicular * perpendicular * 4)
    }

    pub(super) fn focus_direction(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        direction: Direction,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        let monitor = monitor_of(window_manager, monitor_manager, active)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let workspace = monitor_manager
            .get_focused_workspace(monitor.handle)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;
        // On the strip the columns are in a row, in view or not: left and
        // right are the column before and the column after.
        if workspace.layout == crate::layouts::LayoutType::Scrolling
            && matches!(direction, Direction::Left | Direction::Right)
        {
            if let Some(index) = workspace.find_container(active) {
                let columns = workspace.containers.elements();
                let target = if direction == Direction::Left {
                    columns[..index]
                        .iter()
                        .rev()
                        .find_map(|column| column.focused_window())
                } else {
                    columns[index + 1..]
                        .iter()
                        .find_map(|column| column.focused_window())
                }
                .ok_or_else(|| anyhow::anyhow!("No window exists in the requested direction"))?;
                // (Brings its column into view before it has the keyboard.)
                return self.focus_managed_window(window_manager, monitor_manager, target);
            }
        }
        let current_rect = queries(window_manager)
            .rect(active)
            .map_err(anyhow::Error::msg)?;

        let target = workspace
            .all_windows()
            .into_iter()
            .filter(|candidate| *candidate != active && !workspace.out_of_view(*candidate))
            .filter_map(|candidate| {
                let rect = queries(window_manager)
                    .rect(candidate)
                    .map_err(anyhow::Error::msg)
                    .ok()?;
                let score = Self::directional_score(current_rect, rect, direction)?;
                Some((score, candidate))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, candidate)| candidate)
            .ok_or_else(|| anyhow::anyhow!("No window exists in the requested direction"))?;

        if let Some(container_index) = workspace.find_container(target) {
            if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor.handle) {
                workspace.focus_container(container_index);
            }
        }
        effect(window_manager, Effect::Focus(Some(target)))?;
        Ok(())
    }

    pub(super) fn swap_direction(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        direction: Direction,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        let monitor = monitor_of(window_manager, monitor_manager, active)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor.handle, window_manager)
            .unwrap_or_else(|| monitor.work_area_rect());

        let (active_index, target_index) = {
            let workspace = monitor_manager
                .get_focused_workspace(monitor.handle)
                .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;
            let active_index = workspace
                .find_container(active)
                .ok_or_else(|| anyhow::anyhow!("The active window is floating"))?;
            let current_rect = workspace
                .containers
                .get(active_index)
                .map(|container| container.rect)
                .ok_or_else(|| anyhow::anyhow!("The active container no longer exists"))?;

            let target_index = if workspace.layout == crate::layouts::LayoutType::Scrolling {
                // The column before or after on the strip, in view or not.
                match direction {
                    Direction::Left => active_index.checked_sub(1),
                    Direction::Right => {
                        Some(active_index + 1).filter(|index| *index < workspace.containers.len())
                    }
                    _ => None,
                }
            } else {
                workspace
                    .containers
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != active_index)
                    .filter_map(|(index, container)| {
                        Self::directional_score(current_rect, container.rect, direction)
                            .map(|score| (score, index))
                    })
                    .min_by_key(|(score, _)| *score)
                    .map(|(_, index)| index)
            }
            .ok_or_else(|| anyhow::anyhow!("No tiled window exists in the requested direction"))?;
            (active_index, target_index)
        };

        let workspace = monitor_manager
            .get_focused_workspace_mut(monitor.handle)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;
        workspace
            .containers
            .elements_mut()
            .swap(active_index, target_index);
        workspace.containers.focus_at(target_index);
        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        // (The view may have followed the column: scrolling layout.)
        self.settle_visibility(window_manager, monitor_manager)?;
        effect(window_manager, Effect::Focus(Some(active)))?;
        Ok(())
    }

    pub(super) fn sync_group_visibility(
        &mut self,
        workspace: &crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        let mut visible = Vec::new();
        let mut hidden = Vec::new();
        for container in workspace.containers.iter() {
            // A column out of view (scrolling layout) is not shown at all.
            if container.offscreen {
                hidden.extend(container.all_windows());
                continue;
            }
            if let Some(window) = container.focused_window() {
                visible.push(window);
            }
            hidden.extend(container.non_focused_windows());
        }
        // What the Floating layout floated is on screen, also when it was
        // in a column out of view before.
        visible.extend(
            workspace
                .floating_windows
                .iter()
                .filter(|window| workspace.floated_by_layout.contains(window))
                .copied(),
        );

        // Only windows whose visibility actually changes are touched, so
        // this is cheap enough to run on every layout flush.
        for window in hidden {
            if !window_manager.is_hidden_by_backend(window) {
                self.hide_workspace_window(window_manager, window)?;
                let _ = effect(
                    window_manager,
                    Effect::Border {
                        id: window,
                        change: BorderChange::Visible(false),
                    },
                );
            }
        }
        for window in visible {
            if window_manager.is_hidden_by_backend(window)
                || self.window_state.get(window).hidden_by_wm
            {
                effect(
                    window_manager,
                    Effect::SetVisible {
                        id: window,
                        visible: true,
                    },
                )?;
                self.window_state.get_mut(window).hidden_by_wm = false;
                let _ = effect(
                    window_manager,
                    Effect::Border {
                        id: window,
                        change: BorderChange::Visible(true),
                    },
                );
            }
        }
        Ok(())
    }

    /// Merge every container on the focused workspace
    /// into a single tab group, keeping the currently focused window visible.
    pub(super) fn stack_all(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let focused = groups::stack_all(monitor_manager, &self.locked_groups, monitor)?;
        let workspace = monitor_manager
            .get_focused_workspace_mut(monitor)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;

        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        self.sync_group_visibility(workspace, window_manager)?;
        if let Some(focused) = focused {
            effect(window_manager, Effect::Focus(Some(focused)))?;
        }
        log::info!("Stacked all windows on the focused workspace");
        Ok(())
    }

    /// Dissolve the focused tab group so every window
    /// gets its own container, preserving the group order.
    pub(super) fn unstack_all(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let (focused, count) = groups::unstack_all(monitor_manager, &self.locked_groups, monitor)?;
        let workspace = monitor_manager
            .get_focused_workspace_mut(monitor)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;

        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        self.sync_group_visibility(workspace, window_manager)?;
        if let Some(focused) = focused {
            effect(window_manager, Effect::Focus(Some(focused)))?;
        }
        log::info!("Unstacked the focused group into {} windows", count);
        Ok(())
    }

    pub(super) fn toggle_group_lock(
        &mut self,
        monitor_manager: &MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        if groups::toggle_lock(monitor_manager, &mut self.locked_groups, window)? {
            log::info!("Locked selected window group");
        } else {
            log::info!("Unlocked selected window group");
        }
        Ok(())
    }

    /// Apply a tab change: on the visible workspace the other tabs hide and
    /// the selected one shows and takes focus; the stackbar follows.
    pub(super) fn show_group_tab(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        change: groups::TabChange,
    ) -> Result<()> {
        if change.location.visible {
            let mut changes = Changes::default();
            workspaces::hide_windows(&mut self.window_state, &change.hidden, &mut changes);
            self.apply_changes(window_manager, changes);
            for window in &change.hidden {
                let _ = effect(
                    window_manager,
                    Effect::Border {
                        id: *window,
                        change: BorderChange::Visible(false),
                    },
                );
            }
            let mut changes = Changes::default();
            workspaces::show_windows(&mut self.window_state, &[change.selected], &mut changes);
            self.apply_changes(window_manager, changes);
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: change.selected,
                    change: BorderChange::Visible(true),
                },
            );
            effect(window_manager, Effect::Focus(Some(change.selected)))?;
        }
        // Re-key the stackbar to the newly focused tab and repaint it.
        let workspace = monitor_manager
            .get_workspaces(change.location.output)
            .and_then(|workspaces| workspaces.get(change.location.workspace_index))
            .ok_or_else(|| anyhow::anyhow!("The selected workspace no longer exists"))?;
        Self::sync_workspace_stackbars(workspace, window_manager)
    }

    pub(super) fn cycle_group(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        forward: bool,
    ) -> Result<()> {
        let change = groups::cycle(monitor_manager, window, forward)?;
        self.show_group_tab(window_manager, monitor_manager, change)
    }

    /// Lay out a workspace whose groups changed and, when it is visible,
    /// show the right tabs and focus `window`.
    pub(super) fn settle_group_change(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        location: groups::GroupLocation,
        work_area: crate::container::Rect,
        window: WindowHandle,
    ) -> Result<()> {
        let workspace = monitor_manager
            .get_workspaces_mut(location.output)
            .and_then(|workspaces| workspaces.get_mut(location.workspace_index))
            .ok_or_else(|| anyhow::anyhow!("The selected workspace no longer exists"))?;
        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        if location.visible {
            self.sync_group_visibility(workspace, window_manager)?;
            effect(window_manager, Effect::Focus(Some(window)))?;
        }
        Ok(())
    }

    pub(super) fn merge_group(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        forward: bool,
    ) -> Result<()> {
        let location = groups::locate(monitor_manager, window)?;
        let work_area = monitor_manager
            .get_adjusted_work_area(location.output, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        groups::merge(monitor_manager, &self.locked_groups, window, forward)?;
        self.settle_group_change(window_manager, monitor_manager, location, work_area, window)
    }

    pub(super) fn ungroup_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        let location = groups::locate(monitor_manager, window)?;
        if self.locked_groups.contains(&location.key()) {
            return Err(groups::GroupError::Locked.into());
        }
        let work_area = monitor_manager
            .get_adjusted_work_area(location.output, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        groups::ungroup(monitor_manager, &self.locked_groups, window)?;
        self.settle_group_change(window_manager, monitor_manager, location, work_area, window)
    }

    pub(super) fn toggle_group(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        if groups::is_grouped(monitor_manager, window)? {
            self.ungroup_window(window_manager, monitor_manager, window)
        } else {
            self.merge_group(window_manager, monitor_manager, window, true)
        }
    }

    pub(super) fn focus_group_index(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        index: usize,
    ) -> Result<()> {
        let change = groups::focus_tab(monitor_manager, window, index)?;
        self.show_group_tab(window_manager, monitor_manager, change)
    }

    /// Focus tab `index` of the group containing `window`; used by stackbar
    /// tab clicks, which identify their container by its key window.
    pub fn focus_stackbar_tab(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        index: usize,
    ) -> Result<()> {
        self.focus_group_index(window_manager, monitor_manager, window, index)
    }

    pub(super) fn move_group_window(
        &mut self,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        forward: bool,
    ) -> Result<()> {
        Ok(groups::move_tab(monitor_manager, window, forward)?)
    }

    /// Release a managed window from tiling for this session. The window
    /// keeps its position and stays visible, but is
    /// removed from its workspace (and therefore from layout, focus cycling,
    /// and borders) until `manage_window` re-adopts it or the WM restarts.
    pub fn unmanage_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        if self.window_state.get(window).session_unmanaged {
            anyhow::bail!("The selected window is already unmanaged");
        }
        let (source, _) = monitor_manager
            .find_window(window)
            .ok_or_else(|| anyhow::anyhow!("The selected window is not managed by gevrek"))?;
        let was_floating = monitor_manager.is_window_floating(window);
        monitor_manager.find_and_remove_window(window);
        if !was_floating {
            self.mark_layout_dirty(source);
        }
        let _ = effect(
            window_manager,
            Effect::Border {
                id: window,
                change: BorderChange::Remove,
            },
        );
        let _ = effect(
            window_manager,
            Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                id: window,
                shown: true,
            })),
        );
        let _ = effect(
            window_manager,
            Effect::SetVisible {
                id: window,
                visible: true,
            },
        );
        self.window_state.get_mut(window).session_unmanaged = true;
        log::info!("Unmanaged window {:?} for this session", window);
        Ok(())
    }

    /// Re-adopt a window into tiling: session-unmanaged
    /// windows return to their monitor's active workspace, and windows that
    /// were never managed (for example through an `Ignore` rule) are adopted
    /// and evaluated against the window rules for the first time.
    /// Adopt an unmanaged window into the focused workspace of its monitor.
    ///
    /// This is the default management path: every new
    /// manageable window becomes managed even when the automation graph does
    /// not route it. Rules still win — a float resolution keeps the window
    /// floating, and windows released with `unmanage` stay unmanaged. Returns
    /// true when the window was adopted by this call.
    pub fn adopt_window_by_default(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<bool> {
        self.adopt_window(window_manager, monitor_manager, window, false)
    }

    /// As `adopt_window_by_default`, for a window that has just opened.
    /// Where the system opens a window on an output of its own choosing
    /// (`Desktop::new_windows_go_where_the_keyboard_is`: Windows, where a
    /// program opens where it last was), one that is to be tiled goes to
    /// the output with the keyboard: that is where it was asked for. One
    /// that floats (a dialog) stays on the output it opened on.
    pub fn adopt_new_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<bool> {
        self.adopt_window(window_manager, monitor_manager, window, true)
    }

    fn adopt_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        just_opened: bool,
    ) -> Result<bool> {
        if monitor_manager.find_window(window).is_some()
            || self.window_state.get(window).session_unmanaged
            || queries(window_manager).ignored_by_rules(window)
            || queries(window_manager).is_decoration(window)
        {
            return Ok(false);
        }
        let resolution = self.resolve_rules(
            window_manager,
            window,
            None,
            None,
            RuleResolutionScope::Static,
        );
        let floating = resolution.floating == Some(true);
        let where_it_is =
            monitor_of(window_manager, monitor_manager, window).map(|monitor| monitor.handle);
        let with_keyboard =
            just_opened && !floating && window_manager.new_windows_go_where_the_keyboard_is();
        let placement = Placement {
            output: if with_keyboard {
                self.active_monitor(window_manager, monitor_manager)
                    .or(where_it_is)
            } else {
                where_it_is.or_else(|| self.active_monitor(window_manager, monitor_manager))
            },
            floating,
        };
        let mut changes = Changes::default();
        let adoption = lifecycle::adopt(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            window,
            placement,
            &mut changes,
        )?;
        self.apply_changes(window_manager, changes);
        let Adoption::Adopted { first, .. } = adoption else {
            return Ok(false);
        };
        if first {
            self.apply_initial_window_rules(window_manager, monitor_manager, window)?;
        }
        log::info!(
            "Adopted new window {:?} by default (no graph routing)",
            window
        );
        Ok(true)
    }

    /// Carry out what a lifecycle step decided: apply its effects and queue
    /// its outputs for the once-per-wake layout.
    pub(super) fn apply_changes(&mut self, window_manager: &mut dyn Desktop, changes: Changes) {
        // Failures are logged and skipped: one window refusing must not
        // stop the rest.
        for effect in &changes.effects {
            if let Err(error) = window_manager.apply(effect) {
                log::debug!("Effect {effect:?} failed: {error:#}");
            }
        }
        for output in changes.relayout {
            self.mark_layout_dirty(output);
        }
    }

    pub fn manage_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        if monitor_manager.find_window(window).is_some() {
            self.window_state.get_mut(window).session_unmanaged = false;
            anyhow::bail!("The selected window is already managed");
        }
        let monitor = monitor_of(window_manager, monitor_manager, window)
            .map(|monitor| monitor.handle)
            .or_else(|| self.active_monitor(window_manager, monitor_manager))
            .ok_or_else(|| {
                anyhow::anyhow!("Could not determine a monitor for the selected window")
            })?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        {
            let workspace = monitor_manager
                .get_focused_workspace_mut(monitor)
                .ok_or_else(|| anyhow::anyhow!("The monitor has no active workspace"))?;
            workspace.add_window(window, true);
            self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        }
        let first_adoption = !self.window_state.get(window).known;
        self.window_state.get_mut(window).session_unmanaged = false;
        self.window_state.get_mut(window).known = true;
        let _ = effect(
            window_manager,
            Effect::SetVisible {
                id: window,
                visible: true,
            },
        );
        self.window_state.get_mut(window).hidden_by_wm = false;
        if first_adoption {
            self.apply_initial_window_rules(window_manager, monitor_manager, window)?;
        }
        log::info!("Managing window {:?} again", window);
        Ok(())
    }

    pub(super) fn center_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        let monitor = monitor_manager
            .find_window(window)
            .map(|(monitor, _)| monitor)
            .ok_or_else(|| anyhow::anyhow!("The selected window is not managed by gevrek"))?;

        // Centering is a floating-window operation. Promote tiled windows to
        // floating first so the action has a durable, unsurprising result.
        self.set_window_floating(window_manager, monitor_manager, window, true)?;
        let rect = queries(window_manager)
            .rect(window)
            .map_err(anyhow::Error::msg)?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let x = work_area.x + (work_area.width - rect.width).div_euclid(2);
        let y = work_area.y + (work_area.height - rect.height).div_euclid(2);
        effect(
            window_manager,
            Effect::Configure {
                id: window,
                rect: gevrek_model::geometry::Rect {
                    x,
                    y,
                    width: rect.width,
                    height: rect.height,
                },
                states: WindowStates::default(),
            },
        )?;
        effect(
            window_manager,
            Effect::Stack {
                id: window,
                op: StackOp::Raise,
            },
        )?;
        effect(window_manager, Effect::Focus(Some(window)))?;
        Ok(())
    }

    /// Execute a typed runtime command. This is the common dispatcher used by
    /// the gRPC API, CLI, and editor command center.
    pub(super) fn snap_active_to_zone(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        index: usize,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        let monitor = monitor_manager
            .find_window(active)
            .map(|(monitor, _)| monitor)
            .or_else(|| {
                monitor_of(window_manager, monitor_manager, active).map(|monitor| monitor.handle)
            })
            .or_else(|| self.active_monitor(window_manager, monitor_manager))
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let zone_id = zone_manager
            .get_monitor_layout(monitor)
            .zones
            .get(index)
            .map(|zone| zone.id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Zone index {} does not exist; active layout has {} zones",
                    index,
                    zone_manager.zone_count(monitor)
                )
            })?;
        let rect = zone_manager
            .snap_to_zone_index(monitor, index, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not resolve zone geometry"))?;
        self.set_window_floating(window_manager, monitor_manager, active, true)?;
        effect(
            window_manager,
            Effect::Configure {
                id: active,
                rect: gevrek_model::geometry::Rect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                },
                states: WindowStates::default(),
            },
        )?;
        zone_manager.assign_window_to_zone(active, monitor, zone_id);
        effect(
            window_manager,
            Effect::SetVisible {
                id: active,
                visible: true,
            },
        )?;
        effect(window_manager, Effect::Focus(Some(active)))?;
        Ok(())
    }

    /// The display whose strip a wheel or fingers would move: the one the
    /// pointer is on, else the one with the keyboard.
    fn strip_output(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
    ) -> Option<MonitorHandle> {
        window_manager
            .output_at_cursor()
            .filter(|output| monitor_manager.get_monitor(*output).is_some())
            .or_else(|| self.active_monitor(window_manager, monitor_manager))
    }

    /// Whether the display the pointer is on shows a strip whose view can
    /// be moved by itself.
    pub fn strip_under_pointer(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
    ) -> bool {
        self.strip_output(window_manager, monitor_manager)
            .and_then(|output| monitor_manager.get_focused_workspace(output))
            .is_some_and(|workspace| workspace.scrolls_freely() && !workspace.is_empty())
    }

    /// Move the view along the strip (the scrolling layout) of the
    /// display the pointer is on, else of the one with the keyboard. A
    /// column that leaves the view altogether hands the keyboard to the
    /// one in the middle. Returns whether it moved.
    pub fn scroll_view(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        pixels: i32,
    ) -> bool {
        let Some(monitor) = self.strip_output(window_manager, monitor_manager) else {
            return false;
        };
        let Some(work_area) = monitor_manager.get_adjusted_work_area(monitor, window_manager)
        else {
            return false;
        };
        let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor) else {
            return false;
        };
        let before = workspace.focused_window();
        if !workspace.scroll_view(pixels, work_area) {
            return false;
        }
        let after = workspace.focused_window();
        self.mark_layout_dirty(monitor);
        // The keyboard goes along only when it was on the column that left.
        if after != before && before.is_some() && queries(window_manager).active_window() == before
        {
            if let Err(error) = effect(window_manager, Effect::Focus(after)) {
                log::debug!("Scrolling: the keyboard did not go along: {error}");
            }
        }
        true
    }

    pub(super) fn cycle_active_zone(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        backwards: bool,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        let monitor = monitor_manager
            .find_window(active)
            .map(|(monitor, _)| monitor)
            .or_else(|| {
                monitor_of(window_manager, monitor_manager, active).map(|monitor| monitor.handle)
            })
            .or_else(|| self.active_monitor(window_manager, monitor_manager))
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let layout = zone_manager.get_monitor_layout(monitor);
        if layout.zones.is_empty() {
            anyhow::bail!("The active monitor's zone layout is empty");
        }
        let current_index = zone_manager
            .get_window_zone(active)
            .filter(|(assigned_monitor, _)| *assigned_monitor == monitor)
            .and_then(|(_, zone_id)| layout.zones.iter().position(|zone| zone.id == zone_id));
        let index = match (current_index, backwards) {
            (Some(0), true) | (None, true) => layout.zones.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % layout.zones.len(),
            (None, false) => 0,
        };
        self.snap_active_to_zone(window_manager, monitor_manager, zone_manager, index)
    }

    pub fn focus_managed_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: WindowHandle,
    ) -> Result<()> {
        self.focus_managed_window_with_rule_bypass(window_manager, monitor_manager, target, false)
    }

    /// Shared focus path. With `bypass_no_focus`, the `NoFocus` rule effect
    /// is skipped for this single explicit action;
    /// the rule keeps applying to future automatic focus decisions.
    pub fn focus_managed_window_with_rule_bypass(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: WindowHandle,
        bypass_no_focus: bool,
    ) -> Result<()> {
        let located = focus::locate_target(
            &World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            target,
            bypass_no_focus,
        )?;
        if let Some(found) = located {
            self.focused_monitor = Some(found.output);
            if found.needs_switch {
                self.switch_workspace_selector(
                    window_manager,
                    monitor_manager,
                    WorkspaceSelector::Index(found.workspace),
                )?;
            }
            // Front its tab: the container's other tabs hide, it shows.
            let mut changes = Changes::default();
            focus::bring_forward(
                &mut World {
                    outputs: monitor_manager,
                    windows: &mut self.window_state,
                },
                target,
                found.output,
                &mut changes,
            )?;
            self.apply_changes(window_manager, changes);
            // Under the scrolling layout the view follows the keyboard:
            // the window's column is brought into view (and shown) before
            // it is given the keyboard.
            let scrolls = monitor_manager
                .get_focused_workspace(found.output)
                .is_some_and(|workspace| {
                    workspace.layout == crate::layouts::LayoutType::Scrolling
                        && workspace.find_container(target).is_some()
                });
            if scrolls {
                self.mark_layout_dirty(found.output);
                self.flush_dirty_monitor_layouts(window_manager, monitor_manager)?;
            }
        }
        if queries(window_manager).is_minimized(target) {
            effect(
                window_manager,
                Effect::State {
                    id: target,
                    change: StateChange::Minimized(false),
                },
            )?;
            self.window_state.get_mut(target).minimized = false;
        }
        effect(window_manager, Effect::Focus(Some(target)))
    }
}

impl Executor {
    /// The display a window that has just opened is tiled on, where the
    /// system opens windows on a display of its own choosing
    /// (`Desktop::new_windows_go_where_the_keyboard_is`): the one with
    /// the keyboard. None where the system's own choice stands: on other
    /// systems, for a window gevrek has already (it is being moved, not
    /// opened), and for one that will float (a dialog stays by its
    /// program).
    pub(super) fn keyboard_output_for_new_tile(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        window: WindowHandle,
        mode: &gevrek_model::graph::WindowMode,
    ) -> Option<crate::outputs::MonitorHandle> {
        use gevrek_model::graph::WindowMode;
        if !window_manager.new_windows_go_where_the_keyboard_is()
            || monitor_manager.find_window(window).is_some()
        {
            return None;
        }
        let floats = match mode {
            WindowMode::Floating => true,
            WindowMode::Auto => {
                queries(window_manager).floats_by_rules(window)
                    || queries(window_manager)
                        .windows()
                        .iter()
                        .find(|known| known.handle == window)
                        .is_none_or(|known| known.title.is_empty())
            }
            _ => false,
        };
        if floats {
            return None;
        }
        self.active_monitor(window_manager, monitor_manager)
    }
}
