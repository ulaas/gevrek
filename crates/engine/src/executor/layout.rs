//! Laying out workspaces and placing their windows; resizing, promoting, the resize-state
//! file, and switching tiling on and off.

use super::*;

impl Executor {
    pub(super) fn batch_move_resize_and_sync_borders(
        window_manager: &mut dyn Desktop,
        operations: &[(WindowHandle, crate::container::Rect)],
    ) -> Result<()> {
        window_manager.apply_animated_rectangles(
            operations,
            crate::animation_scheduler::TransitionScope::Layout,
        )
    }

    /// Mark a monitor as needing layout re-application.
    /// The runtime calls `flush_pending_layouts` once per tick to coalesce multiple events.
    pub fn mark_layout_dirty(&mut self, monitor: crate::outputs::MonitorHandle) {
        self.pending_layout_monitors.insert(monitor);
    }

    /// Apply pending layouts for all dirty monitors, then clear the set.
    /// Call this once per runtime tick AFTER all events have been processed.
    /// This coalesces N events into 1 layout pass per monitor, preventing layout storms.
    pub fn flush_pending_layouts(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        self.flush_dirty_monitor_layouts(window_manager, monitor_manager)?;
        self.keep_desktop_clear(window_manager, monitor_manager);
        for (workspace, launch) in std::mem::take(&mut self.pending_workspace_launches) {
            match window_manager.launch(&launch) {
                Ok(pid) => log::info!(
                    "Workspace '{}' created empty; launched process {}",
                    workspace,
                    pid
                ),
                Err(error) => log::error!(
                    "Workspace '{}' on-created-empty launch failed: {}",
                    workspace,
                    error
                ),
            }
        }
        for handle in std::mem::take(&mut self.pending_window_enters) {
            if queries(window_manager).exists(handle)
                && window_manager.start_configured_window_enter(handle)?
            {
                log::debug!("Started configured entrance animation for {:?}", handle);
            }
        }
        Ok(())
    }

    pub(super) fn flush_dirty_monitor_layouts(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        if self.pending_layout_monitors.is_empty() {
            return self.settle_visibility(window_manager, monitor_manager);
        }

        let monitors: Vec<crate::outputs::MonitorHandle> =
            self.pending_layout_monitors.drain().collect();

        for monitor_handle in monitors {
            let work_area = if let Some(area) =
                monitor_manager.get_adjusted_work_area(monitor_handle, window_manager)
            {
                area
            } else {
                continue;
            };

            if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                // Skip layout flush while expose mode is active — expose manages positions directly
                if self.expose_workspaces.contains(&workspace.id) {
                    log::trace!(
                        "Skipping pending layout flush for monitor {:?} (expose active)",
                        monitor_handle
                    );
                    // (Still to be done: laid out when the overview is left.)
                    self.pending_layout_monitors.insert(monitor_handle);
                    continue;
                }
                // Show before layout: a removal (close, minimize, app hide)
                // can leave a container whose new focused tab is still
                // hidden; it must be visible before it is positioned.
                self.sync_group_visibility(workspace, window_manager)?;
                self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
                log::trace!("Flushed pending layout for monitor {:?}", monitor_handle);
            }
        }

        self.settle_visibility(window_manager, monitor_manager)
    }

    /// Where a layout pass changed which columns are out of view
    /// (scrolling layout), show and hide the windows of the workspaces on
    /// screen to match, and put them in their places (a window is not
    /// always moved while it is hidden). A workspace that is not on screen
    /// keeps its mark: its windows are hidden as it is, and are shown as
    /// the layout says when it is switched to (`workspaces::reveal_of`).
    pub(super) fn settle_visibility(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        for (monitor, _) in monitor_manager.get_monitors_with_handles() {
            let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor) else {
                continue;
            };
            if !workspace.visibility_stale || self.expose_workspaces.contains(&workspace.id) {
                continue;
            }
            workspace.visibility_stale = false;
            self.sync_group_visibility(workspace, window_manager)?;
            Self::batch_position_workspace_windows(workspace, window_manager)?;
        }
        Ok(())
    }

    /// Batch position all windows in a workspace using atomic window positioning
    /// This is much smoother than positioning windows one by one
    pub(super) fn batch_position_workspace_windows(
        workspace: &crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        use crate::container::Rect;

        // Collect all window positions
        let mut operations: Vec<(WindowHandle, Rect)> = Vec::new();

        // A visible stackbar reserves the top strip of its container:
        // the container's windows get the layout rect minus
        // the bar height.
        let stackbar_height = if workspace.stackbar.enabled {
            workspace.stackbar_seen().height
        } else {
            0
        };

        for container in workspace.containers.iter() {
            // Skip containers with invalid/zero rects (inactive containers)
            if container.rect.width == 0 || container.rect.height == 0 {
                continue;
            }

            let rect = if stackbar_height > 0 && container.windows.len() >= 2 {
                let (x, y, width, height) = stackbar_content_rect(
                    (
                        container.rect.x,
                        container.rect.y,
                        container.rect.width,
                        container.rect.height,
                    ),
                    stackbar_height,
                );
                Rect::new(x, y, width, height)
            } else {
                container.rect
            };

            // Invisible borders: reserve a
            // transparent band around every tiled window, exactly like the
            // space a border of that width would need, but draw nothing.
            let rect = if workspace.invisible_borders.enabled {
                let (x, y, width, height) = crate::decoration::invisible_border_content_rect(
                    (rect.x, rect.y, rect.width, rect.height),
                    workspace.seen(workspace.invisible_borders.size),
                );
                Rect::new(x, y, width, height)
            } else {
                rect
            };

            // Add focused window
            if let Some(window_handle) = container.focused_window() {
                operations.push((window_handle, rect));
            }

            // Add stacked windows
            for stacked_window in container.non_focused_windows() {
                operations.push((stacked_window, rect));
            }
        }

        if operations.is_empty() {
            return Ok(());
        }

        log::trace!("Batch positioning {} windows atomically", operations.len());

        // Use batch positioning for smooth, flicker-free updates.
        Self::batch_move_resize_and_sync_borders(window_manager, &operations)?;

        // A new border is drawn as focused only for the window that has
        // the focus now. (The window the workspace remembers as its focused
        // one may not have it: the focus can be on a floating window or on
        // another monitor, and then two borders would look focused.)
        let focused_window = window_manager
            .active_window()
            .filter(|active| workspace.focused_window() == Some(*active));
        for (window_handle, rect) in operations {
            // Invisible borders reserve the band but never draw a border;
            // the `else` hides any border left over from before the toggle.
            if workspace.border_enabled && !workspace.invisible_borders.enabled {
                let displayed = if window_manager.is_animation_event_suppressed(window_handle) {
                    queries(window_manager)
                        .rect(window_handle)
                        .map_err(anyhow::Error::msg)
                        .unwrap_or(rect)
                } else {
                    rect
                };
                // A border is decoration: failing to draw one (e.g. a window
                // owned by a protected process) must not fail the layout.
                let decorated = if !queries(window_manager).has_border(window_handle) {
                    // (A tiled window: the workspace's own border.)
                    let look = workspace.border_look(false);
                    window_manager.decorate_window_at(
                        window_handle,
                        displayed,
                        &look,
                        focused_window == Some(window_handle),
                    )
                } else {
                    window_manager.update_border_position_explicit(
                        window_handle,
                        displayed.x,
                        displayed.y,
                        displayed.width,
                        displayed.height,
                    )
                };
                if let Err(error) = decorated {
                    log::warn!(
                        "Could not draw the border of {:?}: {}",
                        window_handle,
                        error
                    );
                }
            } else {
                try_effect(
                    window_manager,
                    Effect::Border {
                        id: window_handle,
                        change: BorderChange::Visible(false),
                    },
                );
            }
        }

        Self::sync_workspace_stackbars(workspace, window_manager)?;

        Ok(())
    }

    /// Build the desired stackbar state for every grouped container on the
    /// workspace and reconcile the live stackbar windows with it: bars are
    /// created for containers holding 2+ windows, updated/repositioned as
    /// tabs, focus, or rects change, and destroyed when the group dissolves,
    /// group focus re-keys the bar, or the stackbar is disabled.
    pub(super) fn sync_workspace_stackbars(
        workspace: &crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        let desired: Vec<StackbarContainerState> = groups::stackbars(workspace)
            .into_iter()
            .map(|bar| StackbarContainerState {
                key: bar.key,
                rect: (bar.rect.x, bar.rect.y, bar.rect.width, bar.rect.height),
                tabs: bar.tabs,
                focused_tab: bar.focused_tab,
            })
            .collect();
        window_manager.sync_stackbars(&desired, &workspace.stackbar_seen())
    }

    /// Refresh stackbar tab text after a window's metadata (title) changed.
    /// No-op unless the window sits in a group on a workspace with the
    /// stackbar enabled.
    pub(super) fn refresh_stackbar_titles(
        &self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        window: WindowHandle,
    ) {
        let Some((monitor, workspace_index)) = monitor_manager.find_window(window) else {
            return;
        };
        let Some(workspace) = monitor_manager
            .get_workspaces(monitor)
            .and_then(|workspaces| workspaces.get(workspace_index))
        else {
            return;
        };
        if !workspace.stackbar.enabled {
            return;
        }
        let Some(container_index) = workspace.find_container(window) else {
            return;
        };
        let is_grouped = workspace
            .containers
            .get(container_index)
            .is_some_and(|container| container.windows.len() >= 2);
        if is_grouped {
            let _ = Self::sync_workspace_stackbars(workspace, window_manager);
        }
    }

    pub(super) fn apply_workspace_layout_and_position(
        &self,
        workspace: &mut crate::workspace::Workspace,
        work_area: crate::container::Rect,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        // While tiling is suspended globally, or disabled for this workspace,
        // windows stay exactly where they are: membership still changes, but
        // the layout engine never repositions them.
        if self.tiling_suspended || !workspace.tiling_enabled {
            return Self::settle_floating_layout(workspace, work_area, window_manager);
        }
        Self::layout_and_position_workspace(workspace, work_area, window_manager)
    }

    /// Apply the layout and position windows even while tiling is suspended.
    /// This is the forced path used by `Retile`; per-workspace
    /// `tiling_enabled` is still honored inside `Workspace::apply_layout`.
    pub(super) fn layout_and_position_workspace(
        workspace: &mut crate::workspace::Workspace,
        work_area: crate::container::Rect,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        Self::settle_floating_layout(workspace, work_area, window_manager)?;
        // What the windows' programs say of their smallest sizes now.
        workspace.smallest_sizes = workspace
            .containers
            .iter()
            .flat_map(|container| container.windows.iter().copied())
            .filter_map(|window| {
                window_manager
                    .smallest_size(window)
                    .filter(|(width, height)| *width > 0 || *height > 0)
                    .map(|size| (window, size))
            })
            .collect();
        workspace.apply_layout(work_area);
        Self::batch_position_workspace_windows(workspace, window_manager)
    }

    /// A window that floats has the floating look (a border and a glow of
    /// its own, where the configuration sets them apart), and loses it
    /// when it is tiled again. Every window of the workspace that has a
    /// border and is dressed otherwise than it floats is dressed anew.
    pub(super) fn dress_for_floating(
        workspace: &mut crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
    ) {
        let wanted: Vec<(WindowHandle, bool)> = workspace
            .all_windows()
            .into_iter()
            .map(|window| (window, workspace.floats(window)))
            .filter(|(window, floats)| {
                let look = workspace.border_look(*floats);
                workspace.dressed_as_floating.get(window) != Some(&look.floating)
            })
            .collect();
        for (window, floats) in wanted {
            if !queries(window_manager).has_border(window) {
                continue;
            }
            let focused = queries(window_manager).active_window() == Some(window);
            Self::dress_window(workspace, window_manager, window, floats, focused);
        }
    }

    /// The windows the Floating layout floated that have no border get
    /// the one floating windows have.
    ///
    /// A tiled window gets its border when it is put in its tile, and a
    /// window that comes to a workspace under the Floating layout is
    /// floated before it ever is: `dress_for_floating` only dresses anew
    /// what has a border, so such a window was left without one, while
    /// one a rule floats had it.
    fn dress_floated_by_layout(
        workspace: &mut crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
    ) {
        if !workspace.border_enabled || workspace.invisible_borders.enabled {
            return;
        }
        let bare: Vec<WindowHandle> = workspace
            .floating_windows
            .iter()
            .copied()
            .filter(|window| workspace.floated_by_layout.contains(window))
            .filter(|window| {
                let desktop = queries(window_manager);
                !desktop.has_border(*window)
                    && desktop.exists(*window)
                    && !desktop.is_minimized(*window)
                    && !desktop.is_decoration(*window)
            })
            .collect();
        for window in bare {
            let focused = queries(window_manager).active_window() == Some(window);
            if !Self::dress_window(workspace, window_manager, window, true, focused) {
                continue;
            }
            // (On a workspace that is not shown the window is hidden, and
            // its border with it.)
            if window_manager.is_hidden_by_backend(window) {
                try_effect(
                    window_manager,
                    Effect::Border {
                        id: window,
                        change: BorderChange::Visible(false),
                    },
                );
            }
        }
    }

    /// Draw a window's border (and glow) as its workspace says for a
    /// tiled or a floating window, and remember which it was. Returns
    /// whether the window has a border now.
    pub(crate) fn dress_window(
        workspace: &mut crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
        window: WindowHandle,
        floats: bool,
        focused: bool,
    ) -> bool {
        let look = workspace.border_look(floats);
        workspace.dressed_as_floating.insert(window, look.floating);
        if !look.enabled {
            try_effect(
                window_manager,
                Effect::Border {
                    id: window,
                    change: BorderChange::Remove,
                },
            );
            window_manager.set_window_glow(window, None);
            return false;
        }
        if let Err(error) = window_manager.decorate_window(window, &look, focused) {
            log::warn!("Could not draw the border of {window:?}: {error}");
            return false;
        }
        window_manager.set_window_glow(
            window,
            look.floating.then_some(crate::contract::GlowStrength {
                radius: workspace.floating_look.glow_radius,
                opacity: workspace.floating_look.glow_opacity,
                inactive_opacity: workspace.floating_look.glow_inactive_opacity,
            }),
        );
        true
    }

    /// How much of a floating window stays on the display when it is put
    /// out of the way: a tenth of its width.
    const ASIDE_SHOWN: i32 = 10;

    /// `ToggleFloatingAside`: the floating windows of the workspace the
    /// keyboard's display shows go out of the way, each to the side of the
    /// display it is nearer to, with a tenth of it left in sight; or, when
    /// they are out of the way, come back to where they were.
    pub(super) fn toggle_floating_aside(
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
        let workspace = monitor_manager
            .get_focused_workspace_mut(monitor)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;
        if !workspace.aside.is_empty() {
            return Self::bring_floating_back(workspace, window_manager);
        }
        let floating: Vec<WindowHandle> = workspace.floating_windows.iter().copied().collect();
        let mut moved = false;
        for window in floating {
            let Ok(rect) = queries(window_manager).rect(window) else {
                continue;
            };
            if queries(window_manager).is_minimized(window) {
                continue;
            }
            let shown = (rect.width / Self::ASIDE_SHOWN).max(24).min(rect.width);
            let middle = work_area.x + work_area.width / 2;
            let x = if rect.x + rect.width / 2 < middle {
                work_area.x - rect.width + shown
            } else {
                work_area.x + work_area.width - shown
            };
            workspace.aside.insert(window, rect);
            effect(
                window_manager,
                Effect::Configure {
                    id: window,
                    rect: gevrek_model::geometry::Rect { x, ..rect },
                    states: WindowStates::default(),
                },
            )?;
            try_effect(
                window_manager,
                Effect::Border {
                    id: window,
                    change: BorderChange::Follow,
                },
            );
            moved = true;
        }
        if !moved {
            return Ok(());
        }
        // The keyboard goes to the tiled window that was used last: that
        // is what the floating ones were put away for.
        let active = queries(window_manager).active_window();
        if active.is_some_and(|active| workspace.aside.contains_key(&active)) {
            if let Some(tiled) = workspace
                .containers
                .focused()
                .and_then(|container| container.focused_window())
            {
                effect(window_manager, Effect::Focus(Some(tiled)))?;
            }
        }
        log::debug!(
            "Floating windows of workspace '{}' put out of the way",
            workspace.name
        );
        Ok(())
    }

    /// The floating windows that were put out of the way are where they
    /// were again. Nothing to do when none is.
    pub(super) fn bring_floating_back(
        workspace: &mut crate::workspace::Workspace,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        for (window, rect) in std::mem::take(&mut workspace.aside) {
            if !queries(window_manager).exists(window) || !workspace.floats(window) {
                continue;
            }
            effect(
                window_manager,
                Effect::Configure {
                    id: window,
                    rect,
                    states: WindowStates::default(),
                },
            )?;
            try_effect(
                window_manager,
                Effect::Border {
                    id: window,
                    change: BorderChange::Follow,
                },
            );
        }
        Ok(())
    }

    /// What the Floating layout asks of a workspace's windows: under it
    /// the tiled ones float (where they are), and one that came since the
    /// last pass is put in the middle of the display at its own size, no
    /// larger than the display has room for. One that belongs to another
    /// window, or that stands where its program put it, stays where it
    /// is: its place means something (a dialog over its window, a palette
    /// beside it), and is only given up where too little of the window
    /// would be on the display to take hold of. Under another layout the
    /// windows it floated are tiled again (the caller lays them out).
    fn settle_floating_layout(
        workspace: &mut crate::workspace::Workspace,
        work_area: crate::container::Rect,
        window_manager: &mut dyn Desktop,
    ) -> Result<()> {
        Self::dress_for_floating(workspace, window_manager);
        let (floated, retiled) = workspace.sync_floating_layout();
        Self::dress_for_floating(workspace, window_manager);
        Self::dress_floated_by_layout(workspace, window_manager);
        for (windows, shown) in [(floated, true), (retiled, false)] {
            for window in windows {
                effect(
                    window_manager,
                    Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                        id: window,
                        shown,
                    })),
                )?;
            }
        }
        for window in workspace.take_arrivals() {
            let Ok(rect) = queries(window_manager).rect(window) else {
                continue;
            };
            let own_place = queries(window_manager).parent(window).is_some()
                || queries(window_manager).placed_itself(window);
            if own_place && reaches(&work_area, &rect) {
                try_effect(
                    window_manager,
                    Effect::Border {
                        id: window,
                        change: BorderChange::Follow,
                    },
                );
                continue;
            }
            // A window that comes with the size of a tile that filled
            // (nearly) the whole display would cover everything as a
            // floating window: it gets two thirds of the display instead.
            let fills =
                rect.width * 5 >= work_area.width * 4 && rect.height * 5 >= work_area.height * 4;
            let (width, height) = if fills {
                (work_area.width * 2 / 3, work_area.height * 2 / 3)
            } else {
                (
                    rect.width.min(work_area.width),
                    rect.height.min(work_area.height),
                )
            };
            let (width, height) = (width.max(1), height.max(1));
            effect(
                window_manager,
                Effect::Configure {
                    id: window,
                    rect: gevrek_model::geometry::Rect {
                        x: work_area.x + (work_area.width - width) / 2,
                        y: work_area.y + (work_area.height - height) / 2,
                        width,
                        height,
                    },
                    states: WindowStates::default(),
                },
            )?;
            try_effect(
                window_manager,
                Effect::Border {
                    id: window,
                    change: BorderChange::Follow,
                },
            );
        }
        Ok(())
    }

    /// Re-tile a specific window on a specific monitor
    pub(super) fn retile_window_on_monitor(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        monitor_handle: MonitorHandle,
    ) -> Result<()> {
        log::debug!("🔄 Re-tiling window on monitor {:?}", monitor_handle);

        // A window that already has its tile on this output's workspace
        // on screen (it was dragged and let go where it was, or resized)
        // stays in it: taken out and put back, it left its group of tabs,
        // the next tab stayed hidden, and the workspace forgot the shares
        // its windows had been given. The workspace is laid out as it is.
        let stays = monitor_manager
            .get_focused_workspace(monitor_handle)
            .is_some_and(|workspace| workspace.find_container(handle).is_some());
        if stays {
            if let Some(work_area) =
                monitor_manager.get_adjusted_work_area(monitor_handle, window_manager)
            {
                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    self.suppressed_windows.insert(handle);
                    let laid_out = self.apply_workspace_layout_and_position(
                        workspace,
                        work_area,
                        window_manager,
                    );
                    self.suppressed_windows.remove(&handle);
                    return laid_out;
                }
            }
            return Ok(());
        }

        // Add to suppression list to prevent event loops
        self.suppressed_windows.insert(handle);

        // CRITICAL: Check if window was floating AND its container index BEFORE removing it
        // We need to preserve both floating status and order when re-positioning
        let was_floating = monitor_manager.is_window_floating(handle);

        // Find the container index before removing (to preserve order)
        let old_container_idx = if !was_floating {
            // Get the focused workspace and find container index
            if let Some(workspaces) = monitor_manager.get_workspaces(monitor_handle) {
                if let Some(workspace) = workspaces.focused() {
                    workspace.find_container(handle)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Remove from any existing workspace (and retile the old workspace if needed)
        if let Some((old_monitor, old_workspace_idx)) =
            monitor_manager.find_and_remove_window(handle)
        {
            log::debug!("   Removed from old workspace on monitor {:?}", old_monitor);

            // CRITICAL: Only retile if we removed a TILED window
            // Floating windows don't affect tiling layout
            if !was_floating {
                // (Also shown again: the tab that takes the place of the
                // window that left its group.)
                self.mark_layout_dirty(old_monitor);
                // Retile the old workspace to fill the gap
                let old_work_area = if let Some(area) =
                    monitor_manager.get_adjusted_work_area(old_monitor, window_manager)
                {
                    area
                } else {
                    log::warn!("Could not get adjusted work area for retiling");
                    self.suppressed_windows.remove(&handle);
                    return Ok(());
                };
                if let Some(old_workspaces) = monitor_manager.get_workspaces_mut(old_monitor) {
                    if let Some(old_workspace) = old_workspaces.get_mut(old_workspace_idx) {
                        old_workspace.apply_layout(old_work_area);

                        // Reposition remaining windows atomically using batch positioning
                        Self::batch_position_workspace_windows(old_workspace, window_manager)?;
                        log::debug!("   Retiled old workspace (filled gap)");
                    }
                }
            } else {
                log::debug!("   Skipped retiling - window was floating, not tiled");
            }
        }

        // Get monitor work area (adjusted for overlays)
        let work_area = if let Some(area) =
            monitor_manager.get_adjusted_work_area(monitor_handle, window_manager)
        {
            area
        } else {
            self.suppressed_windows.remove(&handle);
            return Ok(());
        };

        // Add to focused workspace on target monitor
        // CRITICAL: Preserve floating status - floating windows don't get tiled
        if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
            log::debug!(
                "   Adding to workspace '{}' with layout {:?}",
                workspace.name,
                workspace.layout
            );

            if was_floating {
                // Window was floating - keep it floating (don't add to containers)
                workspace.add_floating_window(handle);
                log::debug!("   ✅ Window re-added as FLOATING (doesn't affect tiling)");
            } else {
                // Window was tiled - add to containers and retile
                // CRITICAL: Preserve order by inserting at old position if available
                if let Some(idx) = old_container_idx {
                    workspace.add_window_at(handle, idx);
                    log::debug!(
                        "   ✅ Window re-added at SAME POSITION {} (order preserved)",
                        idx
                    );
                } else {
                    workspace.add_window(handle, true); // true = ADD AS TILED
                    log::debug!("   Window added at END (new position)");
                }

                self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
                log::debug!("   Applied {:?} layout to workspace", workspace.layout);

                log::debug!("✅ Window successfully tiled in workspace!");
            }
        }

        self.suppressed_windows.remove(&handle);

        Ok(())
    }

    /// Resize adjacent containers based on which edge was resized
    pub(super) fn resize_adjacent_containers(
        &mut self,
        monitor_handle: MonitorHandle,
        resized: Resized<'_>,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let Resized {
            index: resized_idx,
            width_delta,
            height_delta,
            old: old_rect,
            new: new_rect,
        } = resized;
        use crate::container::Rect;

        // Get all containers in the workspace
        let containers_data: Vec<(Rect, Option<WindowHandle>)> =
            match monitor_manager.get_focused_workspace(monitor_handle) {
                Some(workspace) => workspace
                    .containers
                    .iter()
                    .map(|c| (c.rect, c.focused_window()))
                    .collect(),
                None => return Ok(()),
            };

        if containers_data.len() <= 1 {
            return Ok(());
        }

        // Determine which edge was resized by comparing positions
        let left_edge_resized = new_rect.x < old_rect.x;
        let right_edge_resized = new_rect.x + new_rect.width > old_rect.x + old_rect.width;
        let top_edge_resized = new_rect.y < old_rect.y;
        let bottom_edge_resized = new_rect.y + new_rect.height > old_rect.y + old_rect.height;

        // Find adjacent containers and resize them
        for (idx, (container_rect, window_opt)) in containers_data.iter().enumerate() {
            let window = match window_opt {
                Some(w) if idx != resized_idx => *w,
                _ => continue,
            };
            let mut adjusted_rect = *container_rect;
            let mut needs_adjustment = false;

            // Right edge resized - adjust containers to the right
            if right_edge_resized && width_delta != 0 {
                // Is this container to the right of the resized one?
                if container_rect.x >= old_rect.x + old_rect.width {
                    adjusted_rect.x += width_delta;
                    adjusted_rect.width -= width_delta;
                    needs_adjustment = true;
                    log::trace!(
                        "Adjusting right neighbor: x+={}, width-={}",
                        width_delta,
                        width_delta
                    );
                }
            }

            // Bottom edge resized - adjust containers below
            if bottom_edge_resized && height_delta != 0 {
                // Is this container below the resized one?
                if container_rect.y >= old_rect.y + old_rect.height {
                    adjusted_rect.y += height_delta;
                    adjusted_rect.height -= height_delta;
                    needs_adjustment = true;
                    log::trace!(
                        "Adjusting bottom neighbor: y+={}, height-={}",
                        height_delta,
                        height_delta
                    );
                }
            }

            // Left edge resized - adjust containers to the left
            if left_edge_resized && width_delta != 0 {
                // Is this container to the left of the resized one?
                if container_rect.x + container_rect.width <= old_rect.x {
                    adjusted_rect.width += width_delta;
                    needs_adjustment = true;
                    log::trace!("Adjusting left neighbor: width+={}", width_delta);
                }
            }

            // Top edge resized - adjust containers above
            if top_edge_resized && height_delta != 0 {
                // Is this container above the resized one?
                if container_rect.y + container_rect.height <= old_rect.y {
                    adjusted_rect.height += height_delta;
                    needs_adjustment = true;
                    log::trace!("Adjusting top neighbor: height+={}", height_delta);
                }
            }

            // Apply the adjustment
            if needs_adjustment && adjusted_rect.width > 50 && adjusted_rect.height > 50 {
                // Suppress this window to prevent event loops
                self.suppressed_windows.insert(window);

                effect(
                    window_manager,
                    Effect::Configure {
                        id: window,
                        rect: gevrek_model::geometry::Rect {
                            x: adjusted_rect.x,
                            y: adjusted_rect.y,
                            width: adjusted_rect.width,
                            height: adjusted_rect.height,
                        },
                        states: WindowStates::default(),
                    },
                )?;

                // Update the container rect in the workspace
                if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                    if let Some(container) = workspace.containers.elements_mut().get_mut(idx) {
                        container.rect = adjusted_rect;
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle resize for ultrawide vertical stack layout
    pub(super) fn resize_ultrawide_containers(
        &mut self,
        monitor_handle: MonitorHandle,
        resized_idx: usize,
        width_delta: i32,
        height_delta: i32,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // For ultrawide layout:
        // - Index 0 is the master window (left side)
        // - Other indices are stacked windows (right side)

        if resized_idx == 0 {
            // Master window resized - adjust all stack windows
            if width_delta != 0 {
                let containers_data: Vec<(crate::container::Rect, Option<WindowHandle>)> =
                    match monitor_manager.get_focused_workspace(monitor_handle) {
                        Some(workspace) => workspace
                            .containers
                            .iter()
                            .map(|c| (c.rect, c.focused_window()))
                            .collect(),
                        None => return Ok(()),
                    };

                for (idx, (container_rect, window_opt)) in containers_data.iter().enumerate() {
                    let window = match window_opt {
                        Some(w) if idx != 0 => *w,
                        _ => continue,
                    };
                    let mut adjusted_rect = *container_rect;
                    adjusted_rect.x += width_delta;
                    adjusted_rect.width -= width_delta;

                    if adjusted_rect.width > 50 {
                        self.suppressed_windows.insert(window);
                        effect(
                            window_manager,
                            Effect::Configure {
                                id: window,
                                rect: gevrek_model::geometry::Rect {
                                    x: adjusted_rect.x,
                                    y: adjusted_rect.y,
                                    width: adjusted_rect.width,
                                    height: adjusted_rect.height,
                                },
                                states: WindowStates::default(),
                            },
                        )?;

                        if let Some(workspace) =
                            monitor_manager.get_focused_workspace_mut(monitor_handle)
                        {
                            if let Some(container) =
                                workspace.containers.elements_mut().get_mut(idx)
                            {
                                container.rect = adjusted_rect;
                            }
                        }
                    }
                }
            }
        }
        // Stack window resized - adjust adjacent stack windows (vertical)
        else if height_delta != 0 {
            let adjacent_idx = resized_idx + 1;
            let adjacent = monitor_manager
                .get_focused_workspace(monitor_handle)
                .and_then(|workspace| workspace.containers.elements().get(adjacent_idx))
                .map(|container| (container.rect, container.focused_window()));

            if let Some((mut adjacent_rect, Some(window))) = adjacent {
                adjacent_rect.y += height_delta;
                adjacent_rect.height -= height_delta;
                if adjacent_rect.height > 50 {
                    self.suppressed_windows.insert(window);
                    effect(
                        window_manager,
                        Effect::Configure {
                            id: window,
                            rect: gevrek_model::geometry::Rect {
                                x: adjacent_rect.x,
                                y: adjacent_rect.y,
                                width: adjacent_rect.width,
                                height: adjacent_rect.height,
                            },
                            states: WindowStates::default(),
                        },
                    )?;
                    if let Some(workspace) =
                        monitor_manager.get_focused_workspace_mut(monitor_handle)
                    {
                        if let Some(container) =
                            workspace.containers.elements_mut().get_mut(adjacent_idx)
                        {
                            container.rect = adjacent_rect;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub(super) fn update_active_workspace_layout<F>(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut crate::workspace::Workspace) -> Result<()>,
    {
        // The workspace of the window with the keyboard, when that window
        // is on the display commands act on. Where that display shows a
        // workspace with no window (the user went to an empty display),
        // it is that workspace, and never the one of a window that still
        // has the keyboard on another display.
        let display = self.active_monitor(window_manager, monitor_manager);
        // (And when its workspace is the one that display shows: a window
        // that a rule sent to another workspace as it opened still has
        // the keyboard, and a layout command given then used to lay out
        // the workspace nobody sees and bring its window into sight.)
        let active = queries(window_manager).active_window().and_then(|window| {
            let (monitor, index) = monitor_manager.find_window(window)?;
            let shown = monitor_manager
                .get_workspaces(monitor)
                .map(|workspaces| workspaces.focused_idx());
            ((display.is_none() || display == Some(monitor)) && shown == Some(index))
                .then_some((window, monitor, index))
        });
        let (active, monitor, workspace_index) = match active {
            Some((window, monitor, index)) => (Some(window), monitor, index),
            None => {
                let monitor = display
                    .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
                let index = monitor_manager
                    .get_workspaces(monitor)
                    .map(|workspaces| workspaces.focused_idx())
                    .ok_or_else(|| anyhow::anyhow!("The active monitor has no workspace"))?;
                (None, monitor, index)
            }
        };
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(workspace_index))
            .ok_or_else(|| anyhow::anyhow!("The active workspace no longer exists"))?;
        update(workspace)?;
        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        self.sync_group_visibility(workspace, window_manager)?;
        if let Some(active) = active {
            effect(window_manager, Effect::Focus(Some(active)))?;
        }
        Ok(())
    }

    /// Apply a layout mutation to the focused workspace on the active
    /// monitor and re-layout.
    pub(super) fn update_focused_workspace_layout<F>(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut crate::workspace::Workspace) -> Result<()>,
    {
        let monitor = self
            .active_monitor(window_manager, monitor_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the active monitor"))?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let workspace = monitor_manager
            .get_focused_workspace_mut(monitor)
            .ok_or_else(|| anyhow::anyhow!("The active monitor has no focused workspace"))?;
        update(workspace)?;
        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        self.sync_group_visibility(workspace, window_manager)?;
        Ok(())
    }

    pub(super) fn update_window_workspace_layout<F>(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut crate::workspace::Workspace) -> Result<()>,
    {
        let (monitor, workspace_index) = monitor_manager
            .find_window(window)
            .ok_or_else(|| anyhow::anyhow!("The selected window is not managed"))?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(workspace_index))
            .ok_or_else(|| anyhow::anyhow!("The selected window's workspace no longer exists"))?;
        update(workspace)?;
        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)
    }

    pub(super) fn toggle_window_pseudotile(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        window: WindowHandle,
    ) -> Result<()> {
        let current = queries(window_manager)
            .rect(window)
            .map_err(anyhow::Error::msg)?;
        let (preferred_width, preferred_height) = window_manager
            .preferred_window_size(window)
            .unwrap_or((current.width, current.height));
        let preferred =
            crate::container::Rect::new(current.x, current.y, preferred_width, preferred_height);
        self.update_window_workspace_layout(
            window_manager,
            monitor_manager,
            window,
            move |workspace| {
                if workspace.find_container(window).is_none() {
                    anyhow::bail!("The selected window is floating");
                }
                let enabled = workspace.toggle_pseudotile(window, preferred);
                log::debug!(
                    "Pseudotile {} for {:?} at preferred size {}x{}",
                    if enabled { "enabled" } else { "disabled" },
                    window,
                    preferred.width,
                    preferred.height
                );
                Ok(())
            },
        )
    }

    pub(super) fn promote_to_master(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        self.update_active_workspace_layout(window_manager, monitor_manager, move |workspace| {
            let index = workspace
                .find_container(active)
                .ok_or_else(|| anyhow::anyhow!("The active window is floating"))?;
            if index != 0 {
                workspace.containers.elements_mut().swap(0, index);
            }
            workspace.containers.focus_at(0);
            workspace.split_ratios.clear();
            Ok(())
        })
    }

    /// Move the focused container to the front of the
    /// layout ring by removal and re-insertion, preserving the relative
    /// order of the other containers (unlike `promote_to_master`, which
    /// swaps positions).
    pub(super) fn promote(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        self.update_active_workspace_layout(window_manager, monitor_manager, move |workspace| {
            let index = workspace
                .find_container(active)
                .ok_or_else(|| anyhow::anyhow!("The active window is floating"))?;
            if index != 0 {
                let container = workspace
                    .containers
                    .remove_at(index)
                    .ok_or_else(|| anyhow::anyhow!("The active container no longer exists"))?;
                workspace.containers.push_front(container);
            }
            workspace.containers.focus_at(0);
            workspace.split_ratios.clear();
            Ok(())
        })
    }

    /// Move keyboard focus to the front (master)
    /// container without changing the container order.
    pub(super) fn promote_focus(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        let (monitor, workspace_index) = monitor_manager
            .find_window(active)
            .ok_or_else(|| anyhow::anyhow!("The active window is not managed"))?;
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(workspace_index))
            .ok_or_else(|| anyhow::anyhow!("The active workspace no longer exists"))?;
        if workspace.containers.is_empty() {
            anyhow::bail!("The workspace has no tiled windows");
        }
        workspace.containers.focus_at(0);
        let window = workspace
            .focused_window()
            .ok_or_else(|| anyhow::anyhow!("The front container is empty"))?;
        effect(window_manager, Effect::Focus(Some(window)))?;
        Ok(())
    }

    pub fn resize_delta(&self) -> i32 {
        self.resize_delta
    }

    /// Snapshot every workspace's resize dimensions
    /// (layout, master ratio and count, BSP split ratios) plus the session
    /// resize delta to a JSON file. Returns the number of workspaces written.
    pub fn save_resize_dimensions(
        &self,
        monitor_manager: &MonitorManager,
        path: &std::path::Path,
    ) -> Result<usize> {
        let mut state = crate::persistence::ResizeDimensionsState::new(self.resize_delta);
        for (monitor_index, monitor) in monitor_manager
            .get_all_monitors_ordered()
            .iter()
            .enumerate()
        {
            let Some(workspaces) = monitor_manager.get_workspaces(monitor.handle) else {
                continue;
            };
            for (workspace_index, workspace) in workspaces.elements().iter().enumerate() {
                state
                    .workspaces
                    .push(crate::persistence::WorkspaceResizeState {
                        monitor_index,
                        workspace_index,
                        name: workspace.name.clone(),
                        layout: workspace.layout,
                        master_ratio: workspace.master_ratio,
                        master_count: workspace.master_count,
                        bsp_split_ratios: workspace.bsp_split_ratios.clone(),
                    });
            }
        }
        state.save(path)?;
        log::info!(
            "Saved resize dimensions for {} workspace(s) to {}",
            state.workspaces.len(),
            path.display()
        );
        Ok(state.workspaces.len())
    }

    /// Restore a resize snapshot written by
    /// `save-resize`. Workspaces are matched by monitor and workspace index
    /// (the saved name is diagnostics only); entries without a live
    /// counterpart are skipped. Returns the number of workspaces restored.
    pub fn load_resize_dimensions(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        path: &std::path::Path,
    ) -> Result<usize> {
        let state = crate::persistence::ResizeDimensionsState::load(path)?;
        if !(1..=1000).contains(&state.resize_delta) {
            anyhow::bail!(
                "Saved resize delta {} is outside the valid 1-1000 pixel range",
                state.resize_delta
            );
        }
        self.resize_delta = state.resize_delta;

        let monitors = monitor_manager.get_all_monitors_ordered();
        let mut applied = 0usize;
        for entry in &state.workspaces {
            let Some(monitor) = monitors.get(entry.monitor_index) else {
                log::warn!(
                    "Skipping resize snapshot for missing monitor {}",
                    entry.monitor_index
                );
                continue;
            };
            let Some(workspace) = monitor_manager
                .get_workspaces_mut(monitor.handle)
                .and_then(|workspaces| workspaces.get_mut(entry.workspace_index))
            else {
                log::warn!(
                    "Skipping resize snapshot for missing workspace {} on monitor {}",
                    entry.workspace_index,
                    entry.monitor_index
                );
                continue;
            };
            if !workspace.name.eq_ignore_ascii_case(&entry.name) {
                log::debug!(
                    "Restoring resize dimensions saved from '{}' onto workspace '{}' (monitor {}, index {})",
                    entry.name,
                    workspace.name,
                    entry.monitor_index,
                    entry.workspace_index
                );
            }
            workspace.layout = entry.layout;
            workspace.master_ratio = entry.master_ratio.clamp(0.1, 0.9);
            workspace.master_count = entry.master_count.clamp(1, 8);
            workspace.bsp_split_ratios = entry
                .bsp_split_ratios
                .iter()
                .map(|ratio| ratio.clamp(0.05, 0.95))
                .collect();
            applied += 1;
        }

        // Re-layout so the restored ratios take effect on visible workspaces.
        for monitor in &monitors {
            self.mark_layout_dirty(monitor.handle);
        }
        self.flush_pending_layouts(window_manager, monitor_manager)?;
        log::info!(
            "Loaded resize dimensions for {} workspace(s) from {}",
            applied,
            path.display()
        );
        Ok(applied)
    }

    /// Shared plumbing for tiled resize commands: resolve the active
    /// window's workspace and container, honor the layout resize lock, apply
    /// the ratio change, and re-layout. The `resize` closure returns `false`
    /// when the layout has no adjustable split for the request.
    pub(super) fn resize_active_container<F>(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        resize: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut crate::workspace::Workspace, usize, crate::container::Rect) -> bool,
    {
        let active = queries(window_manager)
            .active_window()
            .ok_or_else(|| anyhow::anyhow!("There is no active window"))?;
        let (monitor, workspace_index) = monitor_manager
            .find_window(active)
            .ok_or_else(|| anyhow::anyhow!("The active window is not managed"))?;
        let work_area = monitor_manager
            .get_adjusted_work_area(monitor, window_manager)
            .ok_or_else(|| anyhow::anyhow!("Could not determine the monitor work area"))?;
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(workspace_index))
            .ok_or_else(|| anyhow::anyhow!("The active workspace no longer exists"))?;
        if workspace.layout_locked {
            anyhow::bail!("The workspace layout is locked against resizing");
        }
        let index = workspace
            .find_container(active)
            .ok_or_else(|| anyhow::anyhow!("The active window is floating"))?;
        if !resize(workspace, index, work_area) {
            anyhow::bail!("The layout has no adjustable split for this resize");
        }
        self.apply_workspace_layout_and_position(workspace, work_area, window_manager)?;
        effect(window_manager, Effect::Focus(Some(active)))?;
        Ok(())
    }

    /// Grow or shrink one edge of the focused tiled
    /// window by the configured resize delta.
    pub(super) fn resize_edge(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        edge: Direction,
        sizing: Sizing,
    ) -> Result<()> {
        let direction = match edge {
            Direction::Left => EdgeDirection::Left,
            Direction::Right => EdgeDirection::Right,
            Direction::Up => EdgeDirection::Up,
            Direction::Down => EdgeDirection::Down,
        };
        let delta = sizing.apply(self.resize_delta);
        self.resize_active_container(
            window_manager,
            monitor_manager,
            move |workspace, index, work_area| {
                workspace.resize_container_edge(index, direction, delta, work_area)
            },
        )
    }

    /// Grow or shrink the focused tiled window along
    /// an axis by the configured resize delta.
    pub(super) fn resize_axis(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        axis: LayoutAxis,
        sizing: Sizing,
    ) -> Result<()> {
        let delta = sizing.apply(self.resize_delta);
        self.resize_active_container(
            window_manager,
            monitor_manager,
            move |workspace, index, work_area| {
                workspace.resize_container_axis(index, axis, delta, work_area)
            },
        )
    }

    /// Force re-application of the current layout to the focused workspace on
    /// every monitor, even while tiling is suspended.
    /// Workspaces with tiling switched off are still left alone by
    /// `Workspace::apply_layout`.
    pub(super) fn retile_focused_workspaces(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let monitors: Vec<MonitorHandle> = monitor_manager
            .get_all_monitors_ordered()
            .iter()
            .map(|monitor| monitor.handle)
            .collect();
        if monitors.is_empty() {
            anyhow::bail!("No attached monitors are available");
        }
        let mut retiled = 0;
        for monitor in &monitors {
            let Some(work_area) = monitor_manager.get_adjusted_work_area(*monitor, window_manager)
            else {
                continue;
            };
            let Some(workspace) = monitor_manager.get_focused_workspace_mut(*monitor) else {
                continue;
            };
            Self::layout_and_position_workspace(workspace, work_area, window_manager)?;
            self.sync_group_visibility(workspace, window_manager)?;
            retiled += 1;
        }
        log::debug!(
            "Retiled the focused workspace on {} of {} monitor(s)",
            retiled,
            monitors.len()
        );
        Ok(())
    }

    /// Toggle the global tiling suspension.
    /// While suspended the layout engine never repositions windows, though
    /// windows stay managed and dispatch/graph commands keep working (unlike
    /// pause, which freezes the automation graph). Resuming re-tiles the
    /// focused workspace on every monitor.
    pub(super) fn toggle_tiling(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        self.tiling_suspended = !self.tiling_suspended;
        log::info!(
            "Tiling {} globally",
            if self.tiling_suspended {
                "suspended"
            } else {
                "resumed"
            }
        );
        if !self.tiling_suspended {
            self.retile_focused_workspaces(window_manager, monitor_manager)?;
        }
        Ok(())
    }

    /// Toggle tiling for one workspace on the active monitor. A workspace with
    /// tiling off keeps every window exactly where it is; the layout engine
    /// leaves it alone until tiling is switched back on, which re-applies
    /// the layout.
    pub(super) fn toggle_workspace_tiling(
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
        let work_area = monitor_manager.get_adjusted_work_area(monitor, window_manager);
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(index))
            .ok_or_else(|| anyhow::anyhow!("The selected workspace no longer exists"))?;
        workspace.tiling_enabled = !workspace.tiling_enabled;
        let enabled = workspace.tiling_enabled;
        log::debug!(
            "Tiling {} for workspace '{}' using selector '{}'",
            if enabled { "enabled" } else { "disabled" },
            workspace.name,
            selector.expression()
        );
        if enabled {
            if let Some(work_area) = work_area {
                Self::layout_and_position_workspace(workspace, work_area, window_manager)?;
            }
        }
        Ok(())
    }

    /// Toggle the float override for one workspace on the active monitor
    ///. While enabled, every
    /// window on the workspace — existing and newly added — floats, ignoring
    /// tiling; disabling re-tiles exactly the windows the override floated.
    pub(super) fn toggle_workspace_float_override(
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
        let work_area = monitor_manager.get_adjusted_work_area(monitor, window_manager);
        let workspace = monitor_manager
            .get_workspaces_mut(monitor)
            .and_then(|workspaces| workspaces.get_mut(index))
            .ok_or_else(|| anyhow::anyhow!("The selected workspace no longer exists"))?;
        let enabled = !workspace.float_override;
        let (floated, retiled) = workspace.set_float_override(enabled);
        for window in floated {
            effect(
                window_manager,
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                    id: window,
                    shown: true,
                })),
            )?;
        }
        let retiled_count = retiled.len();
        for window in retiled {
            effect(
                window_manager,
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                    id: window,
                    shown: false,
                })),
            )?;
        }
        log::debug!(
            "Float override {} for workspace '{}' using selector '{}'",
            if enabled { "enabled" } else { "disabled" },
            workspace.name,
            selector.expression()
        );
        if retiled_count > 0 {
            if let Some(work_area) = work_area {
                Self::layout_and_position_workspace(workspace, work_area, window_manager)?;
            }
        }
        Ok(())
    }
}

/// A container that was made larger or smaller by hand: which one, by
/// how much, and its place before and after.
pub(super) struct Resized<'a> {
    pub index: usize,
    pub width_delta: i32,
    pub height_delta: i32,
    pub old: &'a crate::container::Rect,
    pub new: &'a crate::container::Rect,
}
