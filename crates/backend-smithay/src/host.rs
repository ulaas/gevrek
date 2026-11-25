//! The WM host on Wayland. The host itself (the configuration requests,
//! the commands, the reload, the bindings, the published state) is the one
//! every backend shares, `gevrek_host::wm`; this file runs it pass by pass
//! (`host_tick`) and answers what only the compositor knows (the
//! `WmHost` hooks, at the end: settling after the core ran, what a reload
//! changes of the outputs, the pointer and the session, matching the keys
//! that come from the input devices).

use crate::state::State;
use gevrek_engine::executor::RuntimeEventKind;
use gevrek_engine::WindowHandle;
use gevrek_host::wm::commands::Commands as _;
use gevrek_host::wm::configuration::Configuration as _;
use gevrek_host::wm::published::Published as _;
use gevrek_model::config::Config;
use gevrek_model::settings::{BindingDevice, BindingTrigger};

pub use gevrek_host::wm::HostLinks;

impl State {
    /// Connect the WM loop to the IPC server (the publisher already holds
    /// the configuration the WM started with).
    pub fn attach_host(&mut self, links: HostLinks) {
        self.host
            .attach(links, self.config.desktop.watch_configuration);
        self.publish_state(Some("started".to_string()));
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
        if !self.host.attached() {
            return self.running;
        }
        if self.host.stop_requested() {
            // (A session is not cut off: it goes under its veil first.)
            if self.leaving_before_it_stops() {
                self.tick_lock_change();
                return true;
            }
            log::info!("Stop requested");
            self.running = false;
            return false;
        }

        let had_commands = self.run_queued_commands();

        // Paused: bindings do not run. Configuration changes are still
        // applied and published.
        self.paused = self.host.paused();
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
            self.publish_state(Some("accent".to_string()));
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
                self.host.startup.tick();
            }
            if let Some(program) = self.pending_program.take() {
                // (`--ends-with-program`: gevrek is there for this one
                // program, as the login's greeter is, and ends with it.)
                let stop = self
                    .ends_with_program
                    .then(|| self.host.stop_flag.clone())
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

        let due = self.host.last_published.elapsed() >= std::time::Duration::from_millis(250);
        // A floating window its program moved to another display is that
        // display's once it has come to rest (the state that is published
        // next says so).
        if due {
            self.core
                .settle_moved_windows(&mut self.desktop, std::time::Instant::now());
        }
        if had_commands || due || self.host.state_stale.get() {
            self.publish_state(None);
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

    /// Apply `new_config` as a diff against the running configuration:
    /// only what changed is touched, and windows stay where they are.
    fn reload_wayland(&mut self, old: &Config) {
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
        self.host.emit(kind, window, fields);
    }

    /// Stop the programs gevrek started and say goodbye.
    pub fn shutdown(&mut self) {
        self.host.startup.shutdown();
        self.emit(RuntimeEventKind::RuntimeStopping, None, Vec::new());
        self.forget_window_icons();
        log::info!("gevrek stopped cleanly");
    }
}

/// The compositor is a host of the shared kind (`gevrek_host::wm`): it
/// lends its parts, and says what only Wayland knows.
impl gevrek_host::wm::WmHost for State {
    type Desktop = crate::desktop::WlDesktop;

    fn parts(&mut self) -> gevrek_host::wm::Parts<'_, crate::desktop::WlDesktop> {
        gevrek_host::wm::Parts {
            config: &mut self.config,
            core: &mut self.core,
            desktop: &mut self.desktop,
            host: &mut self.host,
        }
    }

    fn host(&self) -> &gevrek_host::wm::HostState {
        &self.host
    }

    fn config(&self) -> &Config {
        &self.config
    }

    fn noted_work(&mut self) {
        self.host_pace.last_work = Some(std::time::Instant::now());
    }

    fn settle(&mut self) {
        State::settle(self);
    }

    fn install_bindings(
        &mut self,
        bindings: Vec<(i32, gevrek_engine::input::Hotkey)>,
    ) -> anyhow::Result<()> {
        // Matched against as keys come (`matching_bindings`).
        self.bindings = bindings;
        Ok(())
    }

    fn apply_appearance(&mut self) {
        State::apply_appearance(self);
    }

    fn stop_without_server(&mut self) {
        self.running = false;
    }

    fn reload_system(&mut self, old: &Config) {
        self.reload_wayland(old);
    }

    fn set_animations(&mut self, animations: &gevrek_model::settings::AnimationProfile) {
        self.desktop.animations.set_profile(animations.clone());
    }

    fn configure_monitors(&mut self) {
        gevrek_engine::config_apply::configure_monitor_profiles(
            &mut self.core.monitor_manager,
            &self.config.desktop,
        );
        self.core
            .monitor_manager
            .set_panels(&self.config.desktop.shell.panels);
        // (As at the start: gevrek-shell's panel asks for its own room, so
        // none is held here in case a bar is about to come. Held, it was
        // room twice for twenty seconds after every change of the panel's
        // settings, and the windows gave way and came back.)
        self.core.monitor_manager.expect_no_bar();
    }

    fn rules(&self) -> &std::sync::Arc<std::sync::Mutex<gevrek_engine::rules::RuleEngine>> {
        &self.rules
    }

    fn system_state(&self) -> gevrek_host::wm::published::SystemState {
        let notices = {
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
        };
        gevrek_host::wm::published::SystemState {
            event_loop_fps: self.host_pace.passes_per_second,
            active_animations: self.desktop.animations.active_count(),
            gestures_available: self.gestures_available,
            gesture_backend: if self.gestures_available {
                "libinput"
            } else {
                "none"
            }
            .to_string(),
            gesture_devices: 0,
            gesture: self.gesture_progress.clone(),
            gesture_error: None,
            notices,
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
        }
    }

    // The room the shell's zones keep is asked of the compositor by surfaces
    // of their own (`protocols.rs`, `zones_room`): how much of a display's
    // kept room is theirs.
    fn adjust_published_monitors(&self, monitors: &mut [gevrek_engine::overlay_data::MonitorData]) {
        for monitor in monitors {
            monitor.zones_room = self.zones_room(monitor.handle);
        }
    }

    fn expect_window(&mut self, output: String) -> anyhow::Result<()> {
        self.expected_window = Some((output, std::time::Instant::now()));
        Ok(())
    }

    fn pick_window(&mut self) -> anyhow::Result<()> {
        self.toggle_pick();
        Ok(())
    }

    fn workspace_overview(&mut self) -> anyhow::Result<()> {
        self.toggle_workspace_overview()
    }

    fn keyboard_layout(
        &mut self,
        target: gevrek_model::command::LayoutTarget,
    ) -> anyhow::Result<()> {
        self.switch_keyboard_layout(target)
    }

    fn shortcuts_inhibit(&mut self) -> anyhow::Result<()> {
        if self.toggle_shortcuts_inhibit() {
            Ok(())
        } else {
            anyhow::bail!("No program has asked for every key")
        }
    }

    fn answer_output_trial(&mut self, answer: gevrek_model::command::TrialAnswer) -> bool {
        State::answer_output_trial(self, answer)
    }

    fn shell_action(&mut self, action: gevrek_model::command::ShellAction) {
        use gevrek_model::command::ShellAction;
        match action {
            // Locking is not left to whether a shell is listening.
            ShellAction::Lock => self.start_locker(),
            // Nor is switching the displays off: it is the compositor's.
            ShellAction::DisplaysOff => self.switch_displays_off(),
            _ => {}
        }
    }

    fn night_light_switched(&mut self) {
        self.desktop.needs_redraw = true;
    }

    fn preview_night_light(&mut self, temperature: u32, seconds: u32) -> anyhow::Result<()> {
        self.night_preview = Some((
            gevrek_model::night_light::within(temperature),
            std::time::Instant::now()
                + std::time::Duration::from_secs(gevrek_model::night_light::preview_seconds(
                    seconds,
                )),
        ));
        self.desktop.needs_redraw = true;
        Ok(())
    }

    fn inject_input(
        &mut self,
        steps: &[gevrek_host::input_injection::InjectedInput],
    ) -> anyhow::Result<()> {
        State::inject_input(self, steps)
    }
}
