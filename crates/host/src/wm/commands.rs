//! Commands: from the server (the CLI, the editor, the shell), from the
//! bindings, from the graph.
//!
//! Most commands are the core's (`WmCore::dispatch`). Some are the host's
//! own: the binding submaps, the startup programs, the toggles that are
//! mirrored into the configuration (so that a reload does not undo them),
//! what is only passed on to the shell. A few only some systems can carry
//! out (picking a window, the keyboard layout, the night light's preview);
//! each of those is a hook of [`WmHost`] whose default says so.
//!
//! Every command is told to the event stream as having succeeded or
//! failed, and is followed by the backend's `settle` (the layouts it
//! changed are laid out, the focus effects follow).

use super::bindings::Bindings;
use super::published::Published;
use super::WmHost;
use crate::command::{run_dispatch_batch, ManualCommand, RuntimeCommand, RuntimeCommandResult};
use gevrek_engine::executor::{Executor, RuntimeEventKind};
use gevrek_model::command::{DispatchCommand, ShellAction};
use gevrek_model::settings::ReservedArea;

/// The host's handling of commands, for every backend.
pub trait Commands: WmHost {
    /// Run every command the server queued since the last pass and answer
    /// each. Returns whether there were any.
    ///
    /// They are all taken at once (one a pass added a frame's wait to each
    /// in a queue), so a client that floods the queue cannot starve the
    /// window events.
    fn run_queued_commands(&mut self) -> bool {
        let commands: Vec<ManualCommand> = self
            .host()
            .command_receiver
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        if commands.is_empty() {
            return false;
        }
        self.noted_work();
        for command in commands {
            // The caller gives up after its timeout and drops the receiver:
            // run now, the command would be applied after the caller was
            // told it failed.
            if command.caller_gave_up() {
                log::warn!(
                    "Skipping command {:?}: the caller already timed out",
                    command.command
                );
                continue;
            }
            let result = match &command.command {
                // A built-in alias runs as its command; a number names a
                // Manual Trigger node of the graph.
                RuntimeCommand::Trigger(target) => match Executor::manual_target_command(target) {
                    Ok(Some(dispatch)) => self.run_command(dispatch),
                    Ok(None) => {
                        let parts = self.parts();
                        let result = parts.core.executor.execute_manual_target(
                            parts.desktop,
                            &mut parts.core.monitor_manager,
                            target,
                        );
                        self.settle();
                        result
                    }
                    Err(error) => Err(error),
                }
                .map(|_| RuntimeCommandResult::Complete),
                RuntimeCommand::Dispatch(dispatch) => self
                    .run_command(dispatch.clone())
                    .map(|_| RuntimeCommandResult::Complete),
                RuntimeCommand::DispatchBatch {
                    commands,
                    stop_on_error,
                } => {
                    let results = run_dispatch_batch(commands, *stop_on_error, |dispatch| {
                        self.run_command(dispatch.clone())
                            .map_err(|error| error.to_string())
                    });
                    Ok(RuntimeCommandResult::Batch(results))
                }
                RuntimeCommand::InjectInput(steps) => self
                    .inject_input(steps)
                    .map(|_| RuntimeCommandResult::Complete),
            }
            .map_err(|error| error.to_string());
            if let Err(error) = &result {
                log::warn!("Command {:?} failed: {error}", command.command);
            }
            if command.response.send(result).is_err() {
                log::debug!("the answer to a command was not given: who asked is gone");
            }
        }
        true
    }

    /// Carry out a command from a binding, the CLI, gRPC or the editor, and
    /// tell the event stream how it went.
    fn run_command(&mut self, command: DispatchCommand) -> anyhow::Result<()> {
        self.host().state_stale.set(true);
        let name = command.display_name().to_string();
        let payload = serde_json::to_string(&command).unwrap_or_else(|_| name.clone());
        let result = self.run_command_unsettled(command);
        self.settle();
        let mut fields = vec![
            ("command".to_string(), name),
            ("payload".to_string(), payload),
        ];
        let kind = match &result {
            Ok(()) => RuntimeEventKind::DispatchSucceeded,
            Err(error) => {
                fields.push(("error".to_string(), error.to_string()));
                RuntimeEventKind::DispatchFailed
            }
        };
        self.host().emit(kind, None, fields);
        result
    }

    /// The command itself, before the backend settles.
    fn run_command_unsettled(&mut self, command: DispatchCommand) -> anyhow::Result<()> {
        match command {
            DispatchCommand::SetBindingSubmap { name } => {
                let name = name.trim().to_ascii_lowercase();
                if name.is_empty() {
                    anyhow::bail!("Binding submap name cannot be empty");
                }
                if name != "default"
                    && !self
                        .config()
                        .desktop
                        .bindings
                        .iter()
                        .any(|binding| binding.enabled && binding.normalized_submap() == name)
                {
                    anyhow::bail!("No enabled binding submap named '{name}' exists");
                }
                self.switch_submap(name)
            }
            DispatchCommand::ResetBindingSubmap => self.switch_submap("default".to_string()),
            DispatchCommand::Quit => {
                log::info!("Quit: the window manager stops");
                // (By the way every stop goes: the loop ends at its next
                // pass, and what it started is stopped.)
                if !self.host().request_stop() {
                    self.stop_without_server();
                }
                Ok(())
            }
            DispatchCommand::ExpectWindow { output } => self.expect_window(output),
            DispatchCommand::PickWindow => self.pick_window(),
            DispatchCommand::WorkspaceOverview => self.workspace_overview(),
            DispatchCommand::KeyboardLayout { target } => self.keyboard_layout(target),
            DispatchCommand::ShortcutsInhibit => self.shortcuts_inhibit(),
            DispatchCommand::OutputTrial { answer } => {
                if self.answer_output_trial(answer) {
                    Ok(())
                } else {
                    anyhow::bail!("No display settings are on trial")
                }
            }
            DispatchCommand::Shell { action } => {
                self.host().emit(
                    RuntimeEventKind::ShellCommand,
                    None,
                    [("action".to_string(), action.id())],
                );
                self.shell_action(action);
                // The night light is the window manager's: its key switches
                // it on or off (within its hours, when the configuration
                // gives it hours), and the configuration says so afterwards.
                if action == ShellAction::NightLight {
                    self.switch_night_light();
                }
                Ok(())
            }
            DispatchCommand::NightLightPreview {
                temperature,
                seconds,
            } => self.preview_night_light(temperature, seconds),
            // What the shell shows for someone else: told as its keys are.
            command @ (DispatchCommand::Notify { .. }
            | DispatchCommand::Osd { .. }
            | DispatchCommand::AppShortcut { .. }
            | DispatchCommand::Zones { .. }) => {
                self.host().emit(
                    RuntimeEventKind::ShellCommand,
                    None,
                    command.shell_fields().unwrap_or_default(),
                );
                Ok(())
            }
            DispatchCommand::ShowShortcuts => self.show_shortcuts(),
            DispatchCommand::Exec { program, args } => {
                let launch = gevrek_model::settings::LaunchProfile {
                    program,
                    args,
                    ..Default::default()
                };
                gevrek_engine::Desktop::launch(self.parts().desktop, &launch).map(|pid| {
                    log::info!("Started '{}' as PID {pid}", launch.program);
                })
            }
            DispatchCommand::LaunchStartup { id } => self.parts().host.startup.launch(&id),
            DispatchCommand::StopStartup { id } => self.parts().host.startup.stop(&id),
            DispatchCommand::RestartStartup { id } => self.parts().host.startup.restart(&id),
            // The toggles below are mirrored into the running configuration,
            // so that a later reload (which applies the configuration again)
            // does not undo them; the configuration is published with them.
            DispatchCommand::ToggleMouseFollowsFocus => {
                let parts = self.parts();
                let enabled = !parts.core.focus_manager.mouse_follows_focus();
                parts.core.focus_manager.set_mouse_follows_focus(enabled);
                parts.config.desktop.focus.mouse_follows_focus = enabled;
                parts.host.config_dirty = true;
                log::info!(
                    "Mouse-follows-focus {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                Ok(())
            }
            DispatchCommand::WatchConfiguration { enabled } => {
                let parts = self.parts();
                match &mut parts.host.watcher {
                    Some(watcher) => watcher.set_enabled(enabled),
                    None => log::warn!(
                        "Configuration watch toggled but the window manager has no configuration file"
                    ),
                }
                parts.config.desktop.watch_configuration = enabled;
                parts.host.config_dirty = true;
                log::info!(
                    "Configuration file watch {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                Ok(())
            }
            DispatchCommand::ToggleStackbar => {
                let enabled = {
                    let parts = self.parts();
                    let enabled = !parts.config.desktop.appearance.stackbar.enabled;
                    parts.config.desktop.appearance.stackbar.enabled = enabled;
                    parts.host.config_dirty = true;
                    enabled
                };
                self.apply_appearance();
                // Every output is laid out again, so that containers keep
                // (or give back) the stackbar's strip.
                mark_every_output(self);
                log::info!("Stackbar {}", if enabled { "enabled" } else { "disabled" });
                Ok(())
            }
            DispatchCommand::ToggleTransparency => {
                // Off: every window is drawn opaque (the focus effects
                // follow when the backend settles).
                let parts = self.parts();
                let enabled = !parts.config.desktop.appearance.transparency_enabled;
                parts.config.desktop.appearance.transparency_enabled = enabled;
                parts.host.config_dirty = true;
                log::info!(
                    "Transparency {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                Ok(())
            }
            DispatchCommand::SetTransparencyAlpha { alpha } => {
                let parts = self.parts();
                let alpha = alpha.max(20);
                parts.config.desktop.appearance.inactive_window_opacity = alpha;
                parts.host.config_dirty = true;
                log::info!("Transparency alpha set to {alpha}");
                Ok(())
            }
            DispatchCommand::SetGlobalWorkAreaOffset {
                left,
                top,
                right,
                bottom,
            } => {
                let offset = ReservedArea {
                    left: left.clamp(0, 10_000),
                    top: top.clamp(0, 10_000),
                    right: right.clamp(0, 10_000),
                    bottom: bottom.clamp(0, 10_000),
                };
                {
                    let parts = self.parts();
                    parts.config.desktop.work_area_offset = offset;
                    parts.host.config_dirty = true;
                    parts.core.monitor_manager.set_global_work_area_offset(offset);
                }
                mark_every_output(self);
                log::info!(
                    "Global work-area offset set to {},{},{},{}",
                    offset.left,
                    offset.top,
                    offset.right,
                    offset.bottom
                );
                Ok(())
            }
            command => {
                let parts = self.parts();
                // (What the graph computed for its last run is not this
                // command's.)
                parts.core.executor.clear_state();
                parts.core.dispatch(parts.desktop, command)
            }
        }
    }

    /// Make `name` the live binding submap and take its keys.
    fn switch_submap(&mut self, name: String) -> anyhow::Result<()> {
        if self.host().active_submap == name {
            return Ok(());
        }
        self.parts().host.active_submap = name;
        self.refresh_bindings()?;
        log::debug!(
            "Active binding submap is now '{}'",
            self.host().active_submap
        );
        // Told at once: who shows the keys' mode (the panel) asks for the
        // state as soon as the command is done.
        self.publish_state(Some("keys' mode".to_string()));
        Ok(())
    }

    /// Switch the night light on or off, and say on the screen what that
    /// does: outside its hours nothing turns warm yet, and that must not
    /// look like a key that does nothing.
    fn switch_night_light(&mut self) {
        let text = {
            let parts = self.parts();
            let light = &mut parts.config.desktop.night_light;
            light.enabled = !light.enabled;
            parts.host.config_dirty = true;
            log::info!(
                "The night light is switched {}",
                if light.enabled { "on" } else { "off" }
            );
            light.said_when_switched(minutes_now())
        };
        self.night_light_switched();
        let said = DispatchCommand::Osd { text, level: None };
        self.host().emit(
            RuntimeEventKind::ShellCommand,
            None,
            said.shell_fields().unwrap_or_default(),
        );
    }
}

impl<T: WmHost + ?Sized> Commands for T {}

/// Mark every output to be laid out again (at the next settle).
fn mark_every_output<H: WmHost + ?Sized>(host: &mut H) {
    let parts = host.parts();
    for monitor in parts.core.monitor_manager.get_all_monitors_ordered() {
        parts.core.executor.mark_layout_dirty(monitor.handle);
    }
}

/// The minutes of the day now, by the machine's clock and zone.
fn minutes_now() -> u32 {
    let now = jiff::Zoned::now();
    now.hour() as u32 * 60 + now.minute() as u32
}
