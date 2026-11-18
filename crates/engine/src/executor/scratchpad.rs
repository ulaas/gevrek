//! Scratchpads: toggling, showing, hiding, adopting and launching their windows.

use super::*;

impl Executor {
    pub fn scratchpad_runtime_states(&self) -> Vec<ScratchpadRuntimeState> {
        self.scratchpads.runtime_states()
    }

    pub fn release_removed_scratchpads(
        &mut self,
        profiles: &[ScratchpadProfile],
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) {
        for handle in self.scratchpads.release_removed(profiles) {
            if !queries(window_manager).exists(handle) {
                continue;
            }
            let _ = effect(
                window_manager,
                Effect::SetVisible {
                    id: handle,
                    visible: true,
                },
            );
            if monitor_manager.find_window(handle).is_none() {
                if let Some(monitor) = self.active_monitor(window_manager, monitor_manager) {
                    if let Some(workspace) = monitor_manager.get_focused_workspace_mut(monitor) {
                        workspace.add_floating_window(handle);
                    }
                }
            }
        }
    }

    /// A scratchpad's window is on no workspace: it floats over them.
    /// One that was given a place on a workspace since (made to float or
    /// to tile there, moved to one, taken back among the managed) is an
    /// ordinary window from then on, and the scratchpad looks for
    /// another when it is asked for. (It stayed the scratchpad's, and
    /// was shown from another workspace later: in sight, and on one that
    /// was not.)
    pub(super) fn release_placed_scratchpads(&mut self, monitor_manager: &MonitorManager) {
        self.scratchpads
            .retain_existing(|window| monitor_manager.find_window(window).is_none());
    }

    pub fn reassert_scratchpads(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
    ) {
        for (id, handle, visible) in self.scratchpads.bindings() {
            let Some(profile) = self.scratchpads.profile(&id).cloned() else {
                continue;
            };
            if !queries(window_manager).exists(handle) {
                self.scratchpads.unbind(&id);
                continue;
            }
            let _ = monitor_manager.find_and_remove_window(handle);
            self.position_scratchpad(window_manager, monitor_manager, handle, &profile);
            if visible {
                let _ = effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: true,
                    },
                );
            } else {
                let _ = effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: false,
                    },
                );
            }
        }
    }

    pub(super) fn legacy_launch_profile(command: &str) -> Option<LaunchProfile> {
        let parts = command.split_whitespace().collect::<Vec<_>>();
        let (program, args) = parts.split_first()?;
        Some(LaunchProfile {
            program: (*program).to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            ..LaunchProfile::default()
        })
    }

    pub(super) fn resolve_scratchpad_monitor(
        &self,
        window_manager: &dyn Desktop,
        monitor_manager: &MonitorManager,
        selector: &MonitorSelector,
    ) -> Option<MonitorHandle> {
        scratchpads::resolve_output(
            monitor_manager,
            selector,
            self.active_monitor(window_manager, monitor_manager),
        )
    }

    pub(super) fn launch_scratchpad(
        window_manager: &mut dyn Desktop,
        profile: &ScratchpadProfile,
    ) -> Result<()> {
        let Some(launch) = profile.launch.as_ref() else {
            anyhow::bail!(
                "No matching window exists for scratchpad '{}' and no launch command is configured",
                profile.id
            );
        };
        window_manager.launch(launch).map(|_| ()).map_err(|error| {
            anyhow::anyhow!(
                "Could not launch scratchpad '{}' with '{}': {}",
                profile.id,
                launch.program,
                error
            )
        })
    }

    pub(super) fn adopt_scratchpad(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        normalized_id: &str,
        profile: &ScratchpadProfile,
        handle: WindowHandle,
        show: bool,
    ) -> Result<()> {
        if let Some((source_monitor, _)) = monitor_manager.find_and_remove_window(handle) {
            self.mark_layout_dirty(source_monitor);
        }
        self.position_scratchpad(window_manager, monitor_manager, handle, profile);
        if show {
            effect(
                window_manager,
                Effect::SetVisible {
                    id: handle,
                    visible: true,
                },
            )?;
            effect(window_manager, Effect::Focus(Some(handle)))?;
        } else {
            effect(
                window_manager,
                Effect::SetVisible {
                    id: handle,
                    visible: false,
                },
            )?;
        }
        self.scratchpads.bind(normalized_id, handle, show);
        log::info!("Adopted window {:?} as scratchpad '{}'", handle, profile.id);
        Ok(())
    }

    pub(super) fn adopt_pending_scratchpad(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
    ) -> Result<()> {
        for (id, profile) in self.scratchpads.pending_profiles() {
            if crate::matching::window_matches(
                window_manager,
                handle,
                &profile.pattern,
                &profile.match_kind,
                &profile.match_strategy,
                profile.case_sensitive,
            ) {
                return self.adopt_scratchpad(
                    window_manager,
                    monitor_manager,
                    &id,
                    &profile,
                    handle,
                    true,
                );
            }
        }
        Ok(())
    }

    pub fn toggle_scratchpad(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        id: &str,
    ) -> Result<()> {
        self.release_placed_scratchpads(monitor_manager);
        let toggle = self
            .scratchpads
            .toggle(id, |handle| queries(window_manager).exists(handle))?;
        let (profile, assigned) = match toggle {
            Toggle::Waiting => {
                log::debug!("Scratchpad '{id}' is already waiting for its launched window");
                return Ok(());
            }
            Toggle::Hide(handle) => {
                effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: false,
                    },
                )?;
                self.scratchpads.set_visible(id, false);
                return Ok(());
            }
            Toggle::Show {
                window: handle,
                profile,
            } => {
                self.position_scratchpad(window_manager, monitor_manager, handle, &profile);
                effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: true,
                    },
                )?;
                effect(window_manager, Effect::Focus(Some(handle)))?;
                self.scratchpads.set_visible(id, true);
                return Ok(());
            }
            Toggle::Find { profile, assigned } => (profile, assigned),
        };

        if let Some(handle) = crate::matching::find_windows_by_match(
            window_manager,
            &profile.pattern,
            &profile.match_kind,
            &profile.match_strategy,
            profile.case_sensitive,
        )
        .into_iter()
        .find(|handle| !assigned.contains(handle))
        {
            return self.adopt_scratchpad(
                window_manager,
                monitor_manager,
                &scratchpads::normalize_id(id),
                &profile,
                handle,
                true,
            );
        }

        Self::launch_scratchpad(window_manager, &profile)?;
        self.scratchpads.mark_pending(id);
        log::info!(
            "Launched scratchpad '{}'; waiting for its window",
            profile.id
        );
        Ok(())
    }

    pub(super) fn show_scratchpad(
        &mut self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        id: &str,
    ) -> Result<()> {
        self.release_placed_scratchpads(monitor_manager);
        if self
            .scratchpads
            .window(id)
            .is_some_and(|(_, visible)| visible)
        {
            return Ok(());
        }
        self.toggle_scratchpad(window_manager, monitor_manager, id)
    }

    pub(super) fn hide_scratchpad(
        &mut self,
        window_manager: &mut dyn Desktop,
        id: &str,
    ) -> Result<()> {
        if let Some((handle, visible)) = self.scratchpads.window(id) {
            if visible && queries(window_manager).exists(handle) {
                effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: false,
                    },
                )?;
            }
            self.scratchpads.set_visible(id, false);
        }
        Ok(())
    }

    pub(super) fn hide_all_scratchpads(&mut self, window_manager: &mut dyn Desktop) {
        for handle in self.scratchpads.hide_all() {
            if queries(window_manager).exists(handle) {
                let _ = effect(
                    window_manager,
                    Effect::SetVisible {
                        id: handle,
                        visible: false,
                    },
                );
            }
        }
    }

    /// Position a scratchpad window according to its configuration.
    pub(super) fn position_scratchpad(
        &self,
        window_manager: &mut dyn Desktop,
        monitor_manager: &mut MonitorManager,
        handle: WindowHandle,
        profile: &ScratchpadProfile,
    ) {
        let work_area = self
            .resolve_scratchpad_monitor(window_manager, monitor_manager, &profile.monitor)
            .and_then(|monitor| monitor_manager.get_adjusted_work_area(monitor, window_manager));

        let Some(area) = work_area else { return };

        let rect = scratchpads::placement(profile, area);
        let _ = effect(
            window_manager,
            Effect::Configure {
                id: handle,
                rect: gevrek_model::geometry::Rect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                },
                states: WindowStates::default(),
            },
        );
    }
}
