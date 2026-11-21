//! Single windows: floating, focus by direction, groups and their tabs, managing and
//! unmanaging, centering and zones.

use super::*;

impl Executor {
    pub(super) fn set_window_floating(
        &mut self,
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
        // A tiled window is not kept on top: that is for a floating one.
        if !floating {
            self.set_window_on_top(window_manager, monitor_manager, handle, false)?;
        }
        // (The pass that follows shows the tab that takes the place of a
        // window that left its group.)
        self.mark_layout_dirty(monitor);
        Ok(())
    }

    /// Keep `handle` above the other windows (always on top), or let it
    /// be among them again. Only a floating window is kept on top, and
    /// only one of a workspace's windows: the one kept on top before is
    /// let go. (A window gevrek does not manage floats by itself: it is
    /// kept on top as asked, whatever else is.)
    pub(super) fn set_window_on_top(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        on_top: bool,
    ) -> Result<()> {
        if on_top {
            if let Some((monitor, index)) = monitor_manager.find_window(handle) {
                if !monitor_manager.is_window_floating(handle) {
                    anyhow::bail!("Only a floating window can be kept on top");
                }
                let others: Vec<WindowHandle> = monitor_manager
                    .get_workspaces(monitor)
                    .and_then(|workspaces| workspaces.get(index))
                    .map(|workspace| workspace.all_windows())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|other| *other != handle)
                    .filter(|other| queries(window_manager).is_always_on_top(*other))
                    .collect();
                for other in others {
                    effect(
                        window_manager,
                        Effect::Stack {
                            id: other,
                            op: StackOp::AlwaysOnTop(false),
                        },
                    )?;
                }
            }
        }
        if queries(window_manager).is_always_on_top(handle) != on_top {
            effect(
                window_manager,
                Effect::Stack {
                    id: handle,
                    op: StackOp::AlwaysOnTop(on_top),
                },
            )?;
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
        log::debug!(
            "Suppressed initial focus for {:?}; restored focus to {:?}",
            excluded,
            target
        );
        Ok(true)
    }

    /// The keyboard goes to the window that had it before `from`, where
    /// one such is in sight. Returns whether it did.
    pub fn hand_keyboard_back(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        focus_manager: &mut FocusManager,
        from: WindowHandle,
    ) -> Result<bool> {
        let before = self.previous_focus_candidate(window_manager, focus_manager, from);
        self.restore_previous_focus(window_manager, monitor_manager, focus_manager, from, before)
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
                try_effect(
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
                // (While the desktop is shown it stays away, and comes
                // back with the others.)
                if self.hold_back_for_desktop(window) {
                    continue;
                }
                effect(
                    window_manager,
                    Effect::SetVisible {
                        id: window,
                        visible: true,
                    },
                )?;
                self.window_state.get_mut(window).hidden_by_wm = false;
                try_effect(
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
        log::debug!("Stacked all windows on the focused workspace");
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
        log::debug!("Unstacked the focused group into {} windows", count);
        Ok(())
    }

    pub(super) fn toggle_group_lock(
        &mut self,
        monitor_manager: &MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        if groups::toggle_lock(monitor_manager, &mut self.locked_groups, window)? {
            log::debug!("Locked selected window group");
        } else {
            log::debug!("Unlocked selected window group");
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
                try_effect(
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
            try_effect(
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
        try_effect(
            window_manager,
            Effect::Border {
                id: window,
                change: BorderChange::Remove,
            },
        );
        try_effect(
            window_manager,
            Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                id: window,
                shown: true,
            })),
        );
        try_effect(
            window_manager,
            Effect::SetVisible {
                id: window,
                visible: true,
            },
        );
        self.window_state.get_mut(window).session_unmanaged = true;
        log::debug!("Unmanaged window {:?} for this session", window);
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

    /// The workspace of the window a new window belongs to, where that
    /// workspace is not shown: the nearest window above it (a dialog's
    /// window, or the window of the dialog it is a dialog of) that is on
    /// a workspace. `None` where it belongs to no window, where that
    /// window is on no workspace (a scratchpad's, one gevrek leaves alone,
    /// one the person minimized), and where its workspace is the one its
    /// display shows: the new window then opens as any other does.
    fn parents_workspace_out_of_sight(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        window: WindowHandle,
    ) -> Option<(MonitorHandle, usize)> {
        // (A dialog's own dialog's dialog: enough for any program, and an
        // end to a ring of windows that belong to one another.)
        const DEPTH: usize = 8;
        let mut above = queries(window_manager).parent(window);
        for _ in 0..DEPTH {
            let parent = above.filter(|parent| *parent != window)?;
            if self.scratchpads.is_scratchpad_window(parent) {
                return None;
            }
            if let Some((output, index)) = monitor_manager.find_window(parent) {
                if self.window_state.get(parent).minimized {
                    return None;
                }
                let shown = monitor_manager.get_workspaces(output)?.focused_idx();
                return (shown != index).then_some((output, index));
            }
            above = queries(window_manager).parent(parent);
        }
        None
    }

    /// A window that is opening and belongs to a window whose workspace
    /// is not shown joins that workspace, out of sight with it: a dialog
    /// a program opens while the person works elsewhere is found with its
    /// program, and does not come up over what the person is doing. It
    /// floats, as what belongs to another window does, unless a rule of
    /// the person's says it is tiled. Returns whether it was taken so.
    ///
    /// Not for a window a rule of the person's sends to a workspace by
    /// name (the rule is the last word, and is carried out as for any new
    /// window), and only for the window that is opening now: one that is
    /// taken later (it was minimized when gevrek started, or is handed to
    /// the graph by a key) is in sight, and stays where the person sees it.
    pub(super) fn join_parent_out_of_sight(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<bool> {
        if self.opening != Some(window)
            || self.is_startup_phase
            || monitor_manager.find_window(window).is_some()
            || self.scratchpads.is_scratchpad_window(window)
            || self.window_state.get(window).session_unmanaged
            || queries(window_manager).ignored_by_rules(window)
            || queries(window_manager).is_decoration(window)
        {
            return Ok(false);
        }
        let Some((output, index)) =
            self.parents_workspace_out_of_sight(window_manager, monitor_manager, window)
        else {
            return Ok(false);
        };
        // (Of every rule, those that are looked at again later too: one
        // of them is carried out as soon as the window is on a workspace.)
        let sent_by_a_rule = self
            .resolve_rules(window_manager, window, None, None, RuleResolutionScope::All)
            .workspace
            .is_some();
        if sent_by_a_rule {
            return Ok(false);
        }
        let resolution = self.resolve_rules(
            window_manager,
            window,
            None,
            None,
            RuleResolutionScope::Static,
        );
        let mut changes = Changes::default();
        let adoption = lifecycle::adopt(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            window,
            Placement {
                output: Some(output),
                floating: resolution.floating.unwrap_or(true),
                workspace: Some(index),
            },
            &mut changes,
        )?;
        self.apply_changes(window_manager, changes);
        if !matches!(adoption, Adoption::Adopted { .. }) {
            return Ok(false);
        }
        self.opened_out_of_sight = true;
        log::debug!(
            "New window {window:?} belongs to a window on a workspace that is not shown: it opened there"
        );
        Ok(true)
    }

    fn adopt_window(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        just_opened: bool,
    ) -> Result<bool> {
        // (A scratchpad's window is on no workspace and stays on none:
        // hidden, then restored from being minimized, it was taken for
        // a window nobody had and given a tile.)
        if monitor_manager.find_window(window).is_some()
            || self.scratchpads.is_scratchpad_window(window)
            || self.window_state.get(window).session_unmanaged
            || queries(window_manager).ignored_by_rules(window)
            || queries(window_manager).is_decoration(window)
        {
            return Ok(false);
        }
        if just_opened && self.join_parent_out_of_sight(window_manager, monitor_manager, window)? {
            return Ok(true);
        }
        let resolution = self.resolve_rules(
            window_manager,
            window,
            None,
            None,
            RuleResolutionScope::Static,
        );
        // A rule of the person's own is the last word; where none speaks
        // of floating, the desktop says what the window is (a dialog, a
        // window of one size) and what the rules gevrek comes with say.
        // The same answer the graph's "Auto" arrangement goes by, so a
        // window floats or not whichever of the two takes it.
        let floating = resolution
            .floating
            .unwrap_or_else(|| queries(window_manager).floats_by_rules(window));
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
            workspace: None,
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
        if let Some((output, index)) = monitor_manager.find_window(window) {
            log::debug!(
                "{window:?} is taken{}: it {} on workspace {index} of {output:?}",
                if just_opened { " as it opens" } else { "" },
                if floating { "floats" } else { "is tiled" },
            );
        }
        Ok(true)
    }

    /// Carry out what a lifecycle step decided: apply its effects and queue
    /// its outputs for the once-per-wake layout.
    pub(super) fn apply_changes(&mut self, window_manager: &mut dyn Desktop, changes: Changes) {
        // Failures are logged and skipped: one window refusing must not
        // stop the rest.
        for effect in &changes.effects {
            // While the desktop is shown, what would come into sight on a
            // workspace that is shown (they are the only ones whose
            // windows are shown) stays away with the others: hidden, if
            // it is not hidden already.
            let held;
            let effect = match effect {
                Effect::SetVisible { id, visible: true } if self.hold_back_for_desktop(*id) => {
                    if window_manager.is_hidden_by_backend(*id) {
                        continue;
                    }
                    held = Effect::SetVisible {
                        id: *id,
                        visible: false,
                    };
                    &held
                }
                other => other,
            };
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
        try_effect(
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
        log::debug!("Managing window {:?} again", window);
        Ok(())
    }

    /// Bring a window to the workspace a display shows, floating and in
    /// its middle; for a window that was brought so, put it back as it
    /// was before: on its workspace, tiled or floating, at its place.
    pub(super) fn toggle_window_centered(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        display: MonitorHandle,
    ) -> Result<()> {
        let (monitor, index) = monitor_manager
            .find_window(window)
            .ok_or_else(|| anyhow::anyhow!("The selected window is not managed by gevrek"))?;

        if let Some(before) = self.window_state.get_mut(window).centered_from.take() {
            // Back: to its workspace where that is there still, else it
            // stays on the one it is on.
            let home = monitor_manager
                .get_all_monitors()
                .into_iter()
                .find_map(|monitor| {
                    monitor_manager
                        .get_workspaces(monitor.handle)?
                        .elements()
                        .iter()
                        .position(|workspace| workspace.id == before.workspace)
                        .map(|index| (monitor.handle, index))
                })
                .unwrap_or((monitor, index));
            if home != (monitor, index) {
                self.move_window_to_workspace(
                    window_manager,
                    monitor_manager,
                    window,
                    home.0,
                    home.1,
                    before.floating,
                )?;
            }
            self.set_window_floating(window_manager, monitor_manager, window, before.floating)?;
            if before.floating {
                effect(
                    window_manager,
                    Effect::Configure {
                        id: window,
                        rect: before.rect,
                        states: WindowStates::default(),
                    },
                )?;
            } else if let Some(place) = before.place {
                // Its place among the tiles (it came back as the last).
                if let Some(workspace) = monitor_manager
                    .get_workspaces_mut(home.0)
                    .and_then(|workspaces| workspaces.get_mut(home.1))
                {
                    let alone = workspace.find_container(window).is_some_and(|now| {
                        now != place
                            && workspace
                                .containers
                                .elements()
                                .get(now)
                                .is_some_and(|container| container.windows.len() == 1)
                    });
                    if alone {
                        workspace.remove_window(window);
                        let place = place.min(workspace.containers.len());
                        workspace.add_window_at(window, place);
                    }
                }
            }
            self.mark_layout_dirty(home.0);
            return Ok(());
        }

        let before = {
            let workspace = monitor_manager
                .get_workspaces(monitor)
                .and_then(|workspaces| workspaces.elements().get(index))
                .ok_or_else(|| anyhow::anyhow!("The window's workspace is not there"))?;
            crate::rule_state::BeforeCentered {
                workspace: workspace.id,
                floating: monitor_manager.is_window_floating(window),
                rect: queries(window_manager)
                    .rect(window)
                    .map_err(anyhow::Error::msg)?,
                place: workspace.find_container(window),
            }
        };
        let shown = monitor_manager
            .get_workspaces(display)
            .map(|workspaces| workspaces.focused_idx())
            .ok_or_else(|| anyhow::anyhow!("That display has no workspace"))?;
        if (display, shown) != (monitor, index) {
            self.move_window_to_workspace(
                window_manager,
                monitor_manager,
                window,
                display,
                shown,
                true,
            )?;
        }
        // (Floats it, puts it in the middle, in front, with the keyboard.)
        self.center_window(window_manager, monitor_manager, window)?;
        self.window_state.get_mut(window).centered_from = Some(before);
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
        snap_zone_manager: &mut SnapZoneManager,
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
        let zone_id = snap_zone_manager
            .get_monitor_layout(monitor)
            .zones
            .get(index)
            .map(|zone| zone.id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Zone index {} does not exist; active layout has {} zones",
                    index,
                    snap_zone_manager.zone_count(monitor)
                )
            })?;
        let rect = snap_zone_manager
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
        snap_zone_manager.assign_window_to_zone(active, monitor, zone_id);
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
        snap_zone_manager: &mut SnapZoneManager,
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
        let layout = snap_zone_manager.get_monitor_layout(monitor);
        if layout.zones.is_empty() {
            anyhow::bail!("The active monitor's zone layout is empty");
        }
        let current_index = snap_zone_manager
            .get_window_zone(active)
            .filter(|(assigned_monitor, _)| *assigned_monitor == monitor)
            .and_then(|(_, zone_id)| layout.zones.iter().position(|zone| zone.id == zone_id));
        let index = match (current_index, backwards) {
            (Some(0), true) | (None, true) => layout.zones.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % layout.zones.len(),
            (None, false) => 0,
        };
        self.snap_active_to_zone(window_manager, monitor_manager, snap_zone_manager, index)
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
        // A tiled window the person minimized is in no workspace's layout:
        // it is brought back to its own first (a press on its button in
        // the taskbar asks for this).
        let located = match located {
            None if self.window_state.get(target).minimized
                && self.window_state.get(target).minimized_origin.is_some() =>
            {
                self.bring_back_minimized_tile(window_manager, monitor_manager, target)?;
                focus::locate_target(
                    &World {
                        outputs: monitor_manager,
                        windows: &mut self.window_state,
                    },
                    target,
                    bypass_no_focus,
                )?
            }
            located => located,
        };
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

    /// A tiled window the person minimized, brought back: the workspace
    /// it left is shown on its display (so it comes back to its own, not
    /// to whichever another display shows), the system shows the window
    /// again, and it rejoins that workspace's layout as a restore told by
    /// the system does (`on_window_restored`). That the system tells of
    /// the restore afterwards changes nothing more: it is a member then.
    /// (On Windows giving the keyboard to a minimized window does not
    /// bring it back of itself.)
    fn bring_back_minimized_tile(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        target: WindowHandle,
    ) -> Result<()> {
        let origin = self.window_state.get(target).minimized_origin;
        let left = origin.and_then(|id| monitor_manager.find_workspace_by_id(id));
        if let Some((output, index)) = left {
            let shown = monitor_manager
                .get_workspaces(output)
                .is_some_and(|workspaces| workspaces.focused_idx() == index);
            if !shown {
                self.switch_workspace_on(
                    window_manager,
                    monitor_manager,
                    output,
                    WorkspaceSelector::Index(index),
                )?;
            }
            self.focused_monitor = Some(output);
        }
        if queries(window_manager).is_minimized(target) {
            effect(
                window_manager,
                Effect::State {
                    id: target,
                    change: StateChange::Minimized(false),
                },
            )?;
        }
        self.on_window_restored(window_manager, monitor_manager, target)
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
