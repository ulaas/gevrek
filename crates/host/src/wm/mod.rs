//! The window manager's host, once for every system.
//!
//! Around the engine's core (`gevrek_engine::core::WmCore`) every backend
//! runs the same host: it takes the configuration the server is asked for
//! and the edits of the file, applies a new configuration as a diff, runs
//! the commands of the server, the bindings and the editor, keeps the
//! programs started for the user, and publishes the state and the events
//! the clients read. That host was written twice, once in the Windows
//! backend and once in the Wayland one, and the two had begun to differ.
//! It is written here once.
//!
//! A backend holds a [`HostState`] (what only the host owns) beside its
//! configuration, its core and its desktop, and implements [`WmHost`]: it
//! lends those four out together ([`WmHost::parts`]) and answers the few
//! questions only its system can (the hooks of the trait: what follows a
//! command, how keys are taken, what a reload changes of the system's own
//! settings). The rest is the trait's own methods:
//!
//! - [`configuration`]: requests from the server, edits of the file, and
//!   publishing the configuration that runs.
//! - [`commands`]: the server's queue, and every command, the host's own
//!   and the core's.
//! - [`reload`]: a new configuration as a diff against the running one,
//!   and the workspace profiles.
//! - [`bindings`]: which keys are live, by which id, and what a key that
//!   hit them does.
//! - [`published`]: the state the clients read.
//!
//! What a backend cannot do is said by a hook's default, not left out:
//! a system that has no picker of windows answers `pick_window` with an
//! error that says so.

pub mod bindings;
pub mod commands;
pub mod configuration;
pub mod published;
pub mod reload;
#[cfg(test)]
mod tests;

use crate::command::ManualCommand;
use crate::config_store::{ConfigPublisher, ConfigRequest};
use crate::config_watch::ConfigFileWatcher;
use crate::process_manager::StartupProcessManager;
use gevrek_engine::core::WmCore;
use gevrek_engine::executor::{
    NodeEventBroadcast, NodeExecutionEvent, RuntimeEventKind, RuntimeExtensionEvent,
};
use gevrek_engine::overlay_data::{MonitorData, OverlayData};
use gevrek_engine::Desktop;
use gevrek_model::config::Config;
use gevrek_model::ids::{OutputId, WindowId};
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// What the host owns and no system does. Each backend keeps one.
pub struct HostState {
    /// Configurations the server was asked to run.
    pub config_receiver: Option<Receiver<ConfigRequest>>,
    /// Commands of the server (the CLI, the editor, the shell).
    pub command_receiver: Option<Receiver<ManualCommand>>,
    /// Who reads the running configuration (the server).
    pub config_publisher: Option<ConfigPublisher>,
    /// The configuration file, watched for edits made by hand.
    pub watcher: Option<ConfigFileWatcher>,
    /// Where the state is published for the server to hand out.
    pub overlay: Option<Arc<Mutex<Option<OverlayData>>>>,
    pub pause_flag: Option<Arc<Mutex<bool>>>,
    pub stop_flag: Option<Arc<Mutex<bool>>>,
    /// The stream of events subscribers hear.
    pub events: Option<NodeEventBroadcast>,
    /// The configuration changed here (a toggle) since it was last
    /// published.
    pub config_dirty: bool,
    /// The named set of bindings that is live (`default` unless a command
    /// switched to another).
    pub active_submap: String,
    /// The programs started for the user (`desktop.startup`).
    pub startup: StartupProcessManager,
    /// Workspaces whose `on_created_empty` program was started already: a
    /// reload builds workspaces anew and must not start it again.
    pub workspace_hooks_fired: HashSet<(OutputId, String)>,
    /// The outputs as last published, to tell subscribers what changed.
    pub published_monitors: Vec<MonitorData>,
    /// The notices last told of, to tell when they are others.
    pub notices_told: Vec<String>,
    /// Something was told, or done, since the state was last published:
    /// it is published again at the end of the pass (who hears of a change
    /// asks for the state at once, and is to get the state after it).
    pub state_stale: Cell<bool>,
    /// When the state was last published.
    pub last_published: std::time::Instant,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            config_receiver: None,
            command_receiver: None,
            config_publisher: None,
            watcher: None,
            overlay: None,
            pause_flag: None,
            stop_flag: None,
            events: None,
            config_dirty: false,
            active_submap: "default".to_string(),
            startup: StartupProcessManager::new(),
            workspace_hooks_fired: HashSet::new(),
            published_monitors: Vec::new(),
            notices_told: Vec::new(),
            state_stale: Cell::new(false),
            last_published: std::time::Instant::now(),
        }
    }
}

impl HostState {
    /// Whether the host is connected to a server. (A compositor tried
    /// without one, in tests, has none.)
    pub fn attached(&self) -> bool {
        self.command_receiver.is_some() || self.config_receiver.is_some()
    }

    pub fn paused(&self) -> bool {
        self.pause_flag.as_ref().is_some_and(flag)
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_flag.as_ref().is_some_and(flag)
    }

    /// Ask the loop to stop at its next pass. False where there is nobody
    /// to ask (no server): the backend stops by its own means.
    pub fn request_stop(&self) -> bool {
        let Some(stop) = &self.stop_flag else {
            return false;
        };
        *stop.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        true
    }

    /// Watch the configuration file at `path`, as `desktop.watch_configuration`
    /// says.
    pub fn watch_configuration_file(&mut self, path: std::path::PathBuf, enabled: bool) {
        self.watcher = Some(ConfigFileWatcher::new(path, enabled));
        log::info!(
            "Configuration file watch {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// Hand the running configuration to its readers from now on, at once.
    pub fn publish_to(&mut self, publisher: ConfigPublisher, config: &Config) {
        publisher.publish(config);
        self.config_publisher = Some(publisher);
        self.config_dirty = false;
    }

    /// Tell the subscribers of the event stream what happened. Anything
    /// but a window moved or resized (told many times a second while it is
    /// dragged) makes the published state stale.
    pub fn emit(
        &self,
        kind: RuntimeEventKind,
        window: Option<WindowId>,
        fields: impl IntoIterator<Item = (String, String)>,
    ) {
        if !matches!(
            kind,
            RuntimeEventKind::WindowMoved | RuntimeEventKind::WindowResized
        ) {
            self.state_stale.set(true);
        }
        let Some(broadcast) = &self.events else {
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
}

/// What connects the WM loop to the IPC server, handed over at once.
pub struct HostLinks {
    pub config_receiver: Receiver<ConfigRequest>,
    pub command_receiver: Receiver<ManualCommand>,
    /// It already holds the configuration the WM started with.
    pub config_publisher: ConfigPublisher,
    /// The configuration file, watched for edits made by hand.
    pub config_path: Option<std::path::PathBuf>,
    pub overlay_data: Arc<Mutex<Option<OverlayData>>>,
    pub pause_flag: Arc<Mutex<bool>>,
    pub stop_flag: Arc<Mutex<bool>>,
}

impl HostState {
    /// Connect to the IPC server. The file is watched as
    /// `desktop.watch_configuration` says (`watch`).
    pub fn attach(&mut self, links: HostLinks, watch: bool) {
        self.watcher = links
            .config_path
            .map(|path| ConfigFileWatcher::new(path, watch));
        self.config_receiver = Some(links.config_receiver);
        self.command_receiver = Some(links.command_receiver);
        self.config_publisher = Some(links.config_publisher);
        self.overlay = Some(links.overlay_data);
        self.pause_flag = Some(links.pause_flag);
        self.stop_flag = Some(links.stop_flag);
        self.last_published = std::time::Instant::now();
    }
}

fn flag(flag: &Arc<Mutex<bool>>) -> bool {
    *flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a backend lends the host at once: borrowed together, so that the
/// host can hand the desktop to the core while it reads the configuration.
pub struct Parts<'a, D: Desktop> {
    pub config: &'a mut Config,
    pub core: &'a mut WmCore,
    pub desktop: &'a mut D,
    pub host: &'a mut HostState,
}

/// A backend's side of the host. The required methods lend its parts and
/// answer what only its system knows; the provided ones are the host.
pub trait WmHost {
    /// The backend's desktop (what carries out the core's effects).
    type Desktop: Desktop;

    /// Everything the host works with, borrowed at once.
    fn parts(&mut self) -> Parts<'_, Self::Desktop>;
    fn host(&self) -> &HostState;
    fn config(&self) -> &Config;

    /// The host did something (a command, a configuration): a backend that
    /// paces its loop by how busy it is hears of it. Nothing by default.
    fn noted_work(&mut self) {}

    // --- after the core ran ------------------------------------------------

    /// After a command: lay out what it marked, and let the focus effects
    /// (opacity, borders) follow. A compositor also feeds back what its
    /// desktop did in answer.
    fn settle(&mut self);

    // --- what the host's commands need of the system -----------------------

    /// Take these hotkeys, and no others, from now on (ids as
    /// `bindings` gives them out).
    fn install_bindings(
        &mut self,
        bindings: Vec<(i32, gevrek_engine::input::Hotkey)>,
    ) -> anyhow::Result<()>;

    /// The configuration's appearance, on every workspace and window.
    fn apply_appearance(&mut self);

    /// `quit` where there is no server to stop the loop through.
    fn stop_without_server(&mut self) {}

    // --- what a reload needs of the system ---------------------------------

    /// What only this system has changed between `old` and the running
    /// configuration (its displays' modes, its pointer, its session):
    /// applied before the steps every system shares. Nothing by default.
    fn reload_system(&mut self, _old: &Config) {}
    /// How windows move, as the configuration says now.
    fn set_animations(&mut self, animations: &gevrek_model::settings::AnimationProfile);
    /// The monitor profiles and the work-area offset, as the configuration
    /// says now (`config_apply::configure_monitor_profiles`, and what the
    /// system keeps room for).
    fn configure_monitors(&mut self);
    /// Whether a snap zone's modifier is held now. None by default.
    fn snap_zone_modifier_probe(&self) -> fn(gevrek_model::snap_zones::SnapZoneModifier) -> bool {
        |_| false
    }
    /// The window rules, shared with the desktop and the executor.
    fn rules(&self) -> &Arc<Mutex<gevrek_engine::rules::RuleEngine>>;
    /// The appearance as this system applies it (the system's own settings
    /// may ask for less movement or more contrast). As it is by default.
    fn system_appearance(
        &self,
        appearance: &gevrek_model::settings::AppearanceProfile,
    ) -> gevrek_model::settings::AppearanceProfile {
        appearance.clone()
    }
    /// Windows that are gone are dropped from the system's own records
    /// (the core drops them from its workspaces after). Nothing by default.
    fn prune_windows(&mut self) {}

    // --- what the published state needs of the system ----------------------

    /// What only the system knows of the state: its loop's rate, its
    /// animations, its touchpad, its notices, its keyboard layout.
    fn system_state(&self) -> published::SystemState;
    /// Add to the outputs as published what only the system knows of them
    /// (the room the shell's zones keep, on Wayland). Nothing by default.
    fn adjust_published_monitors(&self, _monitors: &mut [MonitorData]) {}

    // --- what only some systems can do -------------------------------------
    //
    // Each answers, by default, that this system cannot.

    /// The next window that opens goes to the output named.
    fn expect_window(&mut self, _output: String) -> anyhow::Result<()> {
        not_here("Saying where the next window opens")
    }
    /// Choose a window with the pointer (for a rule).
    fn pick_window(&mut self) -> anyhow::Result<()> {
        not_here("Picking a window")
    }
    /// Show, or put away, the overview of every workspace.
    fn workspace_overview(&mut self) -> anyhow::Result<()> {
        not_here("The overview of every workspace")
    }
    fn keyboard_layout(
        &mut self,
        _target: gevrek_model::command::LayoutTarget,
    ) -> anyhow::Result<()> {
        not_here("Switching the keyboard layout")
    }
    /// Give every key back to gevrek from a program that asked for all of
    /// them, or hand them over again.
    fn shortcuts_inhibit(&mut self) -> anyhow::Result<()> {
        not_here("Programs taking gevrek's keys")
    }
    /// Keep, or give back, display settings on trial. False: none are.
    fn answer_output_trial(&mut self, _answer: gevrek_model::command::TrialAnswer) -> bool {
        false
    }
    /// The list of shortcuts: gevrek's own window of it, by default.
    fn show_shortcuts(&mut self) -> anyhow::Result<()> {
        crate::shortcuts::toggle()
    }
    /// What the system does itself for a shell action, besides telling the
    /// shell (locking, switching the displays off). Nothing by default.
    fn shell_action(&mut self, _action: gevrek_model::command::ShellAction) {}
    /// The night light was switched in the configuration: the system
    /// follows (drawn anew, or looked at with the next pass).
    fn night_light_switched(&mut self) {}
    /// Show a night light's warmth for a few seconds.
    fn preview_night_light(&mut self, _temperature: u32, _seconds: u32) -> anyhow::Result<()> {
        not_here("A preview of the night light")
    }
    /// Feed made-up input through the window manager's own input path.
    fn inject_input(
        &mut self,
        _steps: &[crate::input_injection::InjectedInput],
    ) -> anyhow::Result<()> {
        not_here("Made-up input")
    }
}

/// What a system cannot do, as an answer.
fn not_here(what: &str) -> anyhow::Result<()> {
    anyhow::bail!("{what} is not possible on this system")
}
