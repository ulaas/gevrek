//! Nodes that act on one window.

use super::*;

impl Executor {
    /// Run one node of this family (`execute_node_inner` picks the family).
    pub(super) fn run_window_node(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        node_id: NodeId,
        node: &gevrek_model::graph::Node,
        visited: &mut HashSet<NodeId>,
    ) -> Result<NodeDone> {
        match &node.node_type {
            // Action nodes
            NodeType::LaunchProcess {
                program,
                args,
                working_directory,
                create_no_window,
            } => {
                let program = self
                    .get_input_value(node_id, 1)
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| program.clone());
                let launch = LaunchProfile {
                    program,
                    args: args.clone(),
                    working_directory: (!working_directory.trim().is_empty())
                        .then(|| working_directory.clone()),
                    create_no_window: *create_no_window,
                    ..LaunchProfile::default()
                };
                let pid = window_manager.launch(&launch)?;
                log::info!(
                    "Launched process '{}' as PID {} from automation graph",
                    launch.program,
                    pid
                );

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::MoveWindow { x, y } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let x = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*x);

                    let y = self
                        .get_input_value(node_id, 3)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*y);

                    self.apply_window_command_to(
                        window_manager,
                        monitor_manager,
                        handle,
                        &DispatchCommand::MoveWindow {
                            selector: WindowSelector::Handle(handle.0 as i64),
                            x,
                            y,
                            relative: false,
                        },
                    )?;
                    log::info!("Moved window to ({}, {})", x, y);

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::ResizeWindow { width, height } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let width = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*width);

                    let height = self
                        .get_input_value(node_id, 3)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as i32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*height);

                    self.apply_window_command_to(
                        window_manager,
                        monitor_manager,
                        handle,
                        &DispatchCommand::ResizeWindow {
                            selector: WindowSelector::Handle(handle.0 as i64),
                            width,
                            height,
                            relative: false,
                        },
                    )?;
                    log::info!("Resized window to {}x{}", width, height);

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::CloseWindow => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    self.apply_window_command_to(
                        window_manager,
                        monitor_manager,
                        handle,
                        &DispatchCommand::CloseWindow {
                            selector: WindowSelector::Handle(handle.0 as i64),
                        },
                    )?;
                    log::info!("Closed window");

                    // Pass through window for chaining (even though window is closed)
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetWindowTitle { title } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let title = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| title.clone());

                    window_manager.apply(&Effect::Platform(PlatformEffect::Windows(
                        WindowsEffect::Title {
                            id: handle,
                            title: title.clone(),
                        },
                    )))?;
                    log::info!("Set window title to: {}", title);

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::AnimateWindow { animation_type } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let duration = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Number(n) = v {
                                Some(n as u32)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(300);

                    // Convert the automation-facing effect to the shared,
                    // non-blocking runtime scheduler.
                    use crate::animations::AnimationType as DwmAnimationType;

                    let dwm_anim_type = match animation_type {
                        AnimationType::Fade => DwmAnimationType::Fade,
                        AnimationType::Slide => DwmAnimationType::Slide,
                        AnimationType::Scale => DwmAnimationType::Scale,
                        AnimationType::Expand => DwmAnimationType::Expand,
                        AnimationType::Contract => DwmAnimationType::Contract,
                    };

                    if window_manager.start_window_animation(handle, dwm_anim_type, duration)? {
                        log::info!(
                            "Scheduled {:?} animation ({}ms)",
                            animation_type,
                            duration.clamp(16, 5000),
                        );
                    } else {
                        log::debug!(
                            "Skipped {:?} animation because runtime animations are disabled",
                            animation_type,
                        );
                    }

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetWindowArrangement { mode } => {
                // (A window that has gone is arranged nowhere: a graph
                // may hand on the window that closed, and it was given a
                // place for good.)
                if let Some(Value::WindowHandle(handle)) = self
                    .get_input_value(node_id, 1)
                    .filter(|value| match value {
                        Value::WindowHandle(handle) => queries(window_manager).exists(*handle),
                        _ => true,
                    })
                {
                    if self.is_startup_phase
                        && matches!(mode, gevrek_model::graph::WindowMode::Auto)
                    {
                        log::info!(
                            "Skipping startup auto-arrangement for existing window {:?}; startup is non-destructive",
                            handle
                        );
                        self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?;
                        return Ok(NodeDone::Early);
                    }

                    // CRITICAL: Never manage border windows or other Gevrek internal windows
                    if queries(window_manager).is_decoration(handle) {
                        log::warn!(
                            "⚠️  REJECTED border window from tiling system: {:?}",
                            handle
                        );
                        self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?;
                        return Ok(NodeDone::Early);
                    }

                    // CRITICAL: Never manage Gevrek's own windows
                    if queries(window_manager).ignored_by_rules(handle) {
                        log::info!("Skipping window management for excluded window");
                        self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?;
                        return Ok(NodeDone::Early);
                    }

                    // A window that is opening and belongs to a window on
                    // a workspace that is not shown opens there, whatever
                    // arrangement is asked for here: not on the workspace
                    // the person is looking at.
                    if self.join_parent_out_of_sight(window_manager, monitor_manager, handle)? {
                        self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                        self.trigger_output_execution(
                            window_manager,
                            monitor_manager,
                            node_id,
                            0,
                            visited,
                        )?;
                        return Ok(NodeDone::Early);
                    }

                    // Get window title for better logging
                    let window_title = queries(window_manager)
                        .windows()
                        .iter()
                        .find(|w| w.handle == handle)
                        .map(|w| w.title.clone())
                        .unwrap_or_else(|| format!("{:?}", handle));

                    log::info!(
                        "📋 Setting window arrangement: '{}' ({:?}) mode={:?}",
                        window_title,
                        handle,
                        mode
                    );

                    // Add to suppression list before any window operations
                    self.suppressed_windows.insert(handle);

                    // CRITICAL FIX: Detect which monitor to use based on context.
                    // New windows use their actual window position first; cursor fallback
                    // can misclassify launcher/tool windows and tile them on the wrong monitor.
                    let monitor_handle = if self.is_startup_phase {
                        // During startup, ALWAYS use window's current monitor
                        if let Some(window_monitor) =
                            monitor_of(window_manager, monitor_manager, handle)
                        {
                            log::info!(
                                "   └─ STARTUP: Monitor='{}' for window '{}'",
                                window_monitor.name,
                                window_title
                            );
                            window_monitor.handle
                        } else {
                            log::warn!("Could not detect window monitor, using primary monitor");
                            if let Some(primary) = monitor_manager.get_primary_monitor_handle() {
                                primary
                            } else {
                                log::error!("No monitors available!");
                                self.suppressed_windows.remove(&handle);
                                self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                                self.trigger_output_execution(
                                    window_manager,
                                    monitor_manager,
                                    node_id,
                                    0,
                                    visited,
                                )?;
                                return Ok(NodeDone::Early);
                            }
                        }
                    } else if let Some(keyboard_monitor) = self
                        .keyboard_output_for_new_tile(window_manager, monitor_manager, handle, mode)
                    {
                        // Where the system opens a window on a display of
                        // its own choosing (Windows: where the program
                        // last was), one that is to be tiled goes to the
                        // display with the keyboard: where it was asked
                        // for.
                        log::info!(
                            "   └─ NEW WINDOW: tiled on the display with the keyboard ({:?})",
                            keyboard_monitor
                        );
                        keyboard_monitor
                    } else {
                        if let Some(window_monitor) =
                            monitor_of(window_manager, monitor_manager, handle)
                        {
                            log::debug!(
                                "NEW WINDOW: Using monitor from window position: {}",
                                window_monitor.name
                            );
                            window_monitor.handle
                        } else if let Some(cursor_monitor) =
                            monitor_at_cursor(window_manager, monitor_manager)
                        {
                            log::debug!(
                                "NEW WINDOW: Using monitor at cursor position: {}",
                                cursor_monitor.name
                            );
                            cursor_monitor.handle
                        } else {
                            // Fall back to primary monitor if both detection methods fail
                            log::warn!("Could not detect monitor, using primary monitor");
                            if let Some(primary) = monitor_manager.get_primary_monitor_handle() {
                                primary
                            } else {
                                log::error!("No monitors available!");
                                self.suppressed_windows.remove(&handle);
                                self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                                self.trigger_output_execution(
                                    window_manager,
                                    monitor_manager,
                                    node_id,
                                    0,
                                    visited,
                                )?;
                                return Ok(NodeDone::Early);
                            }
                        }
                    };

                    // CRITICAL: Check if window was floating before removing it
                    // We need to preserve floating status to prevent floating windows from being tiled on move
                    let was_floating = monitor_manager.is_window_floating(handle);

                    // CRITICAL: Remove window from any existing workspace first
                    // This handles cross-monitor moves and prevents duplicates
                    let old_location = monitor_manager.find_and_remove_window(handle);

                    if let Some((old_monitor, old_workspace_idx)) = old_location {
                        // Window was moved from another workspace
                        if old_monitor != monitor_handle {
                            log::info!(
                                "Window moved from monitor {:?} to monitor {:?}",
                                old_monitor,
                                monitor_handle
                            );
                        }

                        // CRITICAL: Only retile if we removed a TILED window
                        // Floating windows don't affect tiling layout, so no need to retile
                        if !was_floating {
                            // Retile the old workspace to fill the gap
                            if let Some(old_work_area) =
                                monitor_manager.get_adjusted_work_area(old_monitor, window_manager)
                            {
                                if let Some(old_workspaces) =
                                    monitor_manager.get_workspaces_mut(old_monitor)
                                {
                                    if let Some(old_workspace) =
                                        old_workspaces.get_mut(old_workspace_idx)
                                    {
                                        self.apply_workspace_layout_and_position(
                                            old_workspace,
                                            old_work_area,
                                            window_manager,
                                        )?;
                                        log::info!(
                                            "Retiled old workspace after removing tiled window"
                                        );
                                    }
                                }
                            } else {
                                log::warn!("Could not get monitor info for old workspace retiling");
                            }
                        } else {
                            log::debug!(
                                "Skipped retiling - removed window was floating, not tiled"
                            );
                        }
                    }

                    // Get monitor info for work area
                    let work_area =
                        if let Some(monitor_info) = monitor_manager.get_monitor(monitor_handle) {
                            monitor_manager
                                .get_adjusted_work_area(monitor_handle, window_manager)
                                .unwrap_or_else(|| monitor_info.work_area_rect())
                        } else {
                            log::warn!("Failed to get monitor info for layout application");
                            // Pass through without positioning
                            self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                            self.trigger_output_execution(
                                window_manager,
                                monitor_manager,
                                node_id,
                                0,
                                visited,
                            )?;
                            return Ok(NodeDone::Early);
                        };

                    // Get focused workspace on this monitor and add window
                    if let Some(workspace) =
                        monitor_manager.get_focused_workspace_mut(monitor_handle)
                    {
                        // Determine actual mode to use
                        // Priority: 1) ALWAYS preserve floating status if window was floating (it's part of workspace but not tiled)
                        //          2) Use Auto mode detection if mode is Auto
                        //          3) Use explicit mode (Tiled/Floating) for new windows
                        let actual_mode = if was_floating {
                            // ALWAYS preserve floating status - floating windows are part of workspace but don't affect tiling
                            log::info!(
                                "Preserving FLOATING status for window that was already floating"
                            );
                            gevrek_model::graph::WindowMode::Floating
                        } else if matches!(mode, gevrek_model::graph::WindowMode::Auto) {
                            // Auto mode: detect based on window properties
                            let all_windows = queries(window_manager).windows();
                            if let Some(window) = all_windows.iter().find(|w| w.handle == handle) {
                                // Check if window should be ignored
                                if queries(window_manager).ignored_by_rules(handle) {
                                    log::info!(
                                        "Auto-detected as FLOATING (ignored window): '{}'",
                                        window.title
                                    );
                                    gevrek_model::graph::WindowMode::Floating
                                } else {
                                    let should_float = queries(window_manager)
                                        .floats_by_rules(handle)
                                        || window.title.is_empty();

                                    if should_float {
                                        log::info!(
                                            "Auto-detected as FLOATING (matched rule): '{}'",
                                            window.title
                                        );
                                        gevrek_model::graph::WindowMode::Floating
                                    } else {
                                        // On every monitor: a window no rule floats is tiled.
                                        log::info!("Auto-detected as TILED: '{}'", window.title);
                                        gevrek_model::graph::WindowMode::Tiled
                                    }
                                }
                            } else {
                                // Window not found, default to Floating
                                log::warn!(
                                    "Window not found for auto-detection, defaulting to FLOATING"
                                );
                                gevrek_model::graph::WindowMode::Floating
                            }
                        } else {
                            // Explicit mode (Tiled or Floating)
                            *mode
                        };

                        // Add window to workspace based on determined mode
                        match actual_mode {
                            gevrek_model::graph::WindowMode::Auto => {
                                // Should never reach here, but treat as Floating fallback
                                workspace.add_floating_window(handle);
                                log::info!(
                                    "   └─ Added as FLOATING (Auto fallback) to workspace '{}'",
                                    workspace.name
                                );
                            }
                            gevrek_model::graph::WindowMode::Tiled => {
                                workspace.add_window(handle, true);
                                log::info!(
                                    "   └─ Added as TILED to workspace '{}' ({} containers now)",
                                    workspace.name,
                                    workspace.containers.len()
                                );

                                // TILING WM: Remove min/max buttons from tiled windows
                                if let Err(e) = effect(
                                    window_manager,
                                    Effect::Platform(PlatformEffect::Windows(
                                        WindowsEffect::CaptionButtons {
                                            id: handle,
                                            shown: false,
                                        },
                                    )),
                                ) {
                                    log::warn!("Failed to remove min/max buttons: {}", e);
                                }

                                self.apply_workspace_layout_and_position(
                                    workspace,
                                    work_area,
                                    window_manager,
                                )?;
                                log::info!(
                                    "   └─ Applied {:?} layout to {} containers",
                                    workspace.layout,
                                    workspace.containers.len()
                                );
                            }
                            gevrek_model::graph::WindowMode::Floating => {
                                workspace.add_floating_window(handle);
                                log::info!(
                                    "   └─ Added as FLOATING to workspace '{}'",
                                    workspace.name
                                );
                                // Floating windows keep their current position/size

                                // Restore min/max buttons for floating windows
                                if let Err(e) = effect(
                                    window_manager,
                                    Effect::Platform(PlatformEffect::Windows(
                                        WindowsEffect::CaptionButtons {
                                            id: handle,
                                            shown: true,
                                        },
                                    )),
                                ) {
                                    log::warn!("Failed to restore min/max buttons: {}", e);
                                }

                                // Create border for floating windows too (if borders enabled)
                                if workspace.border_enabled
                                    && !queries(window_manager).has_border(handle)
                                {
                                    let is_focused = window_manager.active_window() == Some(handle);
                                    // (With the look floating windows
                                    // have, where they have their own.)
                                    match Self::dress_window(
                                        workspace,
                                        window_manager,
                                        handle,
                                        true,
                                        is_focused,
                                    )
                                    .then_some(())
                                    .ok_or_else(|| anyhow::anyhow!("no border was drawn"))
                                    {
                                        Ok(_) => log::info!(
                                            "   └─ Border created for floating window {:?}",
                                            handle
                                        ),
                                        Err(e) => log::warn!(
                                            "Failed to create border for floating window {:?}: {}",
                                            handle,
                                            e
                                        ),
                                    }
                                }
                            }
                        }
                    }

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));

                    // Note: suppressed_windows cleanup happens in process_pending_drags
                } else {
                    log::warn!("SetWindowArrangement: No window provided on input pin");
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Z-Order Management Nodes
            NodeType::BringToFront => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    self.apply_window_command_to(
                        window_manager,
                        monitor_manager,
                        handle,
                        &DispatchCommand::RaiseWindow {
                            selector: WindowSelector::Handle(handle.0 as i64),
                        },
                    )?;
                    log::info!("Brought window to front");

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SendToBack => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    self.apply_window_command_to(
                        window_manager,
                        monitor_manager,
                        handle,
                        &DispatchCommand::LowerWindow {
                            selector: WindowSelector::Handle(handle.0 as i64),
                        },
                    )?;
                    log::info!("Sent window to back");

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetAlwaysOnTop { enabled } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let enabled_val = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Bool(b) = v {
                                Some(b)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*enabled);

                    window_manager.apply(&Effect::Stack {
                        id: handle,
                        op: StackOp::AlwaysOnTop(enabled_val),
                    })?;
                    log::info!("Set always on top: {} for window", enabled_val);

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Windows 11 DWM feature nodes
            NodeType::CloakWindow { cloak } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    let cloak_val = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Bool(b) = v {
                                Some(b)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(*cloak);

                    window_manager.apply(&Effect::Platform(PlatformEffect::Windows(
                        WindowsEffect::Cloak {
                            id: handle,
                            cloaked: cloak_val,
                        },
                    )))?;
                    log::info!("Cloaked window: {}", cloak_val);

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetNativeBorderColor { r, g, b } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    // Get color from input pin or use default
                    let color = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Color(r, g, b) = v {
                                Some((r, g, b))
                            } else {
                                None
                            }
                        })
                        .unwrap_or((*r, *g, *b));

                    window_manager.apply(&Effect::Platform(PlatformEffect::Windows(
                        WindowsEffect::NativeBorderColor {
                            id: handle,
                            color: gevrek_model::settings::RuleColor::Rgb {
                                r: color.0,
                                g: color.1,
                                b: color.2,
                            },
                        },
                    )))?;
                    log::info!("Set native border color for window");

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetCaptionColor { r, g, b } => {
                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    // Get color from input pin or use default
                    let color = self
                        .get_input_value(node_id, 2)
                        .and_then(|v| {
                            if let Value::Color(r, g, b) = v {
                                Some((r, g, b))
                            } else {
                                None
                            }
                        })
                        .unwrap_or((*r, *g, *b));

                    window_manager.apply(&Effect::Platform(PlatformEffect::Windows(
                        WindowsEffect::CaptionColor {
                            id: handle,
                            color: gevrek_model::settings::RuleColor::Rgb {
                                r: color.0,
                                g: color.1,
                                b: color.2,
                            },
                        },
                    )))?;
                    log::info!("Set caption color for window");

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetSystemBackdrop { backdrop_type } => {
                use gevrek_model::graph::SystemBackdropType;
                use gevrek_model::platform::windows::SystemBackdrop;

                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    // Convert from graph type to dwm type
                    let backdrop = match backdrop_type {
                        SystemBackdropType::Auto => SystemBackdrop::Auto,
                        SystemBackdropType::None => SystemBackdrop::None,
                        SystemBackdropType::Mica => SystemBackdrop::Mica,
                        SystemBackdropType::Acrylic => SystemBackdrop::Acrylic,
                        SystemBackdropType::MicaAlt => SystemBackdrop::MicaAlt,
                    };

                    window_manager.apply(&Effect::Platform(PlatformEffect::Windows(
                        WindowsEffect::Backdrop {
                            id: handle,
                            backdrop,
                        },
                    )))?;
                    log::info!("Set system backdrop for window");

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            NodeType::SetWindowCorner { corner_type } => {
                use gevrek_model::graph::WindowCornerType;
                use gevrek_model::platform::windows::WindowCorner;

                if let Some(Value::WindowHandle(handle)) = self.get_input_value(node_id, 1) {
                    // Convert from graph type to dwm type
                    let corner = match corner_type {
                        WindowCornerType::Default => WindowCorner::Default,
                        WindowCornerType::Square => WindowCorner::DoNotRound,
                        WindowCornerType::Rounded => WindowCorner::Round,
                        WindowCornerType::RoundedSmall => WindowCorner::RoundSmall,
                    };

                    window_manager.apply(&Effect::Platform(PlatformEffect::Windows(
                        WindowsEffect::Corner { id: handle, corner },
                    )))?;
                    log::info!("Set window corner preference");

                    // Pass through window for chaining
                    self.set_output_value(node_id, 1, Value::WindowHandle(handle));
                }

                self.trigger_output_execution(
                    window_manager,
                    monitor_manager,
                    node_id,
                    0,
                    visited,
                )?;
            }

            // Other families are run by their own function.
            _ => {}
        }
        Ok(NodeDone::Normal)
    }
}
