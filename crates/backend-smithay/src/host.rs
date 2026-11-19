//! The WM host on Wayland: what `Runtime` does on Windows around the core.
//! It takes the commands and configuration requests of the IPC server,
//! applies configuration changes as a diff, runs key bindings, starts the
//! startup programs, and publishes the state the CLI, the editor and the
//! overlay read.
//!
//! The order of the reload steps follows the Windows host
//! (`gevrek-win32`, `runtime/reload.rs`); both call the same engine code.

use crate::state::State;
use gevrek_engine::executor::{
    Executor, NodeExecutionEvent, PendingFocusOp, RuntimeEventKind, RuntimeExtensionEvent,
};
use gevrek_engine::input::{Hotkey, HotkeyEvent};
use gevrek_engine::overlay_data::{GestureStateData, OverlayData, RuleStateData, StartupStateData};
use gevrek_engine::reload::{self as plan, ConfigDiff, ReloadStep};
use gevrek_engine::{Desktop, Effect, WindowHandle};
use gevrek_host::command::{
    run_dispatch_batch, ManualCommand, RuntimeCommand, RuntimeCommandResult,
};
use gevrek_host::config_store::{ConfigPublisher, ConfigRequest};
use gevrek_host::config_watch::ConfigFileWatcher;
use gevrek_model::command::DispatchCommand;
use gevrek_model::config::Config;
use gevrek_model::graph::NodeType;
use gevrek_model::settings::{BindingDevice, BindingTrigger, ReservedArea};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// What connects the WM loop to the IPC server.
pub struct HostLinks {
    pub config_receiver: Receiver<ConfigRequest>,
    pub command_receiver: Receiver<ManualCommand>,
    pub config_publisher: ConfigPublisher,
    /// The configuration file, watched for edits made by hand.
    pub config_path: Option<std::path::PathBuf>,
    pub overlay_data: Arc<Mutex<Option<OverlayData>>>,
    pub pause_flag: Arc<Mutex<bool>>,
    pub stop_flag: Arc<Mutex<bool>>,
}

/// The host state kept beside the links.
pub struct Host {
    pub links: HostLinks,
    pub watcher: Option<ConfigFileWatcher>,
    last_overlay: std::time::Instant,
}

fn flag(flag: &Arc<Mutex<bool>>) -> bool {
    *flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl State {
    /// Connect the WM loop to the IPC server (the publisher already holds
    /// the configuration the WM started with).
    pub fn attach_host(&mut self, links: HostLinks) {
        let watcher = links
            .config_path
            .clone()
            .map(|path| ConfigFileWatcher::new(path, self.config.desktop.watch_configuration));
        self.host = Some(Host {
            links,
            watcher,
            last_overlay: std::time::Instant::now(),
        });
        self.publish_overlay(Some("started".to_string()));
    }

    /// Whether the host has something to advance pass by pass, so that it
    /// may not wait for work: a key held down (its repeat and long press
    /// are timed here), a wallpaper coming in, a border gradient that
    /// turns, a program still to be started.
    pub(crate) fn host_busy(&self) -> bool {
        if !self.held.is_empty() && !self.paused {
            return true;
        }
        // (The pointer rests in a hot corner: its moment is over by time
        // alone, with no event to say so.)
        if self.hot_corners.waits() && !self.paused {
            return true;
        }
        if self.pending_program.is_some()
            || self.wallpapers.borrow().any_fading()
            || self.lock_change.is_some()
            || crate::lock::LEAVE_ASKED.load(std::sync::atomic::Ordering::Relaxed)
            || crate::lock::NEXT_LAYOUT_ASKED.load(std::sync::atomic::Ordering::Relaxed)
        {
            return true;
        }
        let gradient = &self.config.desktop.appearance.effects.gradient;
        gradient.enabled
            && gradient.spin_seconds > 0.0
            && self.config.desktop.animations.enabled
            && !self.desktop.windows.is_empty()
            && !self.locked
    }

    /// One pass of the host: called by the event loop when there is work,
    /// every few milliseconds while something goes on, and ten times a
    /// second otherwise (`session.rs`). Returns false when the WM should
    /// stop.
    pub fn host_tick(&mut self) -> bool {
        gevrek_host::heartbeat::beat();
        let Some(host) = &self.host else {
            return self.running;
        };
        if flag(&host.links.stop_flag) {
            // (A session is not cut off: it goes under its veil first.)
            if self.leaving_before_it_stops() {
                self.tick_lock_change();
                return true;
            }
            log::info!("Stop requested");
            self.running = false;
            return false;
        }

        let commands: Vec<ManualCommand> = host.links.command_receiver.try_iter().collect();
        let had_commands = !commands.is_empty();
        if had_commands {
            self.host_pace.last_work = Some(std::time::Instant::now());
        }
        for command in commands {
            // The caller gave up waiting: running the command now would
            // apply it after the client was told it failed.
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
                        let result = self.core.executor.execute_manual_target(
                            &mut self.desktop,
                            &mut self.core.monitor_manager,
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
            let _ = command.response.send(result);
        }

        // Paused: bindings do not run. Configuration changes are still
        // applied and published.
        self.paused = self
            .host
            .as_ref()
            .is_some_and(|host| flag(&host.links.pause_flag));
        self.apply_configuration_changes();
        self.tick_output_trial();
        self.tick_idle_lock();
        self.tick_idle_displays();
        // A wallpaper that is coming in is drawn until it is there.
        if self.wallpapers.borrow().any_fading()
            || self
                .wallpapers
                .borrow()
                .slide_due(&self.config.desktop.wallpaper.slideshow)
        {
            self.desktop.needs_redraw = true;
        }
        // A theme that takes its accent from the wallpaper follows the
        // picture of the first display, as soon as that has been read.
        let accent = gevrek_model::theme::follows_wallpaper(&self.config.desktop.appearance.theme)
            .then(|| {
                let first = self.desktop.outputs.first()?.id;
                self.wallpapers.borrow().accent(first)
            })
            .flatten();
        if accent != self.core.wallpaper_accent {
            // The borders are made again with the new colour, as after a
            // change of the configuration's appearance.
            let before = gevrek_engine::reload::border_snapshot(&self.core.monitor_manager);
            self.core.wallpaper_accent = accent;
            self.apply_appearance();
            self.core
                .refresh_changed_borders(&mut self.desktop, &before, true);
            self.settle();
            self.desktop.needs_redraw = true;
            self.publish_overlay(Some("accent".to_string()));
        }
        // The turning gradient: a frame when it has moved on a step, and
        // only while there is a window to have a border.
        let angle = self.gradient_angle();
        if angle != self.drawn_gradient_angle {
            self.drawn_gradient_angle = angle;
            if !self.desktop.windows.is_empty() && !self.locked {
                self.desktop.needs_redraw = true;
            }
        }
        self.publish_config_if_changed();
        if self.clients_may_start() {
            if !self.paused {
                self.startup.tick();
            }
            if let Some(program) = self.pending_program.take() {
                // (`--ends-with-program`: gevrek is there for this one
                // program, as the login's greeter is, and ends with it.)
                let stop = self
                    .ends_with_program
                    .then(|| self.host.as_ref().map(|host| host.links.stop_flag.clone()))
                    .flatten();
                let ended = stop.clone();
                let started = crate::launch::spawn_then(&program, move || {
                    if let Some(stop) = ended {
                        log::info!("The program gevrek was started for has ended");
                        if let Ok(mut stop) = stop.lock() {
                            *stop = true;
                        }
                    }
                });
                match started {
                    Ok(pid) => log::info!("Started {} (pid {pid})", program.program),
                    Err(error) => {
                        log::error!("{error}");
                        // (Nothing to be there for.)
                        if let Some(stop) = stop {
                            if let Ok(mut stop) = stop.lock() {
                                *stop = true;
                            }
                        }
                    }
                }
            }
        }
        self.tick_held(std::time::Instant::now());
        self.tick_hot_corners(std::time::Instant::now());
        self.sync_window_overview();
        self.tick_lock_change();

        let due = self.host.as_ref().is_some_and(|host| {
            host.last_overlay.elapsed() >= std::time::Duration::from_millis(250)
        });
        // A floating window its program moved to another display is that
        // display's once it has come to rest (the state that is published
        // next says so).
        if due {
            self.core
                .settle_moved_windows(&mut self.desktop, std::time::Instant::now());
        }
        if had_commands || due || std::mem::take(&mut self.overlay_stale) {
            self.publish_overlay(None);
        }
        if due {
            self.desktop.tick_closing();
            self.refresh_keyboard_layout();
            self.poll_screenshot_request();
            // (Whether a display is still shared or recorded.)
            self.tick_sharing(std::time::Instant::now());
        }
        self.running
    }

    /// Take a configuration request from the server, or an edit of the
    /// file, and apply it.
    fn apply_configuration_changes(&mut self) {
        let Some(host) = &mut self.host else {
            return;
        };
        let mut request = host.links.config_receiver.try_recv().ok();
        if let Some(base) = request.as_ref().and_then(|request| request.base_revision()) {
            // A change made here since the last publish (a toggle) gets its
            // revision first; the request is judged against what runs.
            if std::mem::take(&mut self.config_dirty) {
                host.links.config_publisher.publish(&self.config);
            }
            let running = host.links.config_publisher.revision();
            if running != base {
                log::info!(
                    "Refusing a configuration based on revision {base}: revision {running} is running"
                );
                // The server already saved the refused configuration; put
                // the running one back so the watcher does not load it.
                if let Some(watcher) = &mut host.watcher {
                    if let Err(error) = self.config.save(watcher.path()) {
                        log::warn!("Could not restore the configuration file: {error}");
                    }
                    watcher.acknowledge();
                }
                host.links.config_publisher.refuse_request();
                request = None;
            }
        }
        let from_request = request.is_some();
        let mut new_config = request.map(|request| request.resolve(&self.config));
        if new_config.is_some() {
            // The server wrote the file before asking: not an outside edit.
            if let Some(watcher) = &mut host.watcher {
                watcher.acknowledge();
            }
        } else if let Some(watcher) = &mut host.watcher {
            if watcher.poll() {
                match Config::load(watcher.path()) {
                    Ok(config) => {
                        log::info!(
                            "Configuration file changed on disk; reloading {}",
                            watcher.path().display()
                        );
                        new_config = Some(config);
                    }
                    Err(error) => {
                        log::warn!("Configuration file changed but could not be loaded: {error}");
                    }
                }
            }
        }
        if let Some(new_config) = new_config {
            self.host_pace.last_work = Some(std::time::Instant::now());
            self.reload_configuration(new_config);
            // The state as it is now, before anybody is told that the
            // configuration is: who made the change asks for the state as
            // soon as it hears, and is to get what the change made of it
            // (a workspace's new title, on its tab), not what was there a
            // quarter of a second ago.
            self.publish_overlay(Some("configuration".to_string()));
            if from_request {
                // Its own publish, right away: the server waits for it.
                self.config_dirty = false;
                if let Some(host) = &self.host {
                    let revision = host.links.config_publisher.publish_request(&self.config);
                    log::debug!("Published requested configuration as revision {revision}");
                }
            }
        }
    }

    /// Write the running configuration to the file it was read from (when
    /// there is one), as the file's own content, not as an outside edit.
    pub(crate) fn save_running_configuration(&mut self) {
        let Some(host) = &mut self.host else {
            return;
        };
        if let Some(watcher) = &mut host.watcher {
            if let Err(error) = self.config.save(watcher.path()) {
                log::warn!("Could not write the configuration file: {error}");
            }
            watcher.acknowledge();
        }
    }

    fn publish_config_if_changed(&mut self) {
        if !std::mem::take(&mut self.config_dirty) {
            return;
        }
        if let Some(host) = &self.host {
            let revision = host.links.config_publisher.publish(&self.config);
            log::debug!("Published running configuration revision {revision}");
        }
    }

    /// Apply `new_config` as a diff against the running configuration:
    /// only what changed is touched, and windows stay where they are.
    pub fn reload_configuration(&mut self, new_config: Config) {
        let old = std::mem::replace(&mut self.config, new_config);
        self.config_dirty = true;
        // (Some settings are read as the picture is drawn, the layer
        // rules among them: whatever changed, it is drawn anew.)
        self.desktop.needs_redraw = true;
        if old.desktop.input != self.config.desktop.input {
            self.apply_input_config();
        }
        if old.desktop.wallpaper != self.config.desktop.wallpaper {
            self.backdrop_commit.increment();
            self.desktop.needs_redraw = true;
        }
        if old.desktop.outputs != self.config.desktop.outputs {
            self.outputs_changed(&old.desktop.outputs);
            self.apply_output_config();
        }
        // What used to wait for the next start of the session (the
        // plan's principle 7: a change takes effect at once where it can).
        let (was, now) = (
            &old.desktop.platform.wayland,
            self.config.desktop.platform.wayland.clone(),
        );
        // The displays' scale, where it is set for the system and not
        // for a display of its own.
        if was.output_scale != now.output_scale || was.output_scales != now.output_scales {
            self.apply_output_config();
        }
        // The pointer's theme and size: gevrek's own pointer at once, and
        // the programs started from now on are told.
        if was.cursor_theme != now.cursor_theme || was.cursor_size != now.cursor_size {
            self.cursors = crate::cursor::Cursors::new(&now.cursor_theme, now.cursor_size);
            if now.cursor_theme.is_empty() {
                std::env::remove_var("XCURSOR_THEME");
            } else {
                std::env::set_var("XCURSOR_THEME", &now.cursor_theme);
            }
            if now.cursor_size == 0 {
                std::env::remove_var("XCURSOR_SIZE");
            } else {
                std::env::set_var("XCURSOR_SIZE", now.cursor_size.to_string());
            }
            if self.session_mode {
                crate::session_env::export(&self.config.desktop.session);
            }
            self.desktop.needs_redraw = true;
            log::info!(
                "The pointer's theme is '{}' at {} pixels (programs that run already keep theirs)",
                now.cursor_theme,
                now.cursor_size
            );
        }
        // The session's variables: for every program started from now
        // on (one that runs already keeps what it was started with).
        if old.desktop.session.environment != self.config.desktop.session.environment {
            for name in old.desktop.session.environment.keys() {
                if !self.config.desktop.session.environment.contains_key(name) {
                    std::env::remove_var(name);
                }
            }
            crate::session_env::set_own(&self.config.desktop.session);
            if self.session_mode {
                crate::session_env::export(&self.config.desktop.session);
            }
            log::info!("The session's variables are as the configuration says now");
        }
        // X11 programs: one of the few settings that wait for the next
        // start of the session (ending Xwayland would end every X11
        // program that runs, and it is started with the session). Said,
        // so that nobody waits for it.
        if was.xwayland != now.xwayland {
            log::warn!(
                "desktop.platform.wayland.xwayland is {} now: that holds from the next start of the session",
                if now.xwayland { "on" } else { "off" }
            );
        }
        let diff = ConfigDiff::between(&old, &self.config);
        if diff.unchanged {
            log::info!("Configuration reload: nothing changed");
            self.emit_reloaded();
            return;
        }
        log::info!("Applying configuration reload as a diff");
        let desktop = self.config.desktop.clone();
        let mut borders_before = None;
        let mut windows_changed_workspace = false;
        // As in the Windows host: the workspaces the previous configuration
        // made and the new one does not.
        let mut leftovers: Vec<(gevrek_engine::outputs::MonitorHandle, String)> = Vec::new();
        if !old.desktop.workspaces.iter().any(|profile| profile.enabled) {
            for monitor in self.core.monitor_manager.get_all_monitors() {
                if let Some(workspace) = self
                    .core
                    .monitor_manager
                    .get_workspaces(monitor.handle)
                    .and_then(|ring| ring.focused())
                {
                    if !self
                        .core
                        .executor
                        .graph_creates_workspace(monitor.handle, &workspace.name)
                    {
                        leftovers.push((monitor.handle, workspace.name.clone()));
                    }
                }
            }
        }

        // The order of the steps is the engine's (`ConfigDiff::steps`), the
        // same for every backend; here each is carried out on Wayland.
        for step in diff.steps() {
            match step {
                ReloadStep::Watcher => {
                    if let Some(watcher) = self.host.as_mut().and_then(|host| host.watcher.as_mut())
                    {
                        watcher.set_enabled(desktop.watch_configuration);
                    }
                }
                ReloadStep::Startup => self.startup.configure(&desktop.startup, true),
                ReloadStep::Animations => self
                    .desktop
                    .animations
                    .set_profile(desktop.animations.clone()),
                ReloadStep::Focus => self.core.focus_manager.set_config(desktop.focus.clone()),
                ReloadStep::Monitors => {
                    gevrek_engine::config_apply::configure_monitor_profiles(
                        &mut self.core.monitor_manager,
                        &desktop,
                    );
                    self.core
                        .monitor_manager
                        .set_bar(Some(desktop.shell.panel.clone()));
                    // (As at the start: gevrek-shell's panel asks for its
                    // own room, so none is held here in case a bar is
                    // about to come. Held, it was room twice for twenty
                    // seconds after every change of the panel's settings,
                    // and the windows gave way and came back.)
                    self.core.monitor_manager.expect_no_bar();
                }
                ReloadStep::SnapZones => {
                    self.core.snap_zone_manager = gevrek_engine::config_apply::build_snap_zone_manager(
                        &desktop.snap_zones,
                        &self.core.monitor_manager,
                        |_| false,
                    );
                }
                ReloadStep::Rules => {
                    *self
                        .rules
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        gevrek_engine::config_apply::build_rule_engine(&desktop);
                }
                ReloadStep::Scratchpads => {
                    self.core.executor.release_removed_scratchpads(
                        &desktop.scratchpads,
                        &mut self.desktop,
                        &mut self.core.monitor_manager,
                    );
                    self.core
                        .executor
                        .set_scratchpad_profiles(desktop.scratchpads.clone());
                }
                ReloadStep::StoreProfiles => self
                    .core
                    .executor
                    .set_workspace_profiles(desktop.workspaces.clone(), desktop.appearance.clone()),
                ReloadStep::PruneWindows => {
                    self.core
                        .monitor_manager
                        .cleanup_stale_windows(&self.desktop);
                }
                ReloadStep::SnapshotBorders => {
                    borders_before = Some(plan::border_snapshot(&self.core.monitor_manager));
                }
                ReloadStep::Graph => {
                    self.core.executor.set_graph(self.config.node_graph.clone());
                    self.core.executor.clear_state();
                    match self
                        .core
                        .executor
                        .rerun_startup_for_reload(&mut self.desktop, &mut self.core.monitor_manager)
                    {
                        Ok(dropped) => {
                            leftovers.extend(dropped.iter().cloned());
                            for dropped in self.core.remove_dropped_graph_workspaces(dropped) {
                                if dropped.removed {
                                    self.emit(
                                        RuntimeEventKind::WorkspaceDestroyed,
                                        None,
                                        vec![
                                            ("monitor".to_string(), dropped.output.0.to_string()),
                                            ("name".to_string(), dropped.name),
                                            (
                                                "reason".to_string(),
                                                "removed from the graph".to_string(),
                                            ),
                                        ],
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            log::error!("Startup automation failed during reload: {error}")
                        }
                    }
                }
                ReloadStep::DynamicRules => {
                    if let Err(error) = self.core.executor.reevaluate_all_dynamic_rules(
                        &mut self.desktop,
                        &mut self.core.monitor_manager,
                    ) {
                        log::error!("Dynamic rule re-evaluation failed during reload: {error}");
                    }
                }
                ReloadStep::Bindings => self.register_bindings(),
                ReloadStep::Appearance => self.apply_appearance(),
                // On reload a profile's `default` flag does not switch the
                // visible workspace.
                ReloadStep::ReapplyProfiles => {
                    self.apply_workspace_profiles(
                        |profile| diff.reapplies_profile(&profile.name),
                        false,
                    );
                    windows_changed_workspace |= !self
                        .core
                        .hand_over_leftover_workspaces(
                            &mut self.desktop,
                            &desktop.workspaces,
                            &leftovers,
                        )
                        .is_empty();
                }
                ReloadStep::ReleaseIgnored => {
                    self.core.release_newly_ignored_windows(&mut self.desktop)
                }
                ReloadStep::RefreshBorders => {
                    if let Some(before) = &borders_before {
                        self.core.refresh_changed_borders(
                            &mut self.desktop,
                            before,
                            diff.appearance || windows_changed_workspace,
                        );
                    }
                }
                ReloadStep::MarkLayouts => {
                    for monitor in self.core.monitor_manager.get_all_monitors() {
                        self.core.executor.mark_layout_dirty(monitor.handle);
                    }
                }
                ReloadStep::ReassertScratchpads => self
                    .core
                    .executor
                    .reassert_scratchpads(&mut self.desktop, &mut self.core.monitor_manager),
                // `settle`, below, applies the focus effects.
                ReloadStep::FocusEffects => {}
            }
        }
        self.settle();
        self.emit_reloaded();
    }

    fn emit_reloaded(&self) {
        self.emit(
            RuntimeEventKind::ConfigurationReloaded,
            None,
            vec![
                ("config_version".to_string(), self.config.version.clone()),
                (
                    "schema_version".to_string(),
                    self.config.desktop.schema_version.to_string(),
                ),
            ],
        );
    }

    /// The appearance of the configuration, on every workspace and window.
    pub fn apply_appearance(&mut self) {
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
        let ids: Vec<WindowHandle> = self.desktop.windows.keys().copied().collect();
        for id in ids {
            self.core
                .apply_appearance_to_window(&mut self.desktop, &self.config.desktop, id);
        }
        self.core
            .apply_focus_effects(&mut self.desktop, &self.config.desktop);
    }

    /// Apply the workspace profiles selected by `include`, and start the
    /// program of a profile whose workspace was created empty.
    pub fn apply_workspace_profiles(
        &mut self,
        include: impl Fn(&gevrek_model::settings::WorkspaceProfile) -> bool,
        focus_default: bool,
    ) {
        let created = self.core.apply_workspace_profile_set(
            &self.config.desktop.workspaces,
            &self.config.desktop.appearance,
            include,
            focus_default,
        );
        for workspace in created {
            self.emit(
                RuntimeEventKind::WorkspaceCreated,
                None,
                vec![
                    ("monitor".to_string(), workspace.monitor.0.to_string()),
                    ("name".to_string(), workspace.profile.name.clone()),
                    (
                        "persistent".to_string(),
                        workspace.profile.persistent.to_string(),
                    ),
                ],
            );
            let Some(launch) = &workspace.profile.on_created_empty else {
                continue;
            };
            let key = (
                workspace.monitor,
                workspace.profile.name.trim().to_ascii_lowercase(),
            );
            if !self.workspace_hooks_fired.insert(key) {
                continue;
            }
            match crate::launch::spawn(launch) {
                Ok(pid) => log::info!(
                    "Workspace {} created empty; launched process {pid}",
                    workspace.profile.name
                ),
                Err(error) => log::error!(
                    "Workspace {} on-created-empty launch failed: {error}",
                    workspace.profile.name
                ),
            }
        }
    }

    /// Carry out a command from a binding, the CLI, gRPC or the editor.
    /// Some commands are the host's own (binding submaps, startup programs,
    /// toggles mirrored into the configuration); the rest go to the core.
    pub fn run_command(&mut self, command: DispatchCommand) -> anyhow::Result<()> {
        let name = command.display_name().to_string();
        let payload = serde_json::to_string(&command).unwrap_or_else(|_| name.clone());
        let result = self.run_command_inner(command);
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
        self.emit(kind, None, fields);
        result
    }

    fn run_command_inner(&mut self, command: DispatchCommand) -> anyhow::Result<()> {
        match command {
            DispatchCommand::SetBindingSubmap { name } => {
                let name = name.trim().to_ascii_lowercase();
                if name.is_empty() {
                    anyhow::bail!("Binding submap name cannot be empty");
                }
                if name != "default"
                    && !self
                        .config
                        .desktop
                        .bindings
                        .iter()
                        .any(|binding| binding.enabled && binding.normalized_submap() == name)
                {
                    anyhow::bail!("No enabled binding submap named '{name}' exists");
                }
                if self.active_submap != name {
                    self.active_submap = name;
                    self.register_bindings();
                }
                Ok(())
            }
            DispatchCommand::ResetBindingSubmap => {
                if self.active_submap != "default" {
                    self.active_submap = "default".to_string();
                    self.register_bindings();
                }
                Ok(())
            }
            DispatchCommand::Quit => {
                log::info!("Quit: the session ends");
                // (By the way every stop goes: a session goes under its
                // veil first, and is not cut off.)
                match &self.host {
                    Some(host) => {
                        *host
                            .links
                            .stop_flag
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
                    }
                    None => self.running = false,
                }
                Ok(())
            }
            DispatchCommand::KeyboardLayout { target } => self.switch_keyboard_layout(target),
            DispatchCommand::ExpectWindow { output } => {
                self.expected_window = Some((output, std::time::Instant::now()));
                Ok(())
            }
            DispatchCommand::WorkspaceOverview => self.toggle_workspace_overview(),
            DispatchCommand::PickWindow => {
                self.toggle_pick();
                Ok(())
            }
            DispatchCommand::OutputTrial { answer } => {
                if self.answer_output_trial(answer) {
                    Ok(())
                } else {
                    anyhow::bail!("No display settings are on trial")
                }
            }
            DispatchCommand::ShortcutsInhibit => {
                if self.toggle_shortcuts_inhibit() {
                    Ok(())
                } else {
                    anyhow::bail!("No program has asked for every key")
                }
            }
            DispatchCommand::Shell { action } => {
                self.emit(
                    RuntimeEventKind::ShellCommand,
                    None,
                    vec![("action".to_string(), action.id())],
                );
                // Locking is not left to whether a shell is listening.
                if action == gevrek_model::command::ShellAction::Lock {
                    self.start_locker();
                }
                // Nor is switching the displays off: it is the compositor's.
                if action == gevrek_model::command::ShellAction::DisplaysOff {
                    self.switch_displays_off();
                }
                // The night light is the compositor's too: its key
                // switches it on or off (within its hours, when the
                // configuration gives it hours), and the configuration
                // says so afterwards.
                if action == gevrek_model::command::ShellAction::NightLight {
                    let on = !self.config.desktop.night_light.enabled;
                    self.config.desktop.night_light.enabled = on;
                    self.config_dirty = true;
                    self.desktop.needs_redraw = true;
                    log::info!(
                        "The night light is switched {}",
                        if on { "on" } else { "off" }
                    );
                    // Said on the screen: outside its hours nothing
                    // turns warm yet, and that must not look like a key
                    // that does nothing.
                    let now = {
                        let now = jiff::Zoned::now();
                        now.hour() as u32 * 60 + now.minute() as u32
                    };
                    let said = DispatchCommand::Osd {
                        text: self.config.desktop.night_light.said_when_switched(now),
                        level: None,
                    };
                    self.emit(
                        RuntimeEventKind::ShellCommand,
                        None,
                        said.shell_fields().unwrap_or_default(),
                    );
                }
                Ok(())
            }
            DispatchCommand::NightLightPreview {
                temperature,
                seconds,
            } => {
                self.night_preview = Some((
                    gevrek_model::night_light::within(temperature),
                    std::time::Instant::now()
                        + std::time::Duration::from_secs(
                            gevrek_model::night_light::preview_seconds(seconds),
                        ),
                ));
                self.desktop.needs_redraw = true;
                Ok(())
            }
            // What the shell shows for someone else: told as its keys are.
            command @ (DispatchCommand::Notify { .. }
            | DispatchCommand::Osd { .. }
            | DispatchCommand::AppShortcut { .. }
            | DispatchCommand::Zones { .. }) => {
                self.emit(
                    RuntimeEventKind::ShellCommand,
                    None,
                    command.shell_fields().unwrap_or_default(),
                );
                Ok(())
            }
            DispatchCommand::ShowShortcuts => gevrek_host::shortcuts::toggle(),
            DispatchCommand::Exec { program, args } => {
                let launch = gevrek_model::settings::LaunchProfile {
                    program,
                    args,
                    ..Default::default()
                };
                gevrek_engine::Desktop::launch(&mut self.desktop, &launch).map(|pid| {
                    log::info!("Started '{}' as PID {pid}", launch.program);
                })
            }
            DispatchCommand::LaunchStartup { id } => self.startup.launch(&id),
            DispatchCommand::StopStartup { id } => self.startup.stop(&id),
            DispatchCommand::RestartStartup { id } => self.startup.restart(&id),
            DispatchCommand::ToggleMouseFollowsFocus => {
                let enabled = !self.core.focus_manager.mouse_follows_focus();
                self.core.focus_manager.set_mouse_follows_focus(enabled);
                // Mirrored into the configuration so a later reload does
                // not revert it.
                self.config.desktop.focus.mouse_follows_focus = enabled;
                self.config_dirty = true;
                Ok(())
            }
            DispatchCommand::WatchConfiguration { enabled } => {
                if let Some(watcher) = self.host.as_mut().and_then(|host| host.watcher.as_mut()) {
                    watcher.set_enabled(enabled);
                }
                self.config.desktop.watch_configuration = enabled;
                self.config_dirty = true;
                Ok(())
            }
            DispatchCommand::ToggleStackbar => {
                let enabled = !self.config.desktop.appearance.stackbar.enabled;
                self.config.desktop.appearance.stackbar.enabled = enabled;
                self.config_dirty = true;
                self.apply_appearance();
                for monitor in self.core.monitor_manager.get_all_monitors_ordered() {
                    self.core.executor.mark_layout_dirty(monitor.handle);
                }
                Ok(())
            }
            DispatchCommand::ToggleTransparency => {
                let enabled = !self.config.desktop.appearance.transparency_enabled;
                self.config.desktop.appearance.transparency_enabled = enabled;
                self.config_dirty = true;
                Ok(())
            }
            DispatchCommand::SetTransparencyAlpha { alpha } => {
                self.config.desktop.appearance.inactive_window_opacity = alpha.max(20);
                self.config_dirty = true;
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
                self.config.desktop.work_area_offset = offset;
                self.config_dirty = true;
                self.core
                    .monitor_manager
                    .set_global_work_area_offset(offset);
                for monitor in self.core.monitor_manager.get_all_monitors_ordered() {
                    self.core.executor.mark_layout_dirty(monitor.handle);
                }
                Ok(())
            }
            command => {
                self.core.executor.clear_state();
                self.core.dispatch(&mut self.desktop, command)
            }
        }
    }

    /// The bindings that are live now: the key nodes of the graph and the
    /// bindings of the active submap. Ids follow the Windows host: negative
    /// for graph nodes, from 1,000,000 for configured bindings.
    pub fn register_bindings(&mut self) {
        if self.active_submap != "default"
            && !self
                .config
                .desktop
                .bindings
                .iter()
                .any(|binding| binding.enabled && binding.normalized_submap() == self.active_submap)
        {
            log::warn!(
                "Binding submap '{}' no longer exists; returning to default",
                self.active_submap
            );
            self.active_submap = "default".to_string();
        }
        let mut bindings = Vec::new();
        for (index, (_, node)) in self.core.executor.get_graph().nodes.iter().enumerate() {
            if let NodeType::OnKeyPress {
                key,
                ctrl,
                alt,
                shift,
                win,
            } = &node.node_type
            {
                if !key.is_empty() {
                    let id = (-1i32).saturating_sub(index as i32);
                    bindings.push((id, Hotkey::new(key.clone(), *ctrl, *alt, *shift, *win)));
                }
            }
        }
        for (index, binding) in self.config.desktop.bindings.iter().enumerate() {
            if !binding.enabled
                || binding.key.trim().is_empty()
                || binding.normalized_submap() != self.active_submap
            {
                continue;
            }
            bindings.push((
                1_000_000i32.saturating_add(index as i32),
                Hotkey {
                    key: binding.key.clone(),
                    ctrl: binding.ctrl,
                    alt: binding.alt,
                    shift: binding.shift,
                    win: binding.win,
                    device: binding.device,
                    trigger: binding.trigger,
                    consume: binding.consume,
                    long_press_ms: binding.long_press_ms,
                },
            ));
        }
        log::info!(
            "{} binding(s) live in submap '{}'",
            bindings.len(),
            self.active_submap
        );
        self.bindings = bindings;
    }

    /// The bindings a key or button event hits: their ids, and whether one
    /// of them keeps the event from the focused client.
    pub fn matching_bindings(
        &self,
        device: BindingDevice,
        key: &str,
        modifiers: (bool, bool, bool, bool),
        trigger: BindingTrigger,
    ) -> (Vec<i32>, bool) {
        if self.paused {
            return (Vec::new(), false);
        }
        let (ctrl, alt, shift, logo) = modifiers;
        let mut ids = Vec::new();
        let mut consume = false;
        for (id, hotkey) in &self.bindings {
            let hit = hotkey.device == device
                && crate::keys::canonical(&hotkey.key) == key
                && hotkey.ctrl == ctrl
                && hotkey.alt == alt
                && hotkey.shift == shift
                && hotkey.win == logo;
            if !hit {
                continue;
            }
            // Whatever the trigger, a consuming binding keeps the whole key
            // stroke from the client.
            consume |= hotkey.consume;
            if hotkey.trigger == trigger {
                ids.push(*id);
            }
        }
        (ids, consume)
    }

    /// Run the bindings a key stroke hit.
    pub fn bindings_triggered(&mut self, ids: Vec<i32>, event: HotkeyEvent) {
        log::info!(
            "Input triggered: device={:?}, trigger={:?}, key={}, ctrl={}, alt={}, shift={}, win={}",
            event.device,
            event.trigger,
            event.key,
            event.ctrl,
            event.alt,
            event.shift,
            event.win
        );
        self.emit(
            RuntimeEventKind::BindingTriggered,
            None,
            vec![
                ("device".to_string(), format!("{:?}", event.device)),
                ("trigger".to_string(), format!("{:?}", event.trigger)),
                ("key".to_string(), event.key.clone()),
            ],
        );
        let commands: Vec<DispatchCommand> = ids
            .iter()
            .filter_map(|id| {
                let index = id.checked_sub(1_000_000)? as usize;
                self.config
                    .desktop
                    .bindings
                    .get(index)
                    .map(|binding| binding.command.clone())
            })
            .collect();
        for command in commands {
            if let Err(error) = self.run_command(command) {
                log::error!("Binding failed: {error}");
            }
        }
        if ids.iter().any(|id| *id < 0) {
            if let Err(error) = self.core.executor.execute_hotkey(
                &mut self.desktop,
                &mut self.core.monitor_manager,
                &mut self.core.focus_manager,
                &event,
            ) {
                log::error!("Key node failed: {error}");
            }
            // Focus cycling asked for by a node.
            if let Some(operation) = self.core.executor.take_pending_focus_op() {
                let target = match operation {
                    PendingFocusOp::CycleNext => self.core.focus_manager.cycle_next(),
                    PendingFocusOp::CyclePrevious => self.core.focus_manager.cycle_previous(),
                };
                if let Some(id) = target {
                    if let Err(error) = self.desktop.apply(&Effect::Focus(Some(id))) {
                        log::error!("focus {id:?}: {error}");
                    }
                }
            }
            self.settle();
        }
    }

    /// Publish the state snapshot the server hands to the CLI, the editor
    /// and the overlay.
    pub fn publish_overlay(&mut self, last_event: Option<String>) {
        if self.host.is_none() {
            return;
        }
        let (mut monitors, total_windows) =
            self.core.monitor_data(&self.desktop, &self.config.desktop);
        // The room the shell's zones keep is asked of the compositor by
        // surfaces of their own (`protocols.rs`,
        // `zones_room`): how much of a display's kept room is theirs.
        for monitor in &mut monitors {
            monitor.zones_room = self.zones_room(monitor.handle);
        }
        for (kind, fields) in
            gevrek_engine::overlay_data::changes(&self.published_monitors, &monitors)
        {
            self.emit(kind, None, fields);
        }
        self.published_monitors = monitors.clone();
        let Some(host) = &mut self.host else {
            return;
        };
        host.last_overlay = std::time::Instant::now();
        let data = OverlayData {
            timestamp: std::time::SystemTime::now(),
            monitors,
            graph_node_count: self.config.node_graph.nodes.len(),
            total_windows,
            event_loop_fps: self.host_pace.passes_per_second,
            last_event,
            active_animations: self.desktop.animations.active_count(),
            active_binding_submap: self.active_submap.clone(),
            tiling_suspended: self.core.executor.tiling_suspended(),
            gesture: GestureStateData {
                available: self.gestures_available,
                backend: if self.gestures_available {
                    "libinput"
                } else {
                    "none"
                }
                .to_string(),
                device_count: 0,
                configured_count: self
                    .config
                    .desktop
                    .gestures
                    .profiles
                    .iter()
                    .filter(|profile| self.config.desktop.gestures.enabled && profile.enabled)
                    .count() as u32,
                active: self.gesture_progress.clone(),
                last_error: None,
            },
            rule_states: self
                .rules
                .lock()
                .map(|engine| {
                    engine
                        .rule_states()
                        .into_iter()
                        .map(|(name, enabled, dynamic)| RuleStateData {
                            name,
                            enabled,
                            dynamic,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            scratchpad_states: self.core.scratchpad_data(),
            startup_states: self
                .startup
                .states()
                .into_iter()
                .map(|state| StartupStateData {
                    id: state.id,
                    name: state.name,
                    enabled: state.enabled,
                    status: state.status,
                    pid: state.pid,
                    restart_count: state.restart_count,
                    last_exit_code: state.last_exit_code,
                    last_error: state.last_error,
                })
                .collect(),
            metrics: None,
            notices: {
                let connected: Vec<&str> = self
                    .desktop
                    .outputs
                    .iter()
                    .map(|output| output.name.as_str())
                    .collect();
                let mut notices =
                    crate::notices::of_outputs(&self.config.desktop.outputs, &connected, |name| {
                        self.vrr_on.contains(name)
                    });
                notices.extend(crate::notices::of_night_light(
                    &self.config.desktop.night_light,
                    self.night_light_impossible,
                ));
                notices
            },
            accent: self.core.wallpaper_accent,
            keyboard_layout: self.keyboard_layout.clone(),
            frames: self
                .desktop
                .outputs
                .iter()
                .filter_map(|output| {
                    let (direct, composed, direct_now) = *self.frames_shown.get(&output.id)?;
                    Some(gevrek_engine::overlay_data::DisplayFrames {
                        output: output.name.clone(),
                        direct,
                        composed,
                        direct_now,
                    })
                })
                .collect(),
        };
        let notices: Vec<String> = data
            .notices
            .iter()
            .map(|notice| notice.id.clone())
            .collect();
        *host
            .links
            .overlay_data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(data);
        // Whoever shows the notices (the panel) hears that they are others
        // now and asks for the state, instead of at its next question.
        if notices != self.notices_told {
            let count = notices.len();
            self.notices_told = notices;
            self.emit(
                RuntimeEventKind::NoticesChanged,
                None,
                vec![("count".to_string(), count.to_string())],
            );
        }
    }

    /// A picture of the next frame is asked for by creating the file
    /// `$GEVREK_WL_SCREENSHOT.request`; the picture is written to
    /// `$GEVREK_WL_SCREENSHOT` and the request file is removed.
    fn poll_screenshot_request(&mut self) {
        let Some(path) = std::env::var_os("GEVREK_WL_SCREENSHOT").map(std::path::PathBuf::from)
        else {
            return;
        };
        let mut request = path.clone().into_os_string();
        request.push(".request");
        if std::fs::remove_file(&request).is_ok() {
            self.screenshot = Some(path);
            self.desktop.needs_redraw = true;
        }
    }

    /// Tell the subscribers of the event stream what happened.
    pub fn emit(
        &self,
        kind: RuntimeEventKind,
        window: Option<WindowHandle>,
        fields: Vec<(String, String)>,
    ) {
        let Some(broadcast) = &self.node_events else {
            return;
        };
        broadcast(NodeExecutionEvent::Runtime(RuntimeExtensionEvent {
            kind,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            window_handle: window,
            fields: fields.into_iter().collect(),
        }));
    }

    /// Stop the programs gevrek started and say goodbye.
    pub fn shutdown(&mut self) {
        self.startup.shutdown();
        self.emit(RuntimeEventKind::RuntimeStopping, None, Vec::new());
        self.forget_window_icons();
        log::info!("gevrek stopped cleanly");
    }
}
