//! Applying window rules: the static rules when a window appears, the dynamic ones when it
//! changes, focus policy, size constraints and remembered sizes.

use super::*;

impl Executor {
    pub(super) fn capture_dynamic_rule_baseline(
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
    ) -> DynamicRuleBaseline {
        DynamicRuleBaseline {
            workspace: monitor_manager.find_window(handle),
            rect: queries(window_manager)
                .rect(handle)
                .map_err(anyhow::Error::msg)
                .ok(),
            managed: monitor_manager.find_window(handle).is_some(),
            floating: monitor_manager.is_window_floating(handle),
            always_on_top: queries(window_manager).is_always_on_top(handle),
            opacity: queries(window_manager).opacity(handle),
            maximized: queries(window_manager).is_maximized(handle),
            fullscreen: queries(window_manager).is_fullscreen(handle),
            border_color: queries(window_manager).border_color(handle),
            caption_color: queries(window_manager).caption_color(handle),
            backdrop: queries(window_manager).backdrop(handle),
            corners: queries(window_manager).corners(handle),
        }
    }

    pub(super) fn apply_rule_resolution(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        resolution: &RuleResolution,
        restore: Option<DynamicRuleBaseline>,
        previous: Option<&RuleResolution>,
    ) -> Result<()> {
        let baseline = restore;
        let current_floating = monitor_manager.is_window_floating(handle);
        let workspace_target = resolution.workspace.and_then(|(monitor_index, workspace)| {
            Self::target_monitor(monitor_manager, handle, monitor_index)
                .map(|monitor| (monitor, workspace))
        });
        let restore_workspace = baseline.and_then(|baseline| {
            previous
                .and_then(|previous| previous.workspace)
                .and(baseline.workspace)
        });
        let restore_managed = baseline.and_then(|baseline| {
            previous
                .and_then(|previous| previous.managed)
                .map(|_| baseline.managed)
        });
        let desired_managed = resolution.managed.or(restore_managed);
        let restore_floating = baseline.and_then(|baseline| {
            previous
                .and_then(|previous| previous.floating)
                .map(|_| baseline.floating)
        });

        if desired_managed == Some(false) {
            if let Some((source_monitor, _)) = monitor_manager.find_and_remove_window(handle) {
                self.mark_layout_dirty(source_monitor);
            }
            try_effect(
                window_manager,
                Effect::Border {
                    id: handle,
                    change: BorderChange::Remove,
                },
            );
            try_effect(
                window_manager,
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionButtons {
                    id: handle,
                    shown: true,
                })),
            );
            try_effect(
                window_manager,
                Effect::SetVisible {
                    id: handle,
                    visible: true,
                },
            );
        } else if desired_managed == Some(true)
            && monitor_manager.find_window(handle).is_none()
            && !self.window_state.get(handle).session_unmanaged
        {
            let target = workspace_target
                .or_else(|| baseline.and_then(|baseline| baseline.workspace))
                .or_else(|| {
                    monitor_manager
                        .get_primary_monitor_handle()
                        .and_then(|monitor| {
                            monitor_manager
                                .get_workspaces(monitor)
                                .map(|workspaces| (monitor, workspaces.focused_idx()))
                        })
                });
            if let Some((monitor, workspace)) = target {
                let floating = resolution
                    .floating
                    .or(restore_floating)
                    .or(baseline.map(|baseline| baseline.floating))
                    .unwrap_or(false);
                self.move_window_to_workspace(
                    window_manager,
                    monitor_manager,
                    handle,
                    monitor,
                    workspace,
                    floating,
                )?;
            }
        }

        let is_managed = monitor_manager.find_window(handle).is_some();
        if is_managed {
            if let Some((monitor, workspace)) = workspace_target.or(restore_workspace) {
                let floating = resolution
                    .floating
                    .or(restore_floating)
                    .unwrap_or(current_floating);
                self.move_window_to_workspace(
                    window_manager,
                    monitor_manager,
                    handle,
                    monitor,
                    workspace,
                    floating,
                )?;
            }
            if let Some(floating) = resolution.floating.or(restore_floating) {
                self.set_window_floating(window_manager, monitor_manager, handle, floating)?;
            }
        }

        // Stacking and the Windows-only look: decided in the engine, carried
        // out as effects.
        let observed = crate::rule_state::ObservedStyle {
            always_on_top: queries(window_manager).is_always_on_top(handle),
            border_color: queries(window_manager).border_color(handle),
            caption_color: queries(window_manager).caption_color(handle),
            backdrop: queries(window_manager).backdrop(handle),
            corners: queries(window_manager).corners(handle),
        };
        for effect in crate::rule_state::style_effects(
            handle,
            resolution,
            previous,
            baseline.as_ref(),
            observed,
        ) {
            window_manager.apply(&effect)?;
        }
        let restore_opacity = baseline.and_then(|baseline| {
            previous
                .and_then(|previous| previous.opacity)
                .map(|_| baseline.opacity)
        });
        if let Some(opacity) = resolution.opacity.or(restore_opacity) {
            if queries(window_manager).opacity(handle) != opacity {
                effect(
                    window_manager,
                    Effect::Opacity {
                        id: handle,
                        alpha: opacity,
                    },
                )?;
            }
        }

        let fullscreen = resolution.fullscreen.then_some(true).or_else(|| {
            previous
                .filter(|previous| previous.fullscreen)
                .and(baseline.map(|state| state.fullscreen))
        });
        if fullscreen == Some(false) {
            effect(
                window_manager,
                Effect::State {
                    id: handle,
                    change: StateChange::Fullscreen(false),
                },
            )?;
        }

        let restore_position = baseline.and_then(|baseline| {
            previous
                .and_then(|previous| previous.position.as_ref())
                .and(baseline.rect)
                .map(|rect| (rect.x, rect.y))
        });
        let restore_size = baseline.and_then(|baseline| {
            previous
                .and_then(|previous| previous.size.as_ref())
                .and(baseline.rect)
                .map(|rect| (rect.width, rect.height))
        });
        if resolution.position.is_some()
            || resolution.size.is_some()
            || restore_position.is_some()
            || restore_size.is_some()
        {
            if let Ok(current) = queries(window_manager)
                .rect(handle)
                .map_err(anyhow::Error::msg)
            {
                let monitor = monitor_of(window_manager, monitor_manager, handle)
                    .map(|monitor| monitor.handle)
                    .or_else(|| monitor_manager.get_primary_monitor_handle());
                if let Some(work_area) = monitor.and_then(|monitor| {
                    monitor_manager.get_adjusted_work_area(monitor, window_manager)
                }) {
                    let target = resolve_rule_rect(
                        current,
                        work_area,
                        resolution.position.as_ref(),
                        resolution.size.as_ref(),
                        restore_position,
                        restore_size,
                    );
                    if target != current {
                        effect(
                            window_manager,
                            Effect::Configure {
                                id: handle,
                                rect: gevrek_model::geometry::Rect {
                                    x: target.x,
                                    y: target.y,
                                    width: target.width,
                                    height: target.height,
                                },
                                states: WindowStates::default(),
                            },
                        )?;
                    }
                }
            }
        }

        // Fullscreen owns its own maximized/style restore snapshot. Do not
        // mutate maximization while entering or remaining fullscreen, or the
        // dynamic effect would capture itself as the baseline.
        if fullscreen != Some(true) {
            let maximize = resolution.maximize.then_some(true).or_else(|| {
                previous
                    .filter(|previous| previous.maximize)
                    .and(baseline.map(|state| state.maximized))
            });
            if let Some(maximize) =
                maximize.filter(|value| queries(window_manager).is_maximized(handle) != *value)
            {
                if maximize {
                    effect(
                        window_manager,
                        Effect::State {
                            id: handle,
                            change: StateChange::Maximized(true),
                        },
                    )?;
                } else {
                    effect(
                        window_manager,
                        Effect::State {
                            id: handle,
                            change: StateChange::Maximized(false),
                        },
                    )?;
                }
            }
        }
        if fullscreen == Some(true) {
            effect(
                window_manager,
                Effect::State {
                    id: handle,
                    change: StateChange::Fullscreen(true),
                },
            )?;
        }
        Ok(())
    }

    pub(super) fn persistent_size_key(
        window_manager: &dyn Desktop,
        handle: WindowHandle,
    ) -> PersistentSizeKey {
        constraints::normalize_size_key(
            &queries(window_manager).exe(handle),
            &queries(window_manager).initial_class(handle),
            &queries(window_manager).initial_title(handle),
        )
    }

    pub fn no_initial_focus_enabled(&self, handle: WindowHandle) -> bool {
        self.window_state.get(handle).suppresses_initial_focus()
    }

    pub fn no_focus_enabled(&self, handle: WindowHandle) -> bool {
        self.window_state.get(handle).rejects_focus()
    }

    /// Whether focus-follows-mouse may select this managed window.
    pub fn allows_follow_mouse_focus(&self, handle: WindowHandle) -> bool {
        self.window_state.get(handle).allows_follow_mouse()
    }

    pub(super) fn arm_initial_focus_suppression(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        focus_manager: &mut FocusManager,
        handle: WindowHandle,
    ) -> Result<()> {
        if !self.no_initial_focus_enabled(handle) && !self.no_focus_enabled(handle) {
            return Ok(());
        }
        // EVENT_SYSTEM_FOREGROUND can arrive before EVENT_OBJECT_SHOW. If the
        // window is already active by the time its Created event is processed,
        // it is suppressed now; otherwise its focus event consumes the pending
        // one-shot when it arrives.
        let is_active = queries(window_manager).active_window() == Some(handle);
        if let InitialFocus::SuppressNow { never } =
            focus::arm_initial_suppression(&mut self.window_state, handle, is_active)
        {
            let target = self.previous_focus_candidate(window_manager, focus_manager, handle);
            let restored = self.restore_previous_focus(
                window_manager,
                monitor_manager,
                focus_manager,
                handle,
                target,
            )?;
            if never {
                if !restored {
                    try_effect(window_manager, Effect::Focus(None));
                }
                focus_manager.remove_window(handle);
            }
        }
        Ok(())
    }

    pub(super) fn update_persistent_size_rule(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        dynamic: bool,
        setting: Option<bool>,
    ) -> Result<()> {
        let floating = monitor_manager.is_window_floating(handle);
        let restore = constraints::set_persistent_size_rule(
            self.window_state.get_mut(handle),
            &self.persistent_window_sizes,
            dynamic,
            setting,
            floating,
            || Self::persistent_size_key(window_manager, handle),
        );
        let Some((width, height)) = restore else {
            return Ok(());
        };
        let current = queries(window_manager)
            .rect(handle)
            .map_err(anyhow::Error::msg)?;
        if current.width != width || current.height != height {
            effect(
                window_manager,
                Effect::Configure {
                    id: handle,
                    rect: gevrek_model::geometry::Rect {
                        x: current.x,
                        y: current.y,
                        width,
                        height,
                    },
                    states: WindowStates::default(),
                },
            )?;
            log::debug!(
                "Restored persistent floating size {}x{} for {:?}",
                width,
                height,
                handle
            );
        }
        Ok(())
    }

    pub(super) fn remember_persistent_window_size(
        &mut self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        width: i32,
        height: i32,
    ) {
        let floating = monitor_manager.is_window_floating(handle);
        if constraints::remember_size(
            self.window_state.get_mut(handle),
            &mut self.persistent_window_sizes,
            floating,
            width,
            height,
            || Self::persistent_size_key(window_manager, handle),
        ) {
            log::trace!(
                "Remembered persistent floating size {}x{} for {:?}",
                width,
                height,
                handle
            );
        }
    }

    pub(super) fn enforce_size_constraints(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        width: i32,
        height: i32,
    ) -> Result<(i32, i32)> {
        if !monitor_manager.is_window_floating(handle) {
            return Ok((width, height));
        }
        if !constraints::has_size_constraints(self.window_state.get(handle)) {
            return Ok((width, height));
        }
        let monitor = monitor_manager
            .find_window(handle)
            .map(|(monitor, _)| monitor)
            .or_else(|| {
                monitor_of(window_manager, monitor_manager, handle).map(|monitor| monitor.handle)
            });
        let Some(work_area) = monitor
            .and_then(|monitor| monitor_manager.get_adjusted_work_area(monitor, window_manager))
        else {
            return Ok((width, height));
        };
        let target = constraints::constrained_size(
            self.window_state.get_mut(handle),
            work_area,
            width,
            height,
        );
        if target != (width, height) {
            let current = queries(window_manager)
                .rect(handle)
                .map_err(anyhow::Error::msg)?;
            effect(
                window_manager,
                Effect::Configure {
                    id: handle,
                    rect: gevrek_model::geometry::Rect {
                        x: current.x,
                        y: current.y,
                        width: target.0,
                        height: target.1,
                    },
                    states: WindowStates::default(),
                },
            )?;
            log::trace!(
                "Clamped floating window {:?} from {}x{} to {}x{}",
                handle,
                width,
                height,
                target.0,
                target.1
            );
        }
        Ok(target)
    }

    pub(super) fn update_aspect_ratio_rule(
        &mut self,
        window_manager: &dyn Desktop,
        handle: WindowHandle,
        dynamic: bool,
        setting: Option<bool>,
    ) -> Result<()> {
        let state = self.window_state.get_mut(handle);
        if constraints::set_aspect_ratio_rule(state, dynamic, setting) {
            let current = queries(window_manager)
                .rect(handle)
                .map_err(anyhow::Error::msg)?;
            constraints::seed_aspect_ratio(
                self.window_state.get_mut(handle),
                (current.width, current.height),
            );
        }
        Ok(())
    }

    pub(super) fn update_size_constraint_rules(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &MonitorManager,
        handle: WindowHandle,
        dynamic: bool,
        min_size: Option<RuleSize>,
        max_size: Option<RuleSize>,
    ) -> Result<()> {
        constraints::set_size_rules(
            self.window_state.get_mut(handle),
            dynamic,
            min_size,
            max_size,
        );
        let current = queries(window_manager)
            .rect(handle)
            .map_err(anyhow::Error::msg)?;
        let _ = self.enforce_size_constraints(
            window_manager,
            monitor_manager,
            handle,
            current.width,
            current.height,
        )?;
        Ok(())
    }

    pub(super) fn apply_initial_window_rules(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        let (workspace, workspace_index) = Self::window_workspace_context(monitor_manager, handle);
        let static_resolution = self.resolve_rules(
            window_manager,
            handle,
            workspace,
            workspace_index,
            RuleResolutionScope::Static,
        );
        self.rule_metadata.apply_static(handle, &static_resolution);
        let state = self.window_state.get_mut(handle);
        *state.stay_focused.source_mut(false) = static_resolution.stay_focused;
        *state.no_initial_focus.source_mut(false) = static_resolution.no_initial_focus;
        *state.no_focus.source_mut(false) = static_resolution.no_focus;
        *state.no_follow_mouse.source_mut(false) = static_resolution.no_follow_mouse;
        self.update_aspect_ratio_rule(
            window_manager,
            handle,
            false,
            static_resolution.keep_aspect_ratio,
        )?;
        self.apply_rule_resolution(
            window_manager,
            monitor_manager,
            handle,
            &static_resolution,
            None,
            None,
        )?;
        self.update_persistent_size_rule(
            window_manager,
            monitor_manager,
            handle,
            false,
            static_resolution.persistent_size,
        )?;
        self.update_size_constraint_rules(
            window_manager,
            monitor_manager,
            handle,
            false,
            static_resolution.min_size.clone(),
            static_resolution.max_size.clone(),
        )?;
        self.reevaluate_dynamic_rules(window_manager, monitor_manager, handle)
    }

    pub fn reevaluate_dynamic_rules(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        if !self.window_state.get(handle).known || !queries(window_manager).exists(handle) {
            return Ok(());
        }
        let (workspace, workspace_index) = Self::window_workspace_context(monitor_manager, handle);
        let resolution = self.resolve_rules(
            window_manager,
            handle,
            workspace,
            workspace_index,
            RuleResolutionScope::Dynamic,
        );
        let previous = self.window_state.get(handle).dynamic_rules.clone();
        let baseline = previous
            .as_ref()
            .map(|state| state.baseline)
            .unwrap_or_else(|| {
                Self::capture_dynamic_rule_baseline(window_manager, monitor_manager, handle)
            });
        self.rule_metadata.apply_dynamic(handle, &resolution);
        let state = self.window_state.get_mut(handle);
        *state.stay_focused.source_mut(true) = resolution.stay_focused;
        *state.no_initial_focus.source_mut(true) = resolution.no_initial_focus;
        *state.no_focus.source_mut(true) = resolution.no_focus;
        *state.no_follow_mouse.source_mut(true) = resolution.no_follow_mouse;
        self.update_aspect_ratio_rule(window_manager, handle, true, resolution.keep_aspect_ratio)?;
        self.apply_rule_resolution(
            window_manager,
            monitor_manager,
            handle,
            &resolution,
            Some(baseline),
            previous.as_ref().map(|state| &state.resolution),
        )?;
        self.update_persistent_size_rule(
            window_manager,
            monitor_manager,
            handle,
            true,
            resolution.persistent_size,
        )?;
        self.update_size_constraint_rules(
            window_manager,
            monitor_manager,
            handle,
            true,
            resolution.min_size.clone(),
            resolution.max_size.clone(),
        )?;
        self.window_state.get_mut(handle).dynamic_rules = Some(DynamicRuleState {
            baseline,
            resolution,
        });
        Ok(())
    }

    pub fn reevaluate_all_dynamic_rules(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) -> Result<()> {
        let handles = self.window_state.ids_where(|state| state.known);
        // One window whose rules cannot be carried out (the workspace a
        // rule had moved it from is no longer there) does not keep the
        // rules of all the others, or a whole reload, from being applied.
        for handle in handles {
            if let Err(error) =
                self.reevaluate_dynamic_rules(window_manager, monitor_manager, handle)
            {
                log::warn!("The rules of {handle:?} were not applied again: {error:#}");
            }
        }
        Ok(())
    }

    /// Resolve the rules for a window against its tags, which the executor
    /// holds.
    pub fn resolve_rules(
        &self,
        window_manager: &dyn Desktop,
        handle: WindowHandle,
        workspace: Option<String>,
        workspace_index: Option<usize>,
        scope: RuleResolutionScope,
    ) -> RuleResolution {
        crate::counts::count(crate::counts::Counted::RulesResolved);
        let Some(mut window) = window_manager.rule_facts(handle) else {
            return RuleResolution::default();
        };
        window.tags = match scope {
            RuleResolutionScope::All => self.rule_metadata.tags(handle),
            RuleResolutionScope::Static | RuleResolutionScope::Dynamic => {
                self.rule_metadata.base_tags(handle)
            }
        };
        window.workspace = workspace;
        window.workspace_index = workspace_index;
        let Ok(engine) = self.rule_engine.lock() else {
            return RuleResolution::default();
        };
        match scope {
            RuleResolutionScope::All => engine.resolve(&window),
            RuleResolutionScope::Static => engine.resolve_static(&window),
            RuleResolutionScope::Dynamic => engine.resolve_dynamic(&window),
        }
    }

    /// Add a tag by command; the window must still exist.
    pub(super) fn add_window_tag(
        &mut self,
        window_manager: &dyn Desktop,
        window: WindowHandle,
        tag: &str,
    ) -> Result<bool> {
        if !queries(window_manager).exists(window) {
            anyhow::bail!("Window {:?} no longer exists", window);
        }
        self.rule_metadata
            .add_tag(window, tag)
            .map_err(anyhow::Error::msg)
    }
}
