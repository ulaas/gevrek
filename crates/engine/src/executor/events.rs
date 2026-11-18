//! Reacting to window events from the backend: created, destroyed, focused, moved,
//! minimized, drags, and changes of the monitor set.

use super::*;

impl Executor {
    /// Window-state guards shared by the `Moved` and `Resized` handlers
    /// (a maximize or restore can surface as either event). Reverts a
    /// maximize of a tiled window, and retiles when a minimized window has
    /// come back without a MINIMIZEEND event. Returns true when the event was
    /// fully handled. Callers skip it while the workspace is in expose mode.
    pub(super) fn handle_window_state_change(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<bool> {
        // TILING WM: Prevent maximize for tiled windows
        // Check if window became maximized and revert it immediately
        if queries(window_manager).is_maximized(handle) {
            // (Only a window that has a tile: one in no workspace at all,
            // a scratchpad or one gevrek leaves alone, is as free to be
            // maximised as a floating one.)
            let is_floating = monitor_manager.is_window_floating(handle)
                || monitor_manager.find_window(handle).is_none();
            if !is_floating {
                log::info!("🗖 Tiled window maximize detected - unmaximizing immediately");
                effect(
                    window_manager,
                    Effect::State {
                        id: handle,
                        change: StateChange::Maximized(false),
                    },
                )?;

                // Retile the workspace to restore proper positioning
                if let Some((monitor_handle, _)) = monitor_manager.find_window(handle) {
                    if let Some(work_area) =
                        monitor_manager.get_adjusted_work_area(monitor_handle, window_manager)
                    {
                        if let Some(workspace) =
                            monitor_manager.get_focused_workspace_mut(monitor_handle)
                        {
                            self.apply_workspace_layout_and_position(
                                workspace,
                                work_area,
                                window_manager,
                            )?;
                            log::info!("✅ Workspace retiled - maximize prevented");
                        }
                    }
                }
                return Ok(true);
            } else {
                log::debug!("Floating window {:?} maximized - allowing", handle);
            }
        }

        // A restore whose EVENT_SYSTEM_MINIMIZEEND never arrived.
        if self.window_state.get(handle).minimized && !queries(window_manager).is_minimized(handle)
        {
            log::info!("🔼 Window restored (detected via LOCATIONCHANGE)");
            self.on_window_restored(window_manager, monitor_manager, handle)?;
            // Don't process the normal move event for a restored window
            return Ok(true);
        }

        Ok(false)
    }

    /// A window came back from minimized. A tiled window that left the layout
    /// returns to its workspace if that one is visible, otherwise it joins
    /// the visible workspace of the monitor it reappears on (never a hidden
    /// workspace, which would hide it again). A window that was never managed
    /// (minimized when the WM started) is adopted now.
    pub(super) fn on_window_restored(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        // A scratchpad's window is on no workspace and stays on none.
        // (One that was minimized when the scratchpad took it was put
        // back on the workspace it had left, and was the scratchpad's
        // still.)
        if self.scratchpads.is_scratchpad_window(handle) {
            return Ok(());
        }
        let fallback = monitor_of(window_manager, monitor_manager, handle)
            .map(|monitor| monitor.handle)
            .or_else(|| self.active_monitor(window_manager, monitor_manager));
        let restore = lifecycle::restored(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            handle,
            fallback,
        );
        if restore == Restore::NeedsAdoption {
            self.adopt_window_by_default(window_manager, monitor_manager, handle)?;
        }

        let mut changes = Changes::default();
        let settled = lifecycle::restored_settle(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            handle,
            &mut changes,
        );
        if let Some((_, tiled)) = settled {
            if tiled && queries(window_manager).is_maximized(handle) {
                log::info!("🗖 Tiled window was maximized during restore - unmaximizing");
                effect(
                    window_manager,
                    Effect::State {
                        id: handle,
                        change: StateChange::Maximized(false),
                    },
                )?;
            }
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Visible(true),
                },
            );
        }
        self.apply_changes(window_manager, changes);
        self.reevaluate_dynamic_rules(window_manager, monitor_manager, handle)
    }

    /// Periodic cleanup of stale tracking data
    /// Call this every few seconds to prevent memory leaks from windows that were
    /// destroyed without us receiving the Destroyed event
    ///
    /// Liveness is `IsWindow`, never the visible-only window snapshot: windows
    /// on hidden workspaces, hidden group tabs, and hidden scratchpads are
    /// alive and must keep their tracking state.
    pub fn cleanup_stale_tracking(&mut self, window_manager: &dyn Desktop) {
        self.cleanup_stale_tracking_in(window_manager);
    }

    /// As `cleanup_stale_tracking`, and a window that is gone also leaves
    /// its workspace (its end was never told: without this its tile stays
    /// empty until the configuration is next read), which is laid out
    /// again.
    pub fn cleanup_stale_members(
        &mut self,
        window_manager: &dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
    ) {
        let gone: Vec<WindowHandle> = crate::reload::managed_windows(monitor_manager)
            .into_iter()
            .filter(|window| !queries(window_manager).exists(*window))
            .collect();
        for window in gone {
            if let Some((output, _)) = monitor_manager.find_and_remove_window(window) {
                log::info!("{window:?} is gone without its end being told: it leaves its workspace");
                self.mark_layout_dirty(output);
            }
            zone_manager.remove_window(window);
            self.rule_metadata.forget(window);
        }
        self.cleanup_stale_tracking_in(window_manager);
    }

    fn cleanup_stale_tracking_in(&mut self, window_manager: &dyn Desktop) {
        // Liveness of every tracked window, not only adopted ones: a dead
        // handle's state must not carry over to a new window that reuses it.
        let stale_known: Vec<WindowHandle> = self
            .window_state
            .ids_where(|_| true)
            .into_iter()
            .filter(|h| !queries(window_manager).exists(*h))
            .collect();
        for handle in &stale_known {
            self.suppressed_windows.remove(handle);
            self.window_state.forget(*handle);
        }
        self.scratchpads
            .retain_existing(|handle| queries(window_manager).exists(handle));

        if !stale_known.is_empty() {
            log::debug!(
                "Cleaned up tracking data for {} stale windows",
                stale_known.len()
            );
        }
    }

    /// Apply a display reconciliation: carry monitor-keyed state over to new
    /// handles, show/hide windows that moved between visible and hidden
    /// workspaces, and lay every monitor out again.
    pub fn apply_monitor_changes(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        changes: &crate::outputs::MonitorChanges,
    ) {
        let live = |monitor: MonitorHandle| monitor_manager.get_monitor(monitor).is_some();
        if let Some(focused) = self.focused_monitor {
            self.focused_monitor = changes
                .remap(focused)
                .or(Some(focused))
                .filter(|m| live(*m));
        }
        self.pending_layout_monitors = self
            .pending_layout_monitors
            .iter()
            .map(|monitor| changes.remap(*monitor).unwrap_or(*monitor))
            .filter(|monitor| live(*monitor))
            .collect();
        self.locked_groups = self
            .locked_groups
            .iter()
            .map(|(monitor, workspace, container)| {
                (
                    changes.remap(*monitor).unwrap_or(*monitor),
                    *workspace,
                    *container,
                )
            })
            .filter(|(monitor, _, _)| live(*monitor))
            .collect();
        for drag in self
            .window_state
            .iter_mut()
            .filter_map(|(_, state)| state.drag.as_mut())
        {
            if let Some(original) = drag.original_monitor {
                drag.original_monitor = changes
                    .remap(original)
                    .or(Some(original))
                    .filter(|m| live(*m));
            }
        }

        for handle in &changes.windows_to_show {
            if queries(window_manager).exists(*handle) {
                let _ = effect(
                    window_manager,
                    Effect::SetVisible {
                        id: *handle,
                        visible: true,
                    },
                );
            }
            self.window_state.get_mut(*handle).hidden_by_wm = false;
        }
        for handle in &changes.windows_to_hide {
            if queries(window_manager).exists(*handle) {
                let _ = self.hide_workspace_window(window_manager, *handle);
            }
        }
        for monitor in monitor_manager.get_all_monitors() {
            self.mark_layout_dirty(monitor.handle);
        }
    }

    /// Process pending drags - detects when windows stop moving and re-tiles them
    /// Call this periodically from the runtime event loop (fallback for missed DragEnded events)
    pub fn process_pending_drags(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
    ) -> Result<()> {
        const DRAG_END_TIMEOUT: Duration = Duration::from_millis(500);
        let now = Instant::now();
        let mut completed_drags = Vec::new();

        // A drag with no recent movement is only "completed" once the mouse
        // button has been released. While the button is still held, a
        // stationary window is a PAUSED drag and the real DragEnded event is
        // still coming; finalizing now would drop the drag state (making that
        // DragEnded a no-op) and could zone-snap a window the user is still
        // holding.
        let primary_button_down = window_manager.primary_button_down();

        // Find drags that have timed out (window stopped moving)
        for (handle, drag_state) in self
            .window_state
            .iter()
            .filter_map(|(handle, state)| state.drag.as_ref().map(|drag| (handle, drag)))
        {
            if now.duration_since(drag_state.last_move_time) > DRAG_END_TIMEOUT
                && !primary_button_down
            {
                completed_drags.push(handle);
            }
        }

        // Process completed drags
        for handle in completed_drags {
            if let Some(_drag_state) = self.window_state.get_mut(handle).drag.take() {
                log::info!("Drag timeout - processing as completed drag");

                // EARLY EXIT: Floating and unmanaged windows stay where dropped
                let is_floating = monitor_manager.is_window_floating(handle);
                let is_unmanaged = monitor_manager.find_window(handle).is_none();
                if is_floating || is_unmanaged {
                    log::info!(
                        "📍 Floating/unmanaged window - keeping at current position (drag timeout)"
                    );
                    let _ = effect(
                        window_manager,
                        Effect::Border {
                            id: handle,
                            change: BorderChange::Follow,
                        },
                    );
                    continue;
                }

                // --- Tiled window only: zone snapping and retiling ---
                let current_monitor = monitor_of(window_manager, monitor_manager, handle);

                if let Some(current_monitor_info) = current_monitor {
                    let current_monitor_handle = current_monitor_info.handle;

                    let mut snapped_to_zone = false;
                    if zone_manager.is_enabled() {
                        if let Ok(window_rect) = queries(window_manager)
                            .rect(handle)
                            .map_err(anyhow::Error::msg)
                        {
                            if let Some(zone_hit) =
                                zone_manager.find_zone_for_rect(&window_rect, monitor_manager)
                            {
                                log::info!("📍 Tiled window dropped on zone '{}' (via timeout) - snapping!", zone_hit.zone.name);
                                effect(
                                    window_manager,
                                    Effect::Configure {
                                        id: handle,
                                        rect: gevrek_model::geometry::Rect {
                                            x: zone_hit.rect.x,
                                            y: zone_hit.rect.y,
                                            width: zone_hit.rect.width,
                                            height: zone_hit.rect.height,
                                        },
                                        states: WindowStates::default(),
                                    },
                                )?;
                                zone_manager.assign_window_to_zone(
                                    handle,
                                    zone_hit.monitor,
                                    zone_hit.zone.id,
                                );

                                // Keep workspace membership: a zone-snapped
                                // window becomes floating (same semantics as
                                // SnapActiveToZone). Removing it from the
                                // workspace outright would orphan it —
                                // find_window() would return None and the
                                // window would be treated as unmanaged.
                                self.set_window_floating(
                                    window_manager,
                                    monitor_manager,
                                    handle,
                                    true,
                                )?;
                                snapped_to_zone = true;
                            }
                        }
                    }

                    if !snapped_to_zone {
                        self.retile_window_on_monitor(
                            window_manager,
                            monitor_manager,
                            handle,
                            current_monitor_handle,
                        )?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle live resize - adjust adjacent windows while user resizes
    pub(super) fn handle_live_resize(
        &mut self,
        handle: WindowHandle,
        old_rect: &crate::container::Rect,
        new_rect: &crate::container::Rect,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        // Find which monitor and workspace this window belongs to
        let monitor_handle = match monitor_of(window_manager, monitor_manager, handle) {
            Some(m) => m.handle,
            None => return Ok(()),
        };

        // Check if workspace exists
        if monitor_manager
            .get_focused_workspace(monitor_handle)
            .is_none()
        {
            return Ok(());
        }

        // Find the container index for this window
        let container_idx = match monitor_manager
            .get_focused_workspace(monitor_handle)
            .and_then(|ws| ws.find_container(handle))
        {
            Some(idx) => idx,
            None => return Ok(()),
        };

        // Calculate the resize delta
        let width_delta = new_rect.width - old_rect.width;
        let height_delta = new_rect.height - old_rect.height;

        log::info!(
            "Live resize: width_delta={}, height_delta={}",
            width_delta,
            height_delta
        );

        // Get workspace layout type
        let (layout_type, container_count) =
            match monitor_manager.get_focused_workspace(monitor_handle) {
                Some(workspace) => (workspace.layout, workspace.container_count()),
                None => return Ok(()),
            };

        use crate::layouts::LayoutType;
        // On the strip a column's width is its own: the column takes the
        // width its window was given, and no other column gives or takes.
        if layout_type == LayoutType::Scrolling {
            if width_delta != 0 {
                let work_area =
                    monitor_manager.get_adjusted_work_area(monitor_handle, window_manager);
                let resized = work_area.is_some_and(|work_area| {
                    monitor_manager
                        .get_focused_workspace_mut(monitor_handle)
                        .is_some_and(|workspace| {
                            workspace.set_column_width(container_idx, new_rect.width, work_area)
                        })
                });
                if resized {
                    self.mark_layout_dirty(monitor_handle);
                }
            }
            return Ok(());
        }

        // Only adjust adjacent windows if we have multiple containers
        if container_count <= 1 {
            return Ok(());
        }

        // Find and resize adjacent containers based on layout type
        match layout_type {
            LayoutType::BSP
            | LayoutType::Columns
            | LayoutType::Rows
            | LayoutType::Grid
            | LayoutType::Spiral
            | LayoutType::Dwindle => {
                // For these layouts, we need to find adjacent containers and resize them
                self.resize_adjacent_containers(
                    monitor_handle,
                    super::layout::Resized {
                        index: container_idx,
                        width_delta,
                        height_delta,
                        old: old_rect,
                        new: new_rect,
                    },
                    window_manager,
                    monitor_manager,
                )?;
            }
            LayoutType::Monocle | LayoutType::Floating | LayoutType::Scrolling => {
                // Monocle doesn't have adjacent windows; a floating
                // workspace has no tiles at all. (The scrolling layout
                // was seen to above.)
            }
            LayoutType::UltrawideVerticalStack
            | LayoutType::MasterStack
            | LayoutType::CenteredMaster => {
                // Handle master-based layouts resize specially
                self.resize_ultrawide_containers(
                    monitor_handle,
                    container_idx,
                    width_delta,
                    height_delta,
                    window_manager,
                    monitor_manager,
                )?;
            }
        }

        Ok(())
    }

    pub fn execute_window_event(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        focus_manager: &mut FocusManager,
        event: WindowEvent,
    ) -> Result<()> {
        self.release_placed_scratchpads(monitor_manager);
        match event {
            WindowEvent::Created(handle) => {
                self.on_window_created(window_manager, monitor_manager, focus_manager, handle)
            }
            WindowEvent::Destroyed(handle) => {
                let result =
                    self.on_window_destroyed(window_manager, monitor_manager, zone_manager, handle);
                // What the graph was told of the window is not kept: a
                // key pressed next would hand a window that has gone to
                // the nodes behind it.
                self.pin_values.retain(|_, value| match value {
                    Value::WindowHandle(window) => *window != handle,
                    Value::WindowList(windows) => {
                        windows.retain(|window| *window != handle);
                        true
                    }
                    _ => true,
                });
                result
            }
            WindowEvent::Focused(handle) => {
                self.on_window_focused(window_manager, monitor_manager, focus_manager, handle)
            }
            WindowEvent::Moved(handle, x, y) => {
                self.on_window_moved(window_manager, monitor_manager, handle, x, y)
            }
            WindowEvent::Resized(handle, width, height) => {
                self.on_window_resized(window_manager, monitor_manager, handle, width, height)
            }
            WindowEvent::DragStarted(handle) => {
                self.on_drag_started(window_manager, monitor_manager, handle)
            }
            WindowEvent::DragEnded(handle) => {
                let result =
                    self.on_drag_ended(window_manager, monitor_manager, zone_manager, handle);
                // The window with the keyboard was dropped on another
                // output: the keyboard is on that output now, and what is
                // asked for next (a layout, a workspace) is asked of it.
                if queries(window_manager).active_window() == Some(handle) {
                    if let Some((output, _)) = monitor_manager.find_window(handle) {
                        self.focused_monitor = Some(output);
                    }
                }
                result
            }
            WindowEvent::Minimized(handle) => {
                self.on_window_minimized(window_manager, monitor_manager, handle)
            }
            WindowEvent::Restored(handle) => {
                self.on_window_restored_event(window_manager, monitor_manager, handle)
            }
            WindowEvent::Unmapped(handle) => {
                self.on_window_unmapped(window_manager, monitor_manager, handle)
            }
            WindowEvent::MetadataChanged(handle) => {
                self.on_window_metadata_changed(window_manager, monitor_manager, handle)
            }
        }
    }

    /// A window appeared (or became visible again).
    fn on_window_created(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        focus_manager: &mut FocusManager,
        handle: WindowHandle,
    ) -> Result<()> {
        // A window that is gone by the time its opening is told (it was
        // shown for a moment, or the telling was held back) is no window
        // to take: every question about it would be answered with nothing,
        // and nothing reads as a window like any other.
        if !queries(window_manager).exists(handle) {
            log::debug!("{handle:?} is gone before it was told of as opened");
            return Ok(());
        }

        // CRITICAL: Skip events for windows that should be ignored (like Gevrek's own windows)
        if queries(window_manager).ignored_by_rules(handle) {
            log::debug!("Skipping OnWindowCreated event for excluded window");
            return Ok(());
        }

        // CRITICAL: Skip duplicate Created events for windows we've already processed
        // EVENT_OBJECT_SHOW can fire multiple times for the same window
        if self.window_state.get(handle).known {
            log::debug!(
                "Skipping duplicate Created event for already known window {:?}",
                handle
            );
            return Ok(());
        }

        // Track this window as known
        self.window_state.get_mut(handle).known = true;

        // Find all OnWindowCreated nodes and set their output values
        let mut event_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnWindowCreated))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        event_nodes.sort_unstable();

        for node_id in event_nodes {
            // Set window handle output (output index 1)
            self.set_output_value(node_id, 1, Value::WindowHandle(handle));
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }

        self.apply_initial_window_rules(window_manager, monitor_manager, handle)?;
        self.arm_initial_focus_suppression(window_manager, monitor_manager, focus_manager, handle)?;
        self.adopt_pending_scratchpad(window_manager, monitor_manager, handle)?;
        if !self.scratchpads.is_scratchpad_window(handle)
            && self.adopt_new_window(window_manager, monitor_manager, handle)?
        {
            // The rules were applied above, before the window was in a
            // workspace: what they say of a workspace (move it to
            // another) had nothing to act on then. Now it has.
            self.apply_initial_window_rules(window_manager, monitor_manager, handle)?;
        }
        // Started by `flush_pending_layouts`, once the window is tiled.
        self.pending_window_enters.push(handle);
        // A window can have the keyboard before it is told of as opened
        // (Windows tells of the two in either order). What follows the
        // keyboard was then carried out for a window that was not in a
        // workspace and had no border yet: the workspace still held the
        // window before it as its focused one, and no border was drawn as
        // focused. Now that it has its place, it is carried out again.
        // (Not for a window whose rules keep the keyboard from it: that
        // was answered above.)
        if queries(window_manager).active_window() == Some(handle)
            && monitor_manager.find_window(handle).is_some()
            && !self.no_initial_focus_enabled(handle)
            && !self.no_focus_enabled(handle)
        {
            self.on_window_focused(window_manager, monitor_manager, focus_manager, handle)?;
        }
        Ok(())
    }

    /// A window is gone.
    fn on_window_destroyed(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        handle: WindowHandle,
    ) -> Result<()> {
        log::debug!("🗑️  Handling Destroyed event for window {:?}", handle);

        if self.window_state.get(handle).drag.is_some() {
            log::debug!("Cleaned up drag state for destroyed window {:?}", handle);
        }
        self.suppressed_windows.remove(&handle);
        self.scratchpads.forget_window(handle);
        zone_manager.remove_window(handle);

        // Remove border if this window has one
        let _ = effect(
            window_manager,
            Effect::Border {
                id: handle,
                change: BorderChange::Remove,
            },
        );
        window_manager.untrack_window(handle);
        self.rule_metadata.forget(handle);

        // Expose mode: handle window destruction while expose is active
        if let Some((monitor_handle, ws_idx)) = monitor_manager.find_window(handle) {
            // Get work area before mutable borrow of workspace
            let work_area = monitor_manager.get_adjusted_work_area(monitor_handle, window_manager);

            if let Some(workspace) = monitor_manager
                .get_workspaces_mut(monitor_handle)
                .and_then(|ws| ws.get_mut(ws_idx))
            {
                let ws_id = workspace.id;
                if self.expose_workspaces.contains(&ws_id) {
                    match expose::window_destroyed(workspace, handle, work_area) {
                        AfterDestroy::NotActive => {}
                        AfterDestroy::Ended => {
                            self.expose_workspaces.remove(&ws_id);
                            // (The borders are back with the overview's
                            // end, as when it is left by its key.)
                            let _ = effect(window_manager, Effect::DecorationsVisible(true));
                            log::info!("Expose mode auto-deactivated: no windows remaining");
                        }
                        AfterDestroy::Regrid(ops) => {
                            if !ops.is_empty() {
                                let _ =
                                    Self::batch_move_resize_and_sync_borders(window_manager, &ops);
                                self.last_expose_transition = Instant::now();
                            }
                            if work_area.is_some() {
                                log::info!("Expose grid recalculated after window destruction");
                            }
                        }
                    }
                }
            }
        }

        // Drop all tracking state and the workspace membership.
        let mut changes = Changes::default();
        let removal = lifecycle::destroyed(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            handle,
            &mut changes,
        );
        // The visible workspace is laid out by the once-per-wake
        // flush (which also skips expose).
        self.apply_changes(window_manager, changes);
        let removed_from = match removal {
            Removal::NotManaged => None,
            Removal::Visible(output) => Some(output),
            Removal::Hidden { output, .. } => Some(output),
        };
        if let Some(monitor_handle) = removed_from {
            log::info!(
                "✅ Window {:?} removed from workspace, reapplying layout",
                handle
            );
            let work_area = monitor_manager.get_adjusted_work_area(monitor_handle, window_manager);
            // The flush only lays out the focused workspace, so a
            // hidden one is laid out here. (An output without a work area,
            // one that has just gone, is laid out by nobody; the graph is
            // told of the window's end all the same.)
            if let (Removal::Hidden { workspace, .. }, Some(work_area)) = (removal, work_area) {
                if let Some(workspace) = monitor_manager
                    .get_workspaces_mut(monitor_handle)
                    .and_then(|workspaces| workspaces.get_mut(workspace))
                {
                    if !self.expose_workspaces.contains(&workspace.id) {
                        self.apply_workspace_layout_and_position(
                            workspace,
                            work_area,
                            window_manager,
                        )?;
                        log::debug!(
                            "Layout reapplied and windows repositioned after window removal"
                        );
                    }
                }
            }
        }

        let mut event_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnWindowDestroyed))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        event_nodes.sort_unstable();

        for node_id in event_nodes {
            self.set_output_value(node_id, 1, Value::WindowHandle(handle));
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }
        Ok(())
    }

    /// A window got the keyboard focus.
    fn on_window_focused(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        focus_manager: &mut FocusManager,
        handle: WindowHandle,
    ) -> Result<()> {
        let decision = focus::on_focused(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            focus_manager,
            handle,
            |candidate| {
                queries(window_manager).exists(candidate)
                    && queries(window_manager).is_visible(candidate)
            },
        );
        match decision {
            FocusDecision::Enforce(target) => {
                self.focus_managed_window(window_manager, monitor_manager, target)?;
                return Ok(());
            }
            FocusDecision::Reject { restore } => {
                let restored = self.restore_previous_focus(
                    window_manager,
                    monitor_manager,
                    focus_manager,
                    handle,
                    restore,
                )?;
                if !restored {
                    let _ = effect(window_manager, Effect::Focus(None));
                }
                focus_manager.remove_window(handle);
                return Ok(());
            }
            FocusDecision::SuppressInitial { restore } => {
                let _ = self.restore_previous_focus(
                    window_manager,
                    monitor_manager,
                    focus_manager,
                    handle,
                    restore,
                )?;
                return Ok(());
            }
            // The workspace focus already follows the window.
            FocusDecision::Accept { output } => {
                // A window that has the keyboard before it was told of as
                // opened (Windows tells of the two in either order) stands
                // where the system put it, not where it was asked for: the
                // keyboard's output stays, and the window is brought to it
                // when it is adopted (`adopt_new_window`).
                let not_yet_opened = output.is_none()
                    && window_manager.new_windows_go_where_the_keyboard_is()
                    && !self.window_state.get(handle).known
                    && !queries(window_manager).ignored_by_rules(handle);
                if not_yet_opened {
                    log::debug!("{handle:?} has the keyboard before it opened; the output stays");
                }
                let output = output.or_else(|| {
                    if not_yet_opened {
                        return None;
                    }
                    monitor_of(window_manager, monitor_manager, handle)
                        .map(|monitor| monitor.handle)
                });
                if let Some(output) = output {
                    self.focused_monitor = Some(output);
                    // Under the scrolling layout the view follows the
                    // keyboard.
                    let scrolls =
                        monitor_manager
                            .get_focused_workspace(output)
                            .is_some_and(|workspace| {
                                workspace.layout == crate::layouts::LayoutType::Scrolling
                                    && workspace.find_container(handle).is_some()
                            });
                    if scrolls {
                        self.mark_layout_dirty(output);
                    }
                }
            }
        }
        for scratchpad_handle in self.scratchpads.hide_on_focus_loss(handle) {
            if queries(window_manager).exists(scratchpad_handle) {
                let _ = effect(
                    window_manager,
                    Effect::SetVisible {
                        id: scratchpad_handle,
                        visible: false,
                    },
                );
            }
        }

        // Update border focus states - O(1) instead of O(n)
        // Skip during expose mode (borders are hidden)
        let focus_in_expose = if let Some((mh, _)) = monitor_manager.find_window(handle) {
            monitor_manager
                .get_focused_workspace(mh)
                .is_some_and(|ws| self.expose_workspaces.contains(&ws.id))
        } else {
            false
        };
        if !focus_in_expose {
            if let Some(prev) = self.last_focused_window {
                if prev != handle {
                    let _ = effect(
                        window_manager,
                        Effect::Border {
                            id: prev,
                            change: BorderChange::Focused(false),
                        },
                    );
                }
            }
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Focused(true),
                },
            );
        }
        self.last_focused_window = Some(handle);

        // In the overview of a workspace's windows, a window that takes
        // the keyboard is the one that was chosen (a press on its
        // picture): the overview is over, every window is where it was,
        // and this one is on top, with the keyboard. (As where the
        // desktop shows the overview itself, `toggle_window_overview`.
        // It used to be brought to the middle of the overview, which
        // then had to be left by its key.)
        if let Some((monitor_handle, _)) = monitor_manager.find_window(handle) {
            if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor_handle) {
                let chosen = self.expose_workspaces.contains(&workspace.id)
                    && workspace
                        .expose_state
                        .as_ref()
                        .is_some_and(|overview| overview.grid_rects.contains_key(&handle));
                if chosen {
                    let workspace_id = workspace.id;
                    let back = expose::deactivate(workspace);
                    if !back.is_empty() {
                        let _ = Self::batch_move_resize_and_sync_borders(window_manager, &back);
                        // (What the windows say of their way back is not
                        // a move of the person's.)
                        self.last_expose_transition = Instant::now();
                    }
                    self.expose_workspaces.remove(&workspace_id);
                    let _ = effect(window_manager, Effect::DecorationsVisible(true));
                    let _ = effect(
                        window_manager,
                        Effect::Stack {
                            id: handle,
                            op: StackOp::Raise,
                        },
                    );
                    log::info!("The overview of workspace {workspace_id} is over: a window was chosen");
                }
            }
        }

        let mut event_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnWindowFocused))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        event_nodes.sort_unstable();

        for node_id in event_nodes {
            self.set_output_value(node_id, 1, Value::WindowHandle(handle));
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }
        Ok(())
    }

    /// A window moved.
    fn on_window_moved(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        x: i32,
        y: i32,
    ) -> Result<()> {
        // Skip tiling-related move handling when expose mode is active —
        // expose manages window positions directly and retiling would destroy the grid
        let in_expose = self.is_in_expose(monitor_manager, handle);

        // Expose mode: skip normal tiling and handle expose-specific logic
        if in_expose {
            // During the 300ms after a promote/demote transition, suppress ALL
            // recenter/snap logic to prevent the feedback loop caused by
            // GetWindowRect returning the full rect (with invisible frame) while
            // batch_move_resize targets the visible rect
            if self.last_expose_transition.elapsed() > Duration::from_millis(300) {
                if let Some((mh, _)) = monitor_manager.find_window(handle) {
                    let work_area = monitor_manager.get_adjusted_work_area(mh, window_manager);
                    if let Some(workspace) = monitor_manager.get_focused_workspace_mut(mh) {
                        let correction = expose::correct_move(workspace, handle, work_area, || {
                            queries(window_manager)
                                .rect(handle)
                                .map_err(anyhow::Error::msg)
                                .ok()
                        });
                        if let Some(op) = correction {
                            let _ = Self::batch_move_resize_and_sync_borders(window_manager, &[op]);
                            self.last_expose_transition = Instant::now();
                        }
                    }
                }
            }
            return Ok(());
        }

        if self.handle_window_state_change(window_manager, monitor_manager, handle)? {
            return Ok(());
        }

        // Runtime animation frames generate normal Windows location
        // events. Keep them out of user-authored move automations and
        // drag/retile handling until the transition settles.
        if window_manager.is_animation_event_suppressed(handle) {
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Follow,
                },
            );
            return Ok(());
        }

        // Check if this is a suppressed window (we caused this move)
        if self.suppressed_windows.remove(&handle) {
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Follow,
                },
            );
            log::debug!("Ignoring move event for suppressed window (self-triggered)");
            return Ok(());
        }

        // Update border position if this window has one
        let _ = effect(
            window_manager,
            Effect::Border {
                id: handle,
                change: BorderChange::Follow,
            },
        );

        // Note: Drag tracking is now handled via WM_ENTERSIZEMOVE/WM_EXITSIZEMOVE events
        // We no longer track drags based on move events (was too aggressive)

        // Execute any OnWindowMoved event nodes (if configured)
        let mut event_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnWindowMoved))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        event_nodes.sort_unstable();

        for node_id in event_nodes {
            self.set_output_value(node_id, 1, Value::WindowHandle(handle));
            self.set_output_value(node_id, 2, Value::Number(x as f64));
            self.set_output_value(node_id, 3, Value::Number(y as f64));
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }
        Ok(())
    }

    /// A window changed size.
    fn on_window_resized(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        width: i32,
        height: i32,
    ) -> Result<()> {
        if !self.is_in_expose(monitor_manager, handle)
            && self.handle_window_state_change(window_manager, monitor_manager, handle)?
        {
            return Ok(());
        }
        let (width, height) =
            self.enforce_size_constraints(window_manager, monitor_manager, handle, width, height)?;
        if window_manager.is_animation_event_suppressed(handle) {
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Follow,
                },
            );
            return Ok(());
        }

        // Check if this is a suppressed window (we caused this resize)
        if self.suppressed_windows.remove(&handle) {
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Follow,
                },
            );
            log::debug!("Ignoring resize event for suppressed window (self-triggered)");
            return Ok(());
        }

        self.remember_persistent_window_size(
            window_manager,
            monitor_manager,
            handle,
            width,
            height,
        );

        // Check if this window is being dragged - if so, it's a user resize operation
        // Extract the data we need before calling handle_live_resize to avoid double mutable borrow
        let (should_handle_resize, last_rect_clone) =
            if let Some(drag_state) = self.window_state.get(handle).drag.as_ref() {
                if let Some(last_rect) = &drag_state.last_known_rect {
                    (true, Some(*last_rect))
                } else {
                    (false, None)
                }
            } else {
                (false, None)
            };

        if should_handle_resize {
            if let Ok(current_rect) = queries(window_manager)
                .rect(handle)
                .map_err(anyhow::Error::msg)
            {
                if let Some(last_rect) = last_rect_clone {
                    if current_rect.width != last_rect.width
                        || current_rect.height != last_rect.height
                    {
                        // This is a resize operation!
                        let is_first_resize =
                            if let Some(drag_state) = self.window_state.get(handle).drag.as_ref() {
                                !drag_state.is_resizing
                            } else {
                                false
                            };

                        if is_first_resize {
                            log::info!("🔧 User is RESIZING window (size changed during drag)");
                        }

                        // Handle live resize - adjust adjacent windows
                        self.handle_live_resize(
                            handle,
                            &last_rect,
                            &current_rect,
                            window_manager,
                            monitor_manager,
                        )?;
                    }
                }

                // Update drag state after handle_live_resize completes
                if let Some(drag_state) = self.window_state.get_mut(handle).drag.as_mut() {
                    if current_rect.width
                        != drag_state
                            .last_known_rect
                            .as_ref()
                            .map(|r| r.width)
                            .unwrap_or(0)
                        || current_rect.height
                            != drag_state
                                .last_known_rect
                                .as_ref()
                                .map(|r| r.height)
                                .unwrap_or(0)
                    {
                        drag_state.is_resizing = true;
                    }
                    drag_state.last_known_rect = Some(current_rect);
                    drag_state.last_move_time = Instant::now();
                }
            }
        }

        // Update border position/size if this window has one
        let _ = effect(
            window_manager,
            Effect::Border {
                id: handle,
                change: BorderChange::Follow,
            },
        );

        // Execute any OnWindowResized event nodes (if configured)
        let mut event_nodes: Vec<NodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.node_type, NodeType::OnWindowResized))
            .map(|(id, _)| *id)
            .collect();
        // (In the order of the nodes: the same on every start.)
        event_nodes.sort_unstable();

        for node_id in event_nodes {
            self.set_output_value(node_id, 1, Value::WindowHandle(handle));
            self.set_output_value(node_id, 2, Value::Number(width as f64));
            self.set_output_value(node_id, 3, Value::Number(height as f64));
            let mut visited = HashSet::new();
            self.execute_node(window_manager, monitor_manager, node_id, &mut visited)?;
        }
        Ok(())
    }

    /// The user started dragging or resizing a window.
    fn on_drag_started(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        // User started dragging this window
        log::info!("User started dragging window");

        // Track original monitor for cross-monitor detection
        let original_monitor =
            monitor_of(window_manager, monitor_manager, handle).map(|m| m.handle);

        // Capture initial window rect for resize detection
        let original_rect = queries(window_manager)
            .rect(handle)
            .map_err(anyhow::Error::msg)
            .ok();

        // Start tracking this drag
        self.window_state.get_mut(handle).drag = Some(DragState {
            window: handle,
            last_move_time: Instant::now(),
            original_monitor,
            original_rect,
            is_resizing: false,
            last_known_rect: original_rect,
        });
        Ok(())
    }

    /// The user let go of a window.
    /// The display a window was dropped on. Where it was moved, that is
    /// the display the pointer is on: there is where the person put it.
    /// (Most of a large window still lies on the display it came from
    /// when it is let go just beyond the edge; asked where most of it
    /// was, it had not gone anywhere, and it took a few tries to get a
    /// window to the display beside.) Where it was only made another
    /// size, it is the display most of it is on.
    fn display_dropped_on(
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        was: Option<gevrek_model::geometry::Rect>,
    ) -> Option<crate::outputs::MonitorInfo> {
        let now = queries(window_manager)
            .rect(handle)
            .map_err(anyhow::Error::msg)
            .ok();
        let sized = was.zip(now).is_some_and(|(was, now)| {
            (was.width - now.width).abs() > 10 || (was.height - now.height).abs() > 10
        });
        if !sized {
            if let Some(under_pointer) = monitor_at_cursor(window_manager, monitor_manager) {
                return Some(under_pointer);
            }
        }
        monitor_of(window_manager, monitor_manager, handle)
    }

    fn on_drag_ended(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        zone_manager: &mut ZoneManager,
        handle: WindowHandle,
    ) -> Result<()> {
        log::info!("🎯 DRAG ENDED - User released window");

        // In the overview the windows lie where the overview put them:
        // a drag there ends without the workspace being laid out as tiles.
        if self.is_in_expose(monitor_manager, handle) {
            self.window_state.get_mut(handle).drag = None;
            return Ok(());
        }

        if let Some(drag_state) = self.window_state.get_mut(handle).drag.take() {
            // EARLY EXIT: Floating and unmanaged windows just stay where the user dropped them.
            // No retiling, no zone snapping, no resize ratio updates.
            let is_floating = monitor_manager.is_window_floating(handle);
            let is_unmanaged = monitor_manager.find_window(handle).is_none();
            if is_floating || is_unmanaged {
                log::info!(
                    "📍 Window is {} - keeping at drop position",
                    if is_floating { "floating" } else { "unmanaged" }
                );
                // Dropped on another output, a floating window belongs to
                // the workspace that output shows: it is listed there, hidden
                // and shown with it, and no longer with the one it left. It
                // stays where it was dropped.
                if is_floating {
                    let registered = monitor_manager
                        .find_window(handle)
                        .map(|(output, _)| output);
                    let dropped_on = Self::display_dropped_on(
                        window_manager,
                        monitor_manager,
                        handle,
                        drag_state.original_rect,
                    )
                    .map(|monitor| monitor.handle);
                    if let (Some(from), Some(to)) = (registered, dropped_on) {
                        if from != to && monitor_manager.get_focused_workspace(to).is_some() {
                            monitor_manager.find_and_remove_window(handle);
                            if let Some(workspace) = monitor_manager.get_focused_workspace_mut(to) {
                                workspace.add_floating_window(handle);
                                log::info!(
                                    "Floating window dropped on another output: now on workspace '{}'",
                                    workspace.name
                                );
                            }
                            if queries(window_manager).active_window() == Some(handle) {
                                self.focused_monitor = Some(to);
                            }
                        }
                    }
                }
                // Just update border position
                let _ = effect(
                    window_manager,
                    Effect::Border {
                        id: handle,
                        change: BorderChange::Follow,
                    },
                );
                // Skip all tiling/zone logic
            } else {
                // --- Tiled window drag logic ---
                let current_monitor = Self::display_dropped_on(
                    window_manager,
                    monitor_manager,
                    handle,
                    drag_state.original_rect,
                );

                if let Some(current_monitor_info) = current_monitor {
                    let current_monitor_handle = current_monitor_info.handle;

                    let monitor_changed = drag_state
                        .original_monitor
                        .map(|orig| orig != current_monitor_handle)
                        .unwrap_or(false);

                    let current_rect = queries(window_manager)
                        .rect(handle)
                        .map_err(anyhow::Error::msg)
                        .ok();
                    let was_resized = drag_state
                        .original_rect
                        .as_ref()
                        .zip(current_rect.as_ref())
                        .map(|(orig, curr)| {
                            (orig.width - curr.width).abs() > 10
                                || (orig.height - curr.height).abs() > 10
                        })
                        .unwrap_or(false);

                    if was_resized && !monitor_changed {
                        log::info!("📐 Tiled window RESIZED - updating split ratios");
                        if let Some(workspace) =
                            monitor_manager.get_focused_workspace_mut(current_monitor_handle)
                        {
                            if workspace.layout_locked {
                                log::info!(
                                    "Layout of workspace '{}' is locked - keeping ratios",
                                    workspace.name
                                );
                            } else if let Some(container_idx) = workspace.find_container(handle) {
                                if let Some(current_rect) = current_rect {
                                    let work_area_width = current_monitor_info.work_width;
                                    let work_area_height = current_monitor_info.work_height;

                                    if workspace.layout == crate::layouts::LayoutType::Columns {
                                        workspace.update_split_ratios_from_resize(
                                            container_idx,
                                            current_rect.width,
                                            work_area_width,
                                        );
                                    } else if workspace.layout == crate::layouts::LayoutType::Rows {
                                        workspace.update_split_ratios_from_resize(
                                            container_idx,
                                            current_rect.height,
                                            work_area_height,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // Zone snapping (only for tiled windows)
                    let snapped_to_zone = if zone_manager.is_enabled() && !was_resized {
                        if let Some(window_rect) = current_rect.as_ref() {
                            if let Some(zone_hit) =
                                zone_manager.find_zone_for_rect(window_rect, monitor_manager)
                            {
                                log::info!(
                                    "📍 Tiled window dropped on zone '{}' - snapping!",
                                    zone_hit.zone.name
                                );
                                effect(
                                    window_manager,
                                    Effect::Configure {
                                        id: handle,
                                        rect: gevrek_model::geometry::Rect {
                                            x: zone_hit.rect.x,
                                            y: zone_hit.rect.y,
                                            width: zone_hit.rect.width,
                                            height: zone_hit.rect.height,
                                        },
                                        states: WindowStates::default(),
                                    },
                                )?;
                                zone_manager.assign_window_to_zone(
                                    handle,
                                    zone_hit.monitor,
                                    zone_hit.zone.id,
                                );

                                // Keep workspace membership: a
                                // zone-snapped window becomes floating
                                // (same semantics as SnapActiveToZone).
                                // Removing it from the workspace
                                // outright would orphan it —
                                // find_window() would return None and
                                // the window would be treated as
                                // unmanaged.
                                self.set_window_floating(
                                    window_manager,
                                    monitor_manager,
                                    handle,
                                    true,
                                )?;
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !snapped_to_zone {
                        self.retile_window_on_monitor(
                            window_manager,
                            monitor_manager,
                            handle,
                            current_monitor_handle,
                        )?;
                    }
                } else {
                    log::warn!("Could not determine monitor for dropped tiled window");
                }
            } // end tiled window branch
        } else {
            log::warn!("Drag ended but no drag state found - was this drag tracked?");
        }
        self.reevaluate_dynamic_rules(window_manager, monitor_manager, handle)?;
        Ok(())
    }

    /// A window was minimized.
    fn on_window_minimized(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        let _ = effect(
            window_manager,
            Effect::Border {
                id: handle,
                change: BorderChange::Visible(false),
            },
        );

        let mut changes = Changes::default();
        let minimize = lifecycle::minimized(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            handle,
            &mut changes,
        );
        match minimize {
            Minimize::HiddenByWm => {
                log::debug!(
                    "Window {:?} was hidden by Gevrek (workspace switch) - allowing minimize",
                    handle
                );
                return Ok(());
            }
            Minimize::Kept => {
                log::debug!("Floating or unmanaged window {:?} minimized", handle)
            }
            // A minimized tiled window leaves the layout until it is
            // restored.
            Minimize::LeftLayout(_) => {
                let _ = effect(window_manager, Effect::HideStackbar(handle));
                log::info!(
                    "🔽 Tiled window {:?} minimized; removed from the layout",
                    handle
                );
            }
        }
        self.apply_changes(window_manager, changes);
        self.reevaluate_dynamic_rules(window_manager, monitor_manager, handle)?;
        Ok(())
    }

    /// A window came back from being minimized.
    fn on_window_restored_event(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        log::info!("🔼 Window restored (via event)");
        self.on_window_restored(window_manager, monitor_manager, handle)?;
        Ok(())
    }

    /// The application hid a window itself.
    fn on_window_unmapped(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        if !queries(window_manager).hidden_by_app(handle) {
            return Ok(());
        }
        // The app hid its own window (e.g. minimize to tray). It stops
        // being managed and leaves the layout; when the app shows it
        // again, the show event adopts it like a new window.
        let mut changes = Changes::default();
        let left = lifecycle::unmapped(
            &mut World {
                outputs: monitor_manager,
                windows: &mut self.window_state,
            },
            handle,
            &mut changes,
        );
        if left.is_some() {
            log::info!(
                "Window {:?} was hidden by its app; removing it from the layout",
                handle
            );
            let _ = effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Visible(false),
                },
            );
            let _ = effect(window_manager, Effect::HideStackbar(handle));
        }
        self.apply_changes(window_manager, changes);
        Ok(())
    }

    /// A window's title or other facts changed.
    fn on_window_metadata_changed(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        log::debug!("Window metadata changed for {:?}", handle);
        self.reevaluate_dynamic_rules(window_manager, monitor_manager, handle)?;
        // A title change must refresh the stackbar tab text when the
        // window lives in a group.
        self.refresh_stackbar_titles(window_manager, monitor_manager, handle);
        Ok(())
    }
}
