mod command;
use gevrek_engine::event_coalescing;
pub use gevrek_engine::executor;
pub use gevrek_host::config_store;
use gevrek_host::config_watch;
#[cfg(test)]
mod executor_tests;
use crate::dwm::MonitorManagerWin32;
pub use crate::host::process_guard;
use gevrek_engine::core;
use gevrek_host::process_manager;
#[cfg(test)]
mod characterization_tests;
mod held;
mod pass_times;
mod reload;
#[cfg(test)]
mod test_support;
mod wait;

use crate::config::Config;
use crate::dwm::HwndExt;
use crate::dwm::{
    FocusManager, MonitorManager, OverlayData, RuleEngine, WindowManager, WindowMonitor,
    ZoneManager,
};
use crate::hotkey::{GestureEvent, GesturePhase, GestureRegistration, Hotkey, HotkeyManager};
use crate::node_editor::LogEntry;
use crate::persistence::PersistenceManager;
use crate::settings::{DesktopConfig, ReservedArea, WorkspaceProfile, ZoneSettings};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub use command::{Direction, DispatchCommand, MonitorTarget, WindowSelector, WorkspaceSelector};
pub use config_watch::ConfigFileWatcher;
pub use executor::{
    Executor, NodeExecutionEvent, PendingFocusOp, RuntimeEventKind, RuntimeExtensionEvent,
};
pub use process_manager::StartupProcessState;

#[cfg(test)]
use gevrek_engine::appearance::effective_window_alpha;

pub use gevrek_host::command::{
    run_dispatch_batch, BatchCommandResult, ManualCommand, RuntimeCommand, RuntimeCommandResult,
};

/// Quiet time after the last display notification before reconciling.
const DISPLAY_CHANGE_SETTLE: std::time::Duration = std::time::Duration::from_millis(400);

pub struct Runtime {
    config: Config,
    /// The Win32 backend: it carries out what the core decides.
    window_manager: WindowManager,
    /// Everything the WM knows and decides. The runtime is its host.
    core: core::WmCore,
    window_monitor: WindowMonitor,
    hotkey_manager: HotkeyManager,
    registered_hotkey_ids: Vec<i32>,
    active_binding_submap: String,
    rule_engine: Arc<Mutex<RuleEngine>>,
    startup_process_manager: process_manager::StartupProcessManager,
    /// Persistent workspace creation hooks already fired in this runtime
    /// session. Hot reload internally rebuilds workspace objects and must not
    /// relaunch user programs merely because configuration was reapplied.
    workspace_creation_hooks_fired: HashSet<(crate::dwm::MonitorHandle, String)>,
    persistence: PersistenceManager,
    /// Polling watcher for the configuration file; `None` until the host provides the config
    /// path via `set_config_watch`.
    config_watcher: Option<ConfigFileWatcher>,
    debug_overlay_data: Option<Arc<Mutex<Option<OverlayData>>>>,
    stop_flag: Arc<Mutex<bool>>,
    pause_flag: Arc<Mutex<bool>>,
    config_receiver: Receiver<config_store::ConfigRequest>,
    manual_command_receiver: Receiver<ManualCommand>,
    node_event_broadcast: Option<gevrek_engine::executor::NodeEventBroadcast>,
    event_loop_fps: f64,
    gesture_progress: Option<crate::dwm::GestureProgressData>,
    /// The outputs as last published, to tell subscribers what changed.
    published_monitors: Vec<gevrek_engine::overlay_data::MonitorData>,
    /// Something was told, or done, since the state was last published:
    /// it is published again at the end of the pass. (Who hears of a
    /// change asks for the state; an answer from before the change showed
    /// a panel the old until something else happened.)
    state_stale: std::cell::Cell<bool>,
    /// The corner of a display the pointer is in, and since when
    /// (`desktop.hot_corners`).
    hot_corners: gevrek_engine::hot_corners::Watch,
    gesture_progress_updated: Option<std::time::Instant>,
    /// When the last display notification arrived; the monitor model is
    /// reconciled once notifications have been quiet for a moment.
    display_change_pending: Option<std::time::Instant>,
    /// Windows that opened while the mouse button was held (`held.rs`).
    held_openings: held::Held,
    /// Whether the helper for elevated windows answered when last asked,
    /// and whether its task was already run for want of one.
    elevated_helper: bool,
    elevated_helper_asked: bool,
    /// Publishes the running configuration to readers (the gRPC server).
    config_publisher: Option<config_store::ConfigPublisher>,
    /// `config` changed since the last publish.
    config_dirty: bool,
    /// What there is to tell the person at the machine (`conflicts`).
    notices: Vec<gevrek_engine::overlay_data::Notice>,
    /// When that was last looked into; `None`: on the next pass.
    notices_checked: Option<std::time::Instant>,
    /// The system's wallpaper, followed for its colour while the theme's
    /// accent is the wallpaper's; and when it was last looked at.
    wallpaper_colour: crate::wallpaper_colour::Watch,
    wallpaper_looked: Option<std::time::Instant>,
    /// The night light as it is on the displays.
    night_light: crate::night_light::NightLight,
    /// The wallpaper of the configuration, as it was given to the system.
    wallpaper: crate::wallpaper::Wallpaper,
    /// New display settings wait to be kept (`outputs_changed`).
    display_trial: Option<DisplayTrial>,
    /// Windows' taskbar, put away where the configuration asks for it.
    taskbar: crate::taskbar::Taskbar,
    /// Windows' own movement of windows is switched off (while the
    /// taskbar is put away: `WindowManager::set_system_transitions`).
    transitions_off: bool,
    /// What `desktop.outputs` asks for and Windows does not take.
    display_refused: Option<String>,
    /// The displays' own settings from before `desktop.outputs` changed
    /// them, the oldest first: given back when the configuration asks
    /// nothing of the displays any more (a trial's configuration put
    /// back by the program that tried it, an entry taken out).
    displays_own: Vec<crate::display_settings::Saved>,
    /// The scale each display had before gevrek changed it (in per cent,
    /// by Windows' name for the display): given back when
    /// `desktop.outputs` gives that display no scale any more.
    scales_own: std::collections::HashMap<String, u32>,
}

/// New display settings on trial: a mode the display cannot show after
/// all, or a picture turned the wrong way, would leave a person with no
/// way to put it right. They stay when `output_trial` says `keep`
/// (`gevrek output-trial keep`, the studio's question); with `revert`, or
/// after fifteen seconds without an answer, the displays and
/// `desktop.outputs` are put back as they were.
struct DisplayTrial {
    /// `desktop.outputs` as it was before the first change on trial.
    previous: Vec<gevrek_model::outputs::OutputProfile>,
    /// The displays' settings as they were.
    saved: Vec<crate::display_settings::Saved>,
    until: std::time::Instant,
}

/// The length of a trial; `GEVREK_OUTPUT_TRIAL_SECONDS` sets another (as
/// on Linux: the checks do not wait fifteen seconds).
fn display_trial_length() -> std::time::Duration {
    std::env::var("GEVREK_OUTPUT_TRIAL_SECONDS")
        .ok()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map_or(
            std::time::Duration::from_secs(15),
            std::time::Duration::from_secs_f64,
        )
}

/// How often the wallpaper is looked at for its colour (a name and a date
/// are asked for; the picture is opened only when it is another).
const WALLPAPER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How often other programs are looked for (`conflicts::notices`).
const NOTICE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// How long after the focus left a window its closing still counts as the
/// closing of the focused window (see `output_that_lost_its_focus`).
const FOCUS_LEFT_A_MOMENT_AGO: std::time::Duration = std::time::Duration::from_millis(300);

/// Last-resort safety net: if the runtime is dropped without reaching the
/// clean shutdown at the end of `run` (an early `?` return or a caught
/// panic), put every window Gevrek hid back on screen.
impl Drop for Runtime {
    fn drop(&mut self) {
        let restored = crate::dwm::hidden_journal::emergency_restore();
        if restored > 0 {
            log::warn!("Runtime dropped: restored {restored} hidden window(s)");
        }
    }
}

/// A failure of the runtime loop caused on purpose, to check on a real
/// desktop that the safety nets work: `health` reporting a loop that stopped,
/// and the process stopping (with hidden windows restored) after a panic.
/// Only when `GEVREK_INJECT_FAULT` is set to `stall` or `panic`; it happens
/// `GEVREK_INJECT_FAULT_AFTER_MS` (default 3000) after the loop starts.
struct InjectedFault {
    panic: bool,
    at: std::time::Instant,
}

impl InjectedFault {
    fn from_env() -> Option<Self> {
        let panic = match std::env::var("GEVREK_INJECT_FAULT").ok()?.as_str() {
            "panic" => true,
            "stall" => false,
            _ => return None,
        };
        let after_ms = std::env::var("GEVREK_INJECT_FAULT_AFTER_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(3000);
        log::warn!("GEVREK_INJECT_FAULT is set: the runtime loop will fail on purpose");
        Some(Self {
            panic,
            at: std::time::Instant::now() + std::time::Duration::from_millis(after_ms),
        })
    }

    fn due(&self) -> bool {
        std::time::Instant::now() >= self.at
    }

    fn happen(self) {
        if self.panic {
            panic!("injected runtime panic (GEVREK_INJECT_FAULT=panic)");
        }
        log::warn!("Injected stall: the runtime loop stops for 40 s");
        std::thread::sleep(std::time::Duration::from_secs(40));
    }
}

/// Whether the key that activates zone snapping is held right now.
/// Whether a window is where it is because the pointer carried it there:
/// it has the keyboard and the pointer is on it. (Asked when the mouse
/// button comes up, of a window that opened while it was down.)
fn carried_by_hand(window: crate::dwm::WindowHandle) -> bool {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, GetWindowRect,
    };
    let hwnd = window.to_hwnd();
    let (mut pointer, mut place) = (POINT::default(), RECT::default());
    // SAFETY: plain questions; a window that is gone has no place.
    unsafe {
        GetForegroundWindow() == hwnd
            && GetCursorPos(&mut pointer).is_ok()
            && GetWindowRect(hwnd, &mut place).is_ok()
            && (place.left..place.right).contains(&pointer.x)
            && (place.top..place.bottom).contains(&pointer.y)
    }
}

fn zone_modifier_held(modifier: crate::dwm::ZoneModifier) -> bool {
    use crate::dwm::ZoneModifier;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    // SAFETY: GetAsyncKeyState has no preconditions.
    let pressed = |key: VIRTUAL_KEY| unsafe { GetAsyncKeyState(key.0 as i32) < 0 };
    match modifier {
        ZoneModifier::Shift => pressed(VK_SHIFT),
        ZoneModifier::Control => pressed(VK_CONTROL),
        ZoneModifier::Alt => pressed(VK_MENU),
        ZoneModifier::Win => pressed(VK_LWIN) || pressed(VK_RWIN),
    }
}

/// The runtime's lines to the rest of the program: the flags it shares
/// with the server, where requests and commands come from, the state it
/// tells, and who hears what the graph logs and runs.
pub struct RuntimeLines {
    pub stop_flag: Arc<Mutex<bool>>,
    pub pause_flag: Arc<Mutex<bool>>,
    pub log_entries: Arc<Mutex<Vec<LogEntry>>>,
    pub config_receiver: Receiver<config_store::ConfigRequest>,
    pub manual_command_receiver: Receiver<ManualCommand>,
    pub debug_overlay_data: Option<Arc<Mutex<Option<OverlayData>>>>,
    pub log_broadcast: Option<gevrek_engine::executor::LogBroadcast>,
    pub node_event_broadcast: Option<gevrek_engine::executor::NodeEventBroadcast>,
}

impl Runtime {
    pub fn new(
        config: Config,
        stop_flag: Arc<Mutex<bool>>,
        pause_flag: Arc<Mutex<bool>>,
        log_entries: Arc<Mutex<Vec<LogEntry>>>,
        config_receiver: Receiver<config_store::ConfigRequest>,
        debug_overlay_data: Option<Arc<Mutex<Option<OverlayData>>>>,
        log_broadcast: Option<gevrek_engine::executor::LogBroadcast>,
    ) -> Result<Self> {
        let (_manual_command_sender, manual_command_receiver) = std::sync::mpsc::channel();
        Self::new_with_manual_commands(
            config,
            RuntimeLines {
                stop_flag,
                pause_flag,
                log_entries,
                config_receiver,
                manual_command_receiver,
                debug_overlay_data,
                log_broadcast,
                node_event_broadcast: None,
            },
        )
    }

    pub fn new_with_manual_commands(config: Config, lines: RuntimeLines) -> Result<Self> {
        let RuntimeLines {
            stop_flag,
            pause_flag,
            log_entries,
            config_receiver,
            manual_command_receiver,
            debug_overlay_data,
            log_broadcast,
            node_event_broadcast,
        } = lines;
        log::debug!("Initializing Runtime components...");

        // (Windows cannot cut a window off at a display's edge: the
        // scrolling layout shows whole columns where a column past the
        // edge would be seen on a display beside this one. Looked at
        // again when the displays change: `follow_scrolling_room`.)
        gevrek_engine::layouts::set_scrolling_whole_columns_only(
            crate::dwm::window_manager::columns_must_be_whole(
                &crate::dwm::monitor::all_displays()
                    .into_iter()
                    .map(|(_, info)| (info.x, info.y, info.width, info.height))
                    .collect::<Vec<_>>(),
            ) && !config.desktop.platform.windows.cut_columns_at_displays,
        );
        let mut monitor_manager = MonitorManager::new();
        log::debug!("MonitorManager created");

        // Detect monitors and initialize workspaces (not on the displays
        // the configuration switches off).
        crate::dwm::monitor::set_unmanaged_outputs(&config.desktop.outputs);
        monitor_manager.refresh()?;
        Self::configure_monitor_profiles(&mut monitor_manager, &config.desktop);
        log::debug!("Monitors refreshed and workspaces initialized");

        // Initialize rule engine EARLY so we can pass it to WindowMonitor and WindowManager
        let rule_engine = Self::build_rule_engine(&config.desktop);

        log::info!(
            "RuleEngine initialized with {} float rules",
            rule_engine.float_rules.len()
        );

        // Wrap rule_engine in Arc<Mutex<>> for shared access
        let rule_engine_shared = Arc::new(Mutex::new(rule_engine));

        // Create WindowManager with the RuleEngine
        let mut window_manager = WindowManager::new_with_rules(rule_engine_shared.clone());
        // (With what Windows' own settings ask of every program: less
        // movement, more contrast.)
        crate::system_look::refresh();
        window_manager
            .set_animation_profile(crate::system_look::animations(&config.desktop.animations));
        log::debug!("WindowManager created with RuleEngine filtering");

        // WinEvent callbacks only transport OS events. The runtime thread owns
        // all rule filtering and metadata decisions.
        let window_monitor = WindowMonitor::new();
        log::debug!("WindowMonitor created with runtime-owned rule filtering");

        // Create HotkeyManager
        let hotkey_manager = HotkeyManager::new().map_err(|error| anyhow::anyhow!(error))?;
        log::debug!("HotkeyManager created");

        let mut executor = Executor::new_with_events(
            config.node_graph.clone(),
            log_entries.clone(),
            log_broadcast.clone(),
            node_event_broadcast.clone(),
            rule_engine_shared.clone(),
        );
        executor.set_scratchpad_profiles(config.desktop.scratchpads.clone());
        executor.set_workspace_profiles(
            config.desktop.workspaces.clone(),
            crate::system_look::appearance(&config.desktop.appearance),
        );
        log::debug!("Executor created");

        let focus_manager = FocusManager::with_config(config.desktop.focus.clone());
        log::debug!("FocusManager created");

        let zone_manager = Self::build_zone_manager(&config.desktop.zones, &monitor_manager);
        log::debug!("ZoneManager created");

        let persistence = PersistenceManager::new();
        log::debug!(
            "PersistenceManager created (state path: {})",
            persistence.get_path().display()
        );
        match persistence.load_state() {
            Ok(state) => {
                let count = state.persistent_window_sizes.len();
                executor.set_persistent_window_sizes(state.persistent_window_sizes);
                if count > 0 {
                    log::info!("Loaded {} persistent window size(s)", count);
                }
            }
            Err(error) => log::warn!("Could not load persisted window sizes: {error}"),
        }

        Ok(Self {
            config,
            window_manager,
            window_monitor,
            core: core::WmCore::new(executor, monitor_manager, focus_manager, zone_manager),
            hotkey_manager,
            registered_hotkey_ids: Vec::new(),
            active_binding_submap: "default".to_string(),
            rule_engine: rule_engine_shared,
            startup_process_manager: process_manager::StartupProcessManager::new(),
            workspace_creation_hooks_fired: HashSet::new(),
            persistence,
            config_watcher: None,
            debug_overlay_data,
            stop_flag,
            pause_flag,
            config_receiver,
            manual_command_receiver,
            node_event_broadcast,
            event_loop_fps: 0.0,
            gesture_progress: None,
            published_monitors: Vec::new(),
            state_stale: std::cell::Cell::new(false),
            hot_corners: Default::default(),
            gesture_progress_updated: None,
            display_change_pending: None,
            held_openings: held::Held::default(),
            elevated_helper: false,
            elevated_helper_asked: false,
            config_publisher: None,
            config_dirty: false,
            notices: Vec::new(),
            notices_checked: None,
            wallpaper_colour: Default::default(),
            wallpaper_looked: None,
            night_light: Default::default(),
            wallpaper: Default::default(),
            display_trial: None,
            taskbar: Default::default(),
            transitions_off: false,
            display_refused: None,
            displays_own: Vec::new(),
            scales_own: std::collections::HashMap::new(),
        })
    }

    /// Make this runtime the owner that readers see. The current
    /// configuration is published right away.
    pub fn set_config_publisher(&mut self, publisher: config_store::ConfigPublisher) {
        publisher.publish(&self.config);
        self.config_publisher = Some(publisher);
        self.config_dirty = false;
    }

    /// Apply a configuration the server asked for, or a change of the
    /// configuration file. Runs while execution is paused too: an Apply from
    /// the editor must not wait for the resume.
    fn apply_configuration_changes(&mut self) {
        // Resolved here, on the runtime thread, against the running
        // config (see `ConfigRequest::resolve`).
        let mut request = self.config_receiver.try_recv().ok();
        if let Some(base) = request.as_ref().and_then(|request| request.base_revision()) {
            // A change made here since the last publish (a hotkey toggle)
            // gets its revision first; the request is then judged against
            // what really runs.
            self.publish_config_if_changed();
            let running = self
                .config_publisher
                .as_ref()
                .map(|publisher| publisher.revision());
            if running.is_some_and(|running| running != base) {
                log::info!(
                    "Refusing a configuration based on revision {base}: revision {} is running",
                    running.unwrap_or_default()
                );
                // The server already saved the refused configuration; put
                // the running one back so the file watcher does not load it.
                if let Some(watcher) = &mut self.config_watcher {
                    if let Err(error) = self.config.save(watcher.path()) {
                        log::warn!("Could not restore the configuration file: {error}");
                    }
                    watcher.acknowledge();
                }
                if let Some(publisher) = &self.config_publisher {
                    publisher.refuse_request();
                }
                request = None;
            }
        }
        let from_request = request.is_some();
        let mut hot_reload = request.map(|request| request.resolve(&self.config));
        if hot_reload.is_some() {
            // Channel-driven reloads include the WM's own config writes
            // (UpdateConfiguration and PushGraph persist the file before
            // pushing), so acknowledge the current mtime to keep the
            // watcher from reloading the same change again.
            if let Some(watcher) = &mut self.config_watcher {
                watcher.acknowledge();
            }
        } else if let Some(watcher) = &mut self.config_watcher {
            // Poll the config file for
            // external edits and hot-reload on a confirmed change.
            if watcher.poll() {
                match Config::load(watcher.path()) {
                    Ok(config) => {
                        log::info!(
                            "Configuration file changed on disk; hot-reloading {}",
                            watcher.path().display()
                        );
                        hot_reload = Some(config);
                    }
                    Err(error) => {
                        log::warn!("Configuration file changed but could not be loaded: {error}");
                    }
                }
            }
        }
        if let Some(new_config) = hot_reload {
            self.reload_configuration(new_config);
            // The state as it is now, before anybody is told that the
            // configuration is: who made the change asks for the state as
            // soon as it hears, and is to get what the change made of it
            // (a workspace's new title, on its tab).
            self.send_debug_overlay_data(Some("configuration".to_string()));
            if from_request {
                // Its own publish, right away: the server waits for this
                // request, and its revision must not be shared with a
                // toggle made later in the same pass.
                self.config_dirty = false;
                if let Some(publisher) = &self.config_publisher {
                    let revision = publisher.publish_request(&self.config);
                    log::debug!("Published requested configuration as revision {}", revision);
                }
            }
        }
    }

    /// Publish the configuration if it changed since the last publish.
    fn publish_config_if_changed(&mut self) {
        if !self.config_dirty {
            return;
        }
        self.config_dirty = false;
        if let Some(publisher) = &self.config_publisher {
            let revision = publisher.publish(&self.config);
            log::debug!("Published running configuration revision {}", revision);
        }
    }

    /// Point the configuration file watcher at the WM's config file
    ///. The initial enabled state comes from
    /// `desktop.watch_configuration`.
    pub fn set_config_watch(&mut self, path: std::path::PathBuf) {
        let enabled = self.config.desktop.watch_configuration;
        self.config_watcher = Some(ConfigFileWatcher::new(path, enabled));
        log::info!(
            "Configuration file watch {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    fn emit_runtime_event(
        &self,
        kind: RuntimeEventKind,
        window_handle: Option<crate::dwm::WindowHandle>,
        fields: impl IntoIterator<Item = (String, String)>,
    ) {
        // (A window that is dragged tells of its place many times a
        // second; nothing that is published follows it.)
        if !matches!(
            kind,
            RuntimeEventKind::WindowMoved | RuntimeEventKind::WindowResized
        ) {
            self.state_stale.set(true);
        }
        let Some(broadcast) = &self.node_event_broadcast else {
            return;
        };
        broadcast(NodeExecutionEvent::Runtime(RuntimeExtensionEvent {
            kind,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            window_handle,
            fields: fields.into_iter().collect(),
        }));
    }

    fn emit_window_extension_event(&self, event: &crate::dwm::WindowEvent) {
        let (kind, handle, fields) = RuntimeEventKind::of_window_event(event);
        self.emit_runtime_event(kind, Some(handle), fields);
    }

    pub fn run(&mut self) -> Result<()> {
        log::info!("=== Starting Gevrek Window Manager Runtime ===");
        self.emit_runtime_event(
            RuntimeEventKind::RuntimeStarted,
            None,
            [
                ("config_version".to_string(), self.config.version.clone()),
                (
                    "schema_version".to_string(),
                    self.config.desktop.schema_version.to_string(),
                ),
            ],
        );

        // Prevent Windows from ghosting our windows as "Not Responding".
        // Window managers routinely block the message loop during layout/event
        // processing, which would otherwise trigger the ghost-window mechanism.
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::DisableProcessWindowsGhosting();
        }
        log::info!("Disabled process window ghosting");

        // Refresh initial window list
        log::info!("Enumerating existing windows...");
        self.window_manager.refresh_window_list()?;
        let window_count = self.window_manager.get_all_windows().len();
        log::info!("Found {} existing windows", window_count);

        // Minimized windows stay minimized: they are adopted when restored
        // (no forced SW_RESTORE at startup).

        // Execute startup events
        log::info!("Executing startup event nodes...");
        self.core.executor.clear_state();
        self.core
            .executor
            .execute_startup(&mut self.window_manager, &mut self.core.monitor_manager)?;
        self.apply_workspace_profiles();
        log::info!("Startup execution completed");

        // Register hotkeys from OnKeyPress nodes
        self.register_hotkeys()?;

        // Ensure each monitor has at least one workspace (auto-create default if needed)
        self.ensure_default_workspaces()?;

        // Adopt windows that already exist: without this,
        // only windows created after WM start would ever be managed.
        if self.config.desktop.manage_existing_on_startup {
            self.adopt_existing_windows()?;
            // The windows that were open when the WM last stopped go back
            // to the workspaces they were on, if the user asked for that.
            let saved = if self.config.desktop.restore_session_on_startup {
                self.persistence.load_state()
            } else {
                Err(anyhow::anyhow!("restore_session_on_startup is off"))
            };
            match saved {
                Ok(state) => match self.core.restore_session(&mut self.window_manager, &state) {
                    Ok(0) => {}
                    Ok(moved) => log::info!(
                        "Put {} window(s) back on their workspaces from the saved session",
                        moved
                    ),
                    Err(error) => log::warn!("Could not restore the saved session: {error}"),
                },
                Err(error) => log::debug!("No saved session to restore: {error}"),
            }
        }

        self.apply_appearance_profile();
        self.startup_process_manager
            .configure(&self.config.desktop.startup, false);
        self.startup_process_manager.tick();

        // A taskbar that a window manager which was ended left hidden.
        crate::taskbar::Taskbar::show_what_was_left_hidden();
        // Whose the Windows key is, and whether it drags windows.
        crate::windows_key::configure(
            self.config.desktop.platform.windows.own_windows_key,
            self.config.desktop.platform.windows.drag_with_windows_key,
        );
        // The displays as `desktop.outputs` asks, and what each offers.
        self.apply_display_settings_at_start();
        self.tell_display_modes();
        // Send initial data to debug overlay
        self.send_debug_overlay_data(None);

        // Main event loop
        log::info!("Entering main event loop");
        let mut iteration = 0u64;
        let mut last_overlay_update = std::time::Instant::now();
        let mut last_fps_update = std::time::Instant::now();
        let mut iterations_since_fps_update = 0u64;
        let mut last_ffm_window: Option<crate::dwm::WindowHandle> = None;
        let mut last_ffm_check = std::time::Instant::now();
        let mut last_animation_count = 0usize;
        let mut input_suspended = false;
        let mut injected_fault = InjectedFault::from_env();
        let mut last_bar_check = std::time::Instant::now();
        let mut bar_kept = self.core.monitor_manager.bar_is_kept(&self.window_manager);
        // The loop waits for work between passes (`wait`); what used to be
        // counted in passes is therefore counted in time.
        wait::prepare();
        let mut last_work = Some(std::time::Instant::now());
        // When the focus last went from one window to another, and from
        // which.
        let mut focus_left: Option<(std::time::Instant, Option<crate::dwm::WindowHandle>)> = None;
        let mut last_drag_check = std::time::Instant::now();
        let mut last_cleanup = std::time::Instant::now();

        // (How long a pass takes and which part of it: told at the log's
        // debug level only.)
        let mut pass_times = pass_times::PassTimes::new(std::time::Instant::now());

        loop {
            let _loop_start = std::time::Instant::now();
            pass_times.begin(_loop_start);
            // `health` reports a loop that stopped passing here.
            process_guard::beat();
            if process_guard::session_end_called_off() {
                let hidden = self
                    .core
                    .executor
                    .hide_hidden_windows_again(&mut self.window_manager);
                log::info!(
                    "The session's end was called off: {hidden} window(s) of workspaces that are not shown are hidden again"
                );
            }
            if let Some(fault) = injected_fault.take_if(|fault| fault.due()) {
                fault.happen();
            }

            // Check stop flag (handle poisoned mutex gracefully)
            // Use a scope to ensure the guard is dropped before we call mutable methods
            {
                let should_stop = self
                    .stop_flag
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if *should_stop {
                    log::info!("Stop flag detected, exiting main event loop");
                    break;
                }
            }

            // Run every IPC command queued since the last wake (one per frame
            // used to add ~16 ms of latency per queued command). Commands are
            // drained up front, so a client flooding the queue cannot starve
            // window events.
            let queued_commands: Vec<ManualCommand> =
                self.manual_command_receiver.try_iter().collect();
            if !queued_commands.is_empty() {
                last_work = Some(std::time::Instant::now());
            }
            for command in queued_commands {
                // The caller gives up after its reply timeout and drops the
                // receiver. Running the command afterwards would apply it
                // after the client was told it failed.
                if command.caller_gave_up() {
                    log::warn!(
                        "Skipping runtime command {:?}: the caller already timed out",
                        command.command
                    );
                    continue;
                }
                let label = format!("{:?}", command.command);
                let result = match &command.command {
                    // A built-in alias runs as its command; a number names a
                    // Manual Trigger node of the graph.
                    RuntimeCommand::Trigger(target) => {
                        match Executor::manual_target_command(target) {
                            Ok(Some(dispatch)) => self.execute_runtime_dispatch(dispatch),
                            Ok(None) => self.core.executor.execute_manual_target(
                                &mut self.window_manager,
                                &mut self.core.monitor_manager,
                                target,
                            ),
                            Err(error) => Err(error),
                        }
                        .map(|_| RuntimeCommandResult::Complete)
                    }
                    RuntimeCommand::Dispatch(dispatch) => self
                        .execute_runtime_dispatch(dispatch.clone())
                        .map(|_| RuntimeCommandResult::Complete),
                    RuntimeCommand::DispatchBatch {
                        commands,
                        stop_on_error,
                    } => {
                        let results = run_dispatch_batch(commands, *stop_on_error, |dispatch| {
                            self.execute_runtime_dispatch(dispatch.clone())
                                .map_err(|error| error.to_string())
                        });
                        Ok(RuntimeCommandResult::Batch(results))
                    }
                    // On Windows a check sends real input through the
                    // system (tests/desktop does); the WM has no input
                    // path of its own to feed.
                    RuntimeCommand::InjectInput(_) => Err(anyhow::anyhow!(
                        "Made-up input is not taken on Windows; send real input through the system"
                    )),
                }
                .map_err(|error| error.to_string());

                if let Err(result) = &result {
                    log::warn!("Runtime command '{}' failed: {}", label, result);
                }
                let _ = command.response.send(result);
            }

            pass_times.mark(std::time::Instant::now());
            // Check pause flag (handle poisoned mutex gracefully)
            let is_paused = *self
                .pause_flag
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if is_paused != input_suspended {
                // Paused: bindings stop consuming input and queued input is
                // dropped, so keys pressed during the pause are neither
                // swallowed nor replayed on resume.
                self.hotkey_manager.set_input_suspended(is_paused);
                input_suspended = is_paused;
                log::info!(
                    "Input bindings {}",
                    if is_paused { "suspended" } else { "resumed" }
                );
            }
            if is_paused {
                // Window events stay queued and are coalesced on resume.
                // Configuration changes are still applied and published.
                self.apply_configuration_changes();
                self.publish_config_if_changed();
                // New display settings nobody kept in time go back, paused
                // or not: a display that shows nothing cannot be answered.
                if self
                    .display_trial
                    .as_ref()
                    .is_some_and(|trial| std::time::Instant::now() >= trial.until)
                {
                    log::warn!("The new display settings were not kept: going back to the ones before");
                    self.answer_display_trial(gevrek_model::command::TrialAnswer::Revert);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }

            iteration += 1;
            iterations_since_fps_update += 1;
            if self.gesture_progress_updated.is_some_and(|updated| {
                updated.elapsed() >= std::time::Duration::from_millis(750)
                    && self.gesture_progress.as_ref().is_some_and(|progress| {
                        matches!(progress.phase.as_str(), "completed" | "cancelled")
                    })
            }) {
                self.gesture_progress = None;
                self.gesture_progress_updated = None;
            }
            if last_fps_update.elapsed() >= std::time::Duration::from_secs(1) {
                self.event_loop_fps =
                    iterations_since_fps_update as f64 / last_fps_update.elapsed().as_secs_f64();
                iterations_since_fps_update = 0;
                last_fps_update = std::time::Instant::now();
            }

            // Send overlay data every second
            if last_overlay_update.elapsed() >= std::time::Duration::from_secs(1) {
                self.send_debug_overlay_data(None);
                last_overlay_update = std::time::Instant::now();
            }

            self.apply_configuration_changes();

            // Display topology/work-area/DPI/resume notifications arrive in
            // bursts (a dock sends several over a few hundred ms); reconcile
            // once they settle.
            if self.window_monitor.poll_display_changes() {
                self.display_change_pending = Some(std::time::Instant::now());
                last_work = self.display_change_pending;
            }
            if self
                .display_change_pending
                .is_some_and(|at| at.elapsed() >= DISPLAY_CHANGE_SETTLE)
            {
                self.display_change_pending = None;
                self.handle_display_change();
            }

            // New display settings nobody kept in time go back.
            if self
                .display_trial
                .as_ref()
                .is_some_and(|trial| std::time::Instant::now() >= trial.until)
            {
                log::warn!("The new display settings were not kept: going back to the ones before");
                self.answer_display_trial(gevrek_model::command::TrialAnswer::Revert);
            }

            // A theme that takes its accent from the wallpaper follows the
            // system's picture, as soon as that has been read.
            if self
                .wallpaper_looked
                .is_none_or(|at| at.elapsed() >= WALLPAPER_INTERVAL)
            {
                self.wallpaper_looked = Some(std::time::Instant::now());
                self.follow_wallpaper_colour();
                // At the same pace: the night light's hours come and go,
                // and Windows gives a display its usual colours back when
                // it likes.
                if self.night_light.follow(
                    &self.config.desktop.night_light,
                    crate::night_light::minutes_now(),
                ) {
                    self.refresh_notices();
                }
                // And the wallpaper the configuration names, which the
                // system draws (a slideshow's next picture comes so too).
                self.follow_wallpaper();
                // And Windows' taskbar, where it is to be put away
                // (Windows shows it again now and then).
                // (Only while gevrek's own panel is there to take its
                // place: without one a desktop would be left with neither.)
                // (And the helper for the windows of programs that run
                // as an administrator: found when it comes, missed when
                // it goes.)
                self.follow_elevated_helper();
                // (And the Windows key, as the configuration has it now.)
                crate::windows_key::configure(
                    self.config.desktop.platform.windows.own_windows_key,
                    self.config.desktop.platform.windows.drag_with_windows_key,
                );
                let hide = self.config.desktop.platform.windows.hide_taskbar
                    && self.core.monitor_manager.bar_is_kept(&self.window_manager);
                self.taskbar.follow(hide);
                // (A window that is minimised then does not fly to the
                // taskbar nobody sees.)
                if self.transitions_off != self.taskbar.is_hidden() {
                    self.transitions_off = self.taskbar.is_hidden();
                    self.window_manager
                        .set_system_transitions_for_all(self.transitions_off);
                }
                if crate::dwm::monitor::set_taskbar_hidden(self.taskbar.is_hidden()) {
                    // Its strip is room for windows, or no longer.
                    self.display_change_pending = Some(std::time::Instant::now());
                }
            }

            // Now and then: is there something to tell the person?
            if self
                .notices_checked
                .is_none_or(|at| at.elapsed() >= NOTICE_INTERVAL)
            {
                self.notices_checked = Some(std::time::Instant::now());
                self.refresh_notices();
                // And: did Windows' own settings for movement and contrast
                // change? What follows them is carried out again.
                if crate::system_look::refresh() {
                    log::info!("Windows' settings for movement or contrast changed: applied");
                    self.window_manager
                        .set_animation_profile(crate::system_look::animations(
                            &self.config.desktop.animations,
                        ));
                    self.core.executor.set_workspace_profiles(
                        self.config.desktop.workspaces.clone(),
                        crate::system_look::appearance(&self.config.desktop.appearance),
                    );
                    self.apply_appearance_profile();
                }
            }

            pass_times.mark(std::time::Instant::now());
            // Poll window events
            let before_poll = std::time::Instant::now();
            let mut events = self.window_monitor.poll_events();
            let poll_duration = before_poll.elapsed();
            // A window that opens while the mouse button is held (a tab
            // pulled out of a browser) is told of when the button is up
            // (`held.rs`).
            {
                use gevrek_engine::contract::Desktop;
                let button_down = self.window_manager.primary_button_down();
                let outputs = &self.core.monitor_manager;
                events = self.held_openings.pass(
                    events,
                    button_down,
                    |window| outputs.find_window(window).is_none(),
                    carried_by_hand,
                );
                if self.held_openings.any() {
                    // (Nothing announces the button's release.)
                    last_work = Some(std::time::Instant::now());
                }
            }
            // A window that opens is heard of as opened before it is heard
            // of as having the keyboard, whichever Windows told first: the
            // display with the keyboard is the one it was asked for on, and
            // it must still be that one when the window is given its place
            // (the keyboard's display follows a window that gets it).
            for at in 0..events.len() {
                if let crate::dwm::WindowEvent::Focused(window) = events[at] {
                    let opened = events[at + 1..].iter().position(|event| {
                        matches!(event, crate::dwm::WindowEvent::Created(other) if *other == window)
                    });
                    if let Some(later) = opened {
                        let created = events.remove(at + 1 + later);
                        events.insert(at, created);
                    }
                }
            }
            // A workspace with no window was just shown, and the desktop
            // has the keyboard on purpose: a window that Windows gives the
            // keyboard to by itself in that moment (it does when the window
            // that had it is hidden, and picks one on any display) does
            // not get it, and the display keeps the keyboard. A window the
            // person presses on does.
            if crate::dwm::window_manager::desktop_kept_just_now()
                && crate::windows_key::pressed_just_now().is_none()
            {
                use crate::dwm::HwndExt;
                // SAFETY: a plain query.
                let desktop = unsafe { windows::Win32::UI::WindowsAndMessaging::GetShellWindow() };
                let before = events.len();
                events.retain(|event| {
                    !matches!(event, crate::dwm::WindowEvent::Focused(window)
                        if window.to_hwnd() != desktop)
                });
                if events.len() != before {
                    log::debug!("A window Windows gave the keyboard to by itself does not keep it");
                    self.window_manager.refocus_desktop();
                }
            }
            // A press on the desktop itself gives the display that was
            // pressed on the keyboard (`Desktop::output_of`).
            if let Some(desktop) = crate::windows_key::desktop_pressed() {
                use crate::dwm::HwndExt;
                use windows::Win32::Foundation::HWND;
                events.push(crate::dwm::WindowEvent::Focused(
                    crate::dwm::WindowHandle::from_hwnd(HWND(desktop as *mut _)),
                ));
            }
            // A window dragged with the Windows key and the mouse is a
            // drag as any other.
            for drag in crate::windows_key::take_events() {
                use crate::dwm::HwndExt;
                use windows::Win32::Foundation::HWND;
                events.push(match drag {
                    crate::windows_key::DragEvent::Started(window) => {
                        crate::dwm::WindowEvent::DragStarted(crate::dwm::WindowHandle::from_hwnd(
                            HWND(window as *mut _),
                        ))
                    }
                    crate::windows_key::DragEvent::Ended(window) => {
                        crate::dwm::WindowEvent::DragEnded(crate::dwm::WindowHandle::from_hwnd(
                            HWND(window as *mut _),
                        ))
                    }
                });
            }

            // A window picked by a press on it (`pick.rs`).
            if let Some(picked) = crate::pick::take() {
                use crate::dwm::HwndExt;
                use windows::Win32::Foundation::HWND;
                // (A window gevrek does not know of, the taskbar or the
                // desktop itself, is not one a rule is for.)
                let known = picked.and_then(|(window, class)| {
                    let window = crate::dwm::WindowHandle::from_hwnd(HWND(window as *mut _));
                    let title = self.window_manager.get_window_info(window)?.title.clone();
                    Some((window, class, title))
                });
                match known {
                    Some((window, class, title)) => self.emit_runtime_event(
                        RuntimeEventKind::WindowPicked,
                        Some(window),
                        [("class".to_string(), class), ("title".to_string(), title)],
                    ),
                    None => self.emit_runtime_event(
                        RuntimeEventKind::WindowPicked,
                        None,
                        [("cancelled".to_string(), "true".to_string())],
                    ),
                }
            }

            if poll_duration.as_millis() > 50 {
                log::warn!(
                    "Slow poll_events: took {}ms, got {} events",
                    poll_duration.as_millis(),
                    events.len()
                );
            }

            // Collapse redundant geometry/title events per window, so a title-
            // spamming app or a drag costs one handler run per frame.
            let raw_event_count = events.len();
            events = event_coalescing::coalesce_window_events(events);
            if raw_event_count > 20 {
                log::debug!(
                    "Coalesced {} window events into {}",
                    raw_event_count,
                    events.len()
                );
            }

            // Poll hotkey events
            let hotkey_events = self.hotkey_manager.poll_events();
            if !hotkey_events.is_empty() {
                last_work = Some(std::time::Instant::now());
                log::debug!(
                    "Iteration {}: {} hotkey events received",
                    iteration,
                    hotkey_events.len()
                );
                for hotkey_event in hotkey_events {
                    log::info!(
                        "Input triggered: device={:?}, trigger={:?}, key={}, ctrl={}, alt={}, shift={}, win={}, held_ms={}",
                        hotkey_event.device,
                        hotkey_event.trigger,
                        hotkey_event.key,
                        hotkey_event.ctrl,
                        hotkey_event.alt,
                        hotkey_event.shift,
                        hotkey_event.win,
                        hotkey_event.held_ms,
                    );
                    self.emit_runtime_event(
                        RuntimeEventKind::BindingTriggered,
                        None,
                        [
                            ("device".to_string(), format!("{:?}", hotkey_event.device)),
                            ("trigger".to_string(), format!("{:?}", hotkey_event.trigger)),
                            ("key".to_string(), hotkey_event.key.clone()),
                            ("ctrl".to_string(), hotkey_event.ctrl.to_string()),
                            ("alt".to_string(), hotkey_event.alt.to_string()),
                            ("shift".to_string(), hotkey_event.shift.to_string()),
                            ("win".to_string(), hotkey_event.win.to_string()),
                            ("held_ms".to_string(), hotkey_event.held_ms.to_string()),
                        ],
                    );
                    let graph_triggered = hotkey_event.registration_ids.iter().any(|id| *id < 0);
                    let declarative_commands = hotkey_event
                        .registration_ids
                        .iter()
                        .filter_map(|id| {
                            let index = id.checked_sub(1_000_000)? as usize;
                            self.config
                                .desktop
                                .bindings
                                .get(index)
                                .map(|binding| binding.command.clone())
                        })
                        .collect::<Vec<_>>();
                    for command in declarative_commands {
                        if let Err(error) = self.execute_runtime_dispatch(command) {
                            log::error!("Error executing declarative binding: {}", error);
                        }
                    }
                    // Trigger OnKeyPress nodes that match this hotkey
                    if graph_triggered {
                        if let Err(e) = self.core.executor.execute_hotkey(
                            &mut self.window_manager,
                            &mut self.core.monitor_manager,
                            &mut self.core.focus_manager,
                            &hotkey_event,
                        ) {
                            log::error!("Error executing hotkey event: {}", e);
                        }
                    }

                    // Handle pending focus operations (from CycleFocusNext/CycleFocusPrevious nodes)
                    if let Some(focus_op) = self.core.executor.take_pending_focus_op() {
                        match focus_op {
                            PendingFocusOp::CycleNext => {
                                if let Some(window_handle) = self.core.focus_manager.cycle_next() {
                                    if let Err(e) = self.window_manager.focus_window(window_handle)
                                    {
                                        log::error!("Error focusing window: {}", e);
                                    }
                                    log::info!("Cycled to next window in focus history");
                                }
                            }
                            PendingFocusOp::CyclePrevious => {
                                if let Some(window_handle) =
                                    self.core.focus_manager.cycle_previous()
                                {
                                    if let Err(e) = self.window_manager.focus_window(window_handle)
                                    {
                                        log::error!("Error focusing window: {}", e);
                                    }
                                    log::info!("Cycled to previous window in focus history");
                                }
                            }
                        }
                    }
                }
            }

            let gesture_events = self.hotkey_manager.poll_gesture_events();
            if !gesture_events.is_empty() {
                last_work = Some(std::time::Instant::now());
            }
            for gesture_event in gesture_events {
                self.process_gesture_event(gesture_event);
            }

            // Focus effects only change with focus or window membership.
            let refresh_focus_effects = event_coalescing::affects_focus_effects(&events);
            // The hooks see every program's windows. A title that changes
            // or a window that moves is only work when gevrek has that
            // window; otherwise a clock in some title bar would keep the
            // loop passing once a frame for good.
            let work = events.iter().any(|event| match event {
                crate::dwm::WindowEvent::Moved(handle, _, _)
                | crate::dwm::WindowEvent::Resized(handle, _, _)
                | crate::dwm::WindowEvent::MetadataChanged(handle) => {
                    self.core.monitor_manager.find_window(*handle).is_some()
                }
                _ => true,
            });
            if work {
                last_work = Some(std::time::Instant::now());
            }

            for event in events {
                // Only log important events (not move/resize which spam the console)
                match &event {
                    crate::dwm::WindowEvent::Created(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::Destroyed(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::Focused(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::Moved(_, _, _) => {} // log::debug!("Processing window event: {:?}", event),
                    crate::dwm::WindowEvent::Resized(_, _, _) => {} // log::debug!("Processing window event: {:?}", event),
                    crate::dwm::WindowEvent::DragStarted(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::DragEnded(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::Minimized(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::Restored(_) => {
                        log::info!("Processing window event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::MetadataChanged(_) => {
                        log::debug!("Processing window metadata event: {:?}", event)
                    }
                    crate::dwm::WindowEvent::Unmapped(_) => {
                        log::debug!("Processing window event: {:?}", event)
                    }
                }

                self.emit_window_extension_event(&event);

                // Update the window registry for this one window only. The
                // full EnumWindows refresh used to run here for every title
                // change, which made title-spamming apps burn CPU.
                self.window_manager.apply_window_event(&event);
                let needs_overlay_update = matches!(
                    event,
                    crate::dwm::WindowEvent::Created(_)
                        | crate::dwm::WindowEvent::Destroyed(_)
                        | crate::dwm::WindowEvent::Minimized(_)
                        | crate::dwm::WindowEvent::Restored(_)
                );
                if let crate::dwm::WindowEvent::Created(handle) = event {
                    self.apply_appearance_to_window(handle);
                    // (Of gevrek's own windows only: a menu keeps the way
                    // its program lets it appear.)
                    if self.transitions_off && !self.window_manager.should_ignore_window(handle) {
                        self.window_manager.set_system_transitions(handle, true);
                    }
                }

                // The focus leaving a window, and a window closing: when
                // the window that had the focus closes, its workspace
                // keeps the focus (see `output_that_lost_its_focus`).
                let lost_focus = match &event {
                    crate::dwm::WindowEvent::Focused(to) => {
                        let from = self.core.focus_manager.current();
                        if from != Some(*to) {
                            focus_left = Some((std::time::Instant::now(), from));
                        }
                        None
                    }
                    // A window that closes is hidden first, and that is
                    // told first too: by the time its end is told it has
                    // already left the layout, and nobody would know any
                    // more which workspace lost its focus. So the hiding
                    // counts as well (a window that hides itself for good,
                    // to the tray, leaves its workspace just the same).
                    crate::dwm::WindowEvent::Destroyed(closed)
                    | crate::dwm::WindowEvent::Unmapped(closed) => self
                        .output_that_lost_its_focus(*closed, focus_left)
                        .map(|output| (output, *closed)),
                    _ => None,
                };

                let took_keyboard = match &event {
                    crate::dwm::WindowEvent::Focused(window) => Some(*window),
                    _ => None,
                };
                let changed_size = match &event {
                    crate::dwm::WindowEvent::Moved(window, _, _)
                    | crate::dwm::WindowEvent::Resized(window, _, _) => Some(*window),
                    _ => None,
                };
                // A window dropped on a display the configuration switches
                // off is let go there: that display, and what is on it, is
                // left alone.
                if let crate::dwm::WindowEvent::DragEnded(window) = &event {
                    use crate::dwm::HwndExt;
                    if crate::dwm::monitor::is_on_unmanaged_display(window.to_hwnd()) {
                        self.core
                            .release_newly_ignored_windows(&mut self.window_manager);
                    }
                }

                // Execute event with timing
                let event_name = format!("{:?}", event);
                let start = std::time::Instant::now();
                if let Err(e) = self
                    .core
                    .handle_window_event(&mut self.window_manager, event)
                {
                    log::error!("Error executing window event: {}", e);
                }
                // Windows raises the window that gets the keyboard above
                // everything: a tiled one then lies over the floating
                // windows of its workspace, which belong above the tiled
                // ones. It is put below them again (it keeps the keyboard).
                // The borders are put away for the overview, all of them
                // at once (that is how this backend hides them). While an
                // overview is up somewhere, a window elsewhere that gets
                // the keyboard (another display, another workspace) has
                // its border all the same, and the borders are put away
                // again when the keyboard comes back into the overview.
                // (Before, no window anywhere had a border until the
                // overview was left.)
                if let Some(window) = took_keyboard {
                    if self.core.executor.overview_is_up() {
                        let inside = self
                            .core
                            .executor
                            .is_in_overview(&self.core.monitor_manager, window);
                        if inside {
                            self.window_manager.hide_all_borders();
                            self.window_manager.hide_all_stackbars();
                        } else {
                            self.window_manager.show_all_borders();
                            self.window_manager.show_all_stackbars();
                            self.apply_focus_effects();
                        }
                    }
                }
                if let Some(window) = took_keyboard {
                    let floating = gevrek_engine::focus::floating_to_keep_above(
                        &self.core.monitor_manager,
                        window,
                    );
                    if !floating.is_empty() {
                        if let Err(error) = self.window_manager.put_below(window, &floating) {
                            log::debug!(
                                "a tiled window was not put below the floating ones: {error}"
                            );
                        }
                    }
                }
                if let Some(window) = changed_size {
                    self.keep_maximised_clear_of_panel(window);
                }
                // Only when the window really left the layout: one that
                // gevrek hid itself (another tab chosen, a scratchpad put
                // away) is still there, and gevrek has seen to the focus.
                if let Some((output, closed)) = lost_focus {
                    if self.core.monitor_manager.find_window(closed).is_none() {
                        self.focus_successor_on(output);
                    }
                }
                let duration = start.elapsed();

                // Log slow event processing
                if duration.as_millis() > 100 {
                    log::warn!(
                        "Slow event processing: {} took {}ms",
                        event_name,
                        duration.as_millis()
                    );
                }

                // Send overlay update for important events
                if needs_overlay_update {
                    self.send_debug_overlay_data(Some(event_name.clone()));
                }
            }

            if refresh_focus_effects {
                self.apply_focus_effects();
            }

            pass_times.mark(std::time::Instant::now());
            // Flush any pending layouts that were deferred during event processing.
            // This coalesces multiple events (e.g., 5 windows created) into one layout pass.
            if let Err(e) = self.core.flush_pending_layouts(&mut self.window_manager) {
                log::error!("Error flushing pending layouts: {}", e);
            }
            if let Err(e) = self.window_manager.tick_runtime_animations() {
                log::error!("Error advancing runtime animations: {}", e);
            }
            // (What this pass hid and showed is in the journal's file.)
            crate::dwm::hidden_journal::flush();
            // (After the windows were put where they are for this pass.)
            self.cut_columns();
            self.startup_process_manager.tick();
            let active_animation_count = self.window_manager.active_animation_count();
            if active_animation_count != last_animation_count {
                last_animation_count = active_animation_count;
                self.send_debug_overlay_data(Some(format!(
                    "Animations active: {active_animation_count}"
                )));
            }

            pass_times.mark(std::time::Instant::now());
            // Process pending window drags - now safe with proper WM_ENTERSIZEMOVE/EXITSIZEMOVE detection
            // This handles fallback cases where DragEnded event might be missed
            if last_drag_check.elapsed() >= std::time::Duration::from_millis(500) {
                last_drag_check = std::time::Instant::now();
                if let Err(e) = self.core.executor.process_pending_drags(
                    &mut self.window_manager,
                    &mut self.core.monitor_manager,
                    &mut self.core.zone_manager,
                ) {
                    log::error!("Error processing pending drags: {}", e);
                }
            }

            // The window registry is maintained incrementally from events; there
            // is no periodic EnumWindows refresh any more. A destroy event that
            // was missed is caught by the IsWindow sweep below. (The overlay
            // data goes out once a second further up.)

            // Cleanup stale tracking data periodically (every 5 seconds)
            if last_cleanup.elapsed() >= std::time::Duration::from_secs(5) {
                last_cleanup = std::time::Instant::now();
                let pruned = self.window_manager.prune_dead_windows();
                if pruned > 0 {
                    log::debug!("Pruned {} dead window(s) from the registry", pruned);
                }
                self.core.executor.cleanup_stale_members(
                    &self.window_manager,
                    &mut self.core.monitor_manager,
                    &mut self.core.zone_manager,
                );
                match self.window_manager.cleanup_stale_borders() {
                    Ok(count) if count > 0 => {
                        log::debug!("Cleaned up {} stale border overlay(s)", count)
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("Failed to clean up stale border overlays: {}", e),
                }
                match self.window_manager.cleanup_stale_stackbars() {
                    Ok(count) if count > 0 => {
                        log::debug!("Cleaned up {} stale stackbar(s)", count)
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("Failed to clean up stale stackbars: {}", e),
                }
            }

            // Hot corners: the pointer, left in a corner of a display
            // for a moment, runs that corner's command.
            if self.config.desktop.hot_corners.watched() {
                if let Some(command) = self.hot_corner_due() {
                    log::info!("A hot corner runs: {}", command.display_name());
                    if let Err(error) = self.execute_runtime_dispatch(command) {
                        log::error!("A hot corner's command could not be run: {error}");
                    }
                }
            }

            // Focus-follows-mouse: check cursor position and focus window under cursor
            if self.core.focus_manager.focus_follows_mouse() {
                let delay = std::time::Duration::from_millis(
                    self.core.focus_manager.config().focus_delay_ms as u64,
                );
                if last_ffm_check.elapsed() >= delay {
                    last_ffm_check = std::time::Instant::now();
                    if let Some(handle) = Self::get_window_under_cursor() {
                        // Only focus if it's a different window AND not a border/internal window
                        if last_ffm_window != Some(handle)
                            && !self.window_manager.is_border_window(handle)
                            && self.core.executor.allows_follow_mouse_focus(handle)
                        {
                            // Only focus tiled windows (skip floating/unmanaged)
                            if self.core.monitor_manager.find_window(handle).is_some() {
                                let _ = self.window_manager.focus_window(handle);
                                self.core.focus_manager.record_focus(handle);
                                last_ffm_window = Some(handle);
                            }
                        }
                    }
                }
            }

            // Pump Windows messages for any HWNDs owned by this thread
            // (e.g., expose overlay windows). Without this, Windows will ghost them
            // as "Not Responding" after ~5 seconds of unprocessed messages.
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, PeekMessageW, TranslateMessage, PM_REMOVE,
                };
                let mut msg = std::mem::zeroed();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // Stackbar tab clicks are queued by the stackbar window procedure
            // during the message pump above; focus the clicked tab through the
            // same path as FocusGroupIndex.
            let stackbar_clicks = self.window_manager.poll_stackbar_clicks();
            if !stackbar_clicks.is_empty() {
                last_work = Some(std::time::Instant::now());
            }
            for (window, index) in stackbar_clicks {
                if let Err(error) = self.core.executor.focus_stackbar_tab(
                    &mut self.window_manager,
                    &mut self.core.monitor_manager,
                    window,
                    index,
                ) {
                    log::warn!("Stackbar tab click could not be honored: {}", error);
                }
            }

            pass_times.mark(std::time::Instant::now());
            // Readers see every configuration change made during this pass.
            self.publish_config_if_changed();
            // And the state, where something was told or done in it.
            if self.state_stale.get() {
                self.send_debug_overlay_data(None);
            }

            self.window_manager.tick_effects();

            // The bar came or went (or never came): its place is kept or
            // given to the windows, so they are laid out again.
            if last_bar_check.elapsed() >= std::time::Duration::from_secs(1) {
                last_bar_check = std::time::Instant::now();
                // The shell's panel is no window in the window manager's
                // lists: it is looked for by its class.
                self.core
                    .monitor_manager
                    .set_panel_seen(crate::dwm::window_manager::shell_panel_is_shown());
                let kept = self.core.monitor_manager.bar_is_kept(&self.window_manager);
                if kept != bar_kept {
                    bar_kept = kept;
                    log::info!(
                        "The bar's place is {}",
                        if kept {
                            "kept free"
                        } else {
                            "given to the windows (no bar is running)"
                        }
                    );
                    for monitor in self.core.monitor_manager.get_all_monitors() {
                        self.core.executor.mark_layout_dirty(monitor.handle);
                    }
                }
            }

            {
                let now = std::time::Instant::now();
                pass_times.mark(now);
                let (slow, summary) = pass_times.end(now);
                if log::log_enabled!(target: pass_times::LOG, log::Level::Debug) {
                    if let Some(pass) = slow {
                        log::debug!(
                            target: pass_times::LOG,
                            "a pass of {:.1} ms: {}",
                            pass.whole.as_secs_f64() * 1000.0,
                            pass.in_words()
                        );
                    }
                    if let Some(summary) = summary {
                        log::debug!(target: pass_times::LOG, "the last half minute: {summary}");
                    }
                }
            }

            // Wait for work. While something moves, and shortly after the
            // last work, that is one animation frame (default 60 Hz; the
            // tween math is wall-clock based, so the frame only sets the
            // tick granularity). Otherwise the loop sleeps until it is
            // woken or until it is time to look around again.
            let frame = std::time::Duration::from_millis(
                self.config.desktop.animations.frame_interval_ms() as u64,
            );
            // The night light on its way from one colour to another is
            // looked at every frame, not at the pace of its hours.
            if self.night_light.turning()
                && self.night_light.follow(
                    &self.config.desktop.night_light,
                    crate::night_light::minutes_now(),
                )
            {
                self.refresh_notices();
            }
            let activity = wait::Activity {
                animating: self.window_manager.active_animation_count() > 0
                    || self.night_light.turning()
                    || self.window_manager.effects_animate()
                    || self.display_change_pending.is_some()
                    || self.gesture_progress.is_some(),
                since_work: last_work.map(|at| at.elapsed()),
                watch_mouse_every: {
                    let follows = self.core.focus_manager.focus_follows_mouse().then(|| {
                        std::time::Duration::from_millis(
                            self.core.focus_manager.config().focus_delay_ms as u64,
                        )
                    });
                    // (The pointer is looked for in the corners, often
                    // enough for the moment it has to stay there.)
                    let corners = self
                        .config
                        .desktop
                        .hot_corners
                        .watched()
                        .then(|| std::time::Duration::from_millis(50));
                    match (follows, corners) {
                        (Some(one), Some(other)) => Some(one.min(other)),
                        (one, other) => one.or(other),
                    }
                },
            };
            wait::for_work(wait::interval(frame, activity), frame);
        }

        self.emit_runtime_event(RuntimeEventKind::RuntimeStopping, None, []);

        let restored_hidden = self
            .core
            .executor
            .restore_hidden_windows(&mut self.window_manager);
        if restored_hidden > 0 {
            log::info!(
                "Restored {} Gevrek-hidden window(s) before shutdown",
                restored_hidden
            );
        }
        let restored_composition = self.window_manager.restore_all_window_alpha_styles();
        // (The helper for elevated windows is let go once nothing more is
        // asked of it: at the very end of this.)
        // The title bars and their buttons gevrek took away are back.
        self.window_manager.restore_all_window_frames();
        // Windows that were cut at a display's edge are whole again.
        self.window_manager.cut_at_displays(&[]);
        // The displays have their own colours again.
        self.night_light.restore();
        // And the desktop the wallpaper it had.
        let mut wallpaper_desktop = if self.wallpaper.has_given() {
            crate::wallpaper::system::Desktop::open()
        } else {
            None
        };
        if let Some(desktop) = wallpaper_desktop.as_mut() {
            self.wallpaper.give_back(desktop);
        }
        // Windows moves the windows its own way again.
        self.window_manager.set_system_transitions_for_all(false);
        // And Windows its taskbar, and its key.
        crate::windows_key::configure(false, false);
        self.taskbar.restore();
        crate::dwm::monitor::set_taskbar_hidden(false);
        if restored_composition > 0 {
            log::info!(
                "Restored native composition for {} window(s) before shutdown",
                restored_composition
            );
        }
        self.startup_process_manager.shutdown();

        match self.window_manager.destroy_all_borders() {
            Ok(count) if count > 0 => log::info!(
                "Destroyed {} border overlay window(s) before shutdown",
                count
            ),
            Ok(_) => {}
            Err(e) => log::warn!(
                "Failed to destroy all border overlay windows during shutdown: {}",
                e
            ),
        }
        match self.window_manager.destroy_all_stackbars() {
            Ok(count) if count > 0 => {
                log::info!("Destroyed {} stackbar window(s) before shutdown", count)
            }
            Ok(_) => {}
            Err(e) => log::warn!("Failed to destroy stackbar windows during shutdown: {}", e),
        }

        // Save state on shutdown
        log::info!("Saving window manager state before exit...");
        if let Err(e) = self.save_state() {
            log::error!("Failed to save state on shutdown: {}", e);
        }

        // Last of all, since it takes seconds and whoever ends gevrek does
        // not wait for long: a colour or a slideshow is held for a moment
        // (the system may still finish a picture set shortly before).
        if let Some(desktop) = wallpaper_desktop.as_mut() {
            while self.wallpaper.holds() {
                std::thread::sleep(std::time::Duration::from_millis(250));
                self.wallpaper.hold(desktop);
            }
        }

        crate::dwm::hidden_journal::flush();
        // The helper for elevated windows ends with its window manager.
        crate::elevated_helper::client::disconnect();

        log::info!("Runtime event loop exited gracefully");
        Ok(())
    }

    /// Have the helper for elevated windows where the configuration asks
    /// for it and one is there, and none where it does not. When it
    /// comes, the windows that were left alone for want of it are taken;
    /// when it goes, they are let go again.
    fn follow_elevated_helper(&mut self) {
        use crate::elevated_helper::{client, pipe_name};
        // Whether an elevated program's window is told by its border.
        if self
            .window_manager
            .set_mark_elevated(self.config.desktop.platform.windows.mark_elevated_windows)
        {
            for monitor in self.core.monitor_manager.get_all_monitors() {
                self.core.executor.mark_layout_dirty(monitor.handle);
            }
        }
        let wanted = self.config.desktop.platform.windows.manage_elevated_windows
            && !crate::elevated::own_elevated();
        let had = self.elevated_helper;
        let pipe = pipe_name(&gevrek_model::paths::instance());
        let has = if wanted {
            // (The first time none answers, the task that starts one
            // without a question is run, where it was registered.)
            if !client::connect(&pipe) && !std::mem::replace(&mut self.elevated_helper_asked, true)
            {
                crate::elevated::run_helper_task(&gevrek_model::paths::instance());
            }
            client::connected()
        } else {
            if client::connected() {
                client::disconnect();
            }
            false
        };
        if has == had {
            return;
        }
        self.elevated_helper = has;
        if has {
            log::info!("Windows of programs that run as an administrator are managed (the helper answers)");
            if let Err(error) = self.window_manager.refresh_window_list() {
                log::debug!("the windows were not listed anew: {error}");
            }
            if let Err(error) = self.adopt_existing_windows() {
                log::warn!("the elevated windows that are open were not taken: {error}");
            }
        } else {
            log::info!("Windows of programs that run as an administrator are left alone again (no helper answers)");
            self.core
                .release_newly_ignored_windows(&mut self.window_manager);
        }
    }

    fn build_rule_engine(config: &DesktopConfig) -> RuleEngine {
        gevrek_engine::config_apply::build_rule_engine(config)
    }

    /// Whether the scrolling layout shows whole columns only, for the
    /// displays as they are now (`columns_must_be_whole`): said to the
    /// engine for the workspaces that are made from now on, and to those
    /// that are there, which are laid out again where it is another.
    fn follow_scrolling_room(&mut self) {
        let displays: Vec<(i32, i32, i32, i32)> = crate::dwm::monitor::all_displays()
            .into_iter()
            .map(|(_, info)| (info.x, info.y, info.width, info.height))
            .collect();
        // (Where columns are cut at a display's edge, they need not be
        // whole beside another display either.)
        let whole = crate::dwm::window_manager::columns_must_be_whole(&displays)
            && !self.config.desktop.platform.windows.cut_columns_at_displays;
        if whole == gevrek_engine::layouts::scrolling_whole_columns_only() {
            return;
        }
        log::info!(
            "The scrolling layout shows {} from now on",
            if whole {
                "whole columns only (a display lies beside another)"
            } else {
                "columns partly too (no display lies beside another)"
            }
        );
        gevrek_engine::layouts::set_scrolling_whole_columns_only(whole);
        for monitor in self.core.monitor_manager.get_all_monitors_ordered() {
            if let Some(workspaces) = self.core.monitor_manager.get_workspaces_mut(monitor.handle) {
                for workspace in workspaces.iter_mut() {
                    workspace.scroll_whole_columns = whole;
                }
            }
            self.core.executor.mark_layout_dirty(monitor.handle);
        }
    }

    /// Where the scrolling layout scrolls freely beside another display
    /// (`cut_columns_at_displays`), the windows of its columns in view
    /// are cut at their display's edges, as they stand now; every other
    /// window that was cut is whole again. Looked at with every pass:
    /// a window's shape has to follow it while it moves.
    fn cut_columns(&mut self) {
        let mut windows = Vec::new();
        if self.config.desktop.platform.windows.cut_columns_at_displays {
            for monitor in self.core.monitor_manager.get_all_monitors_ordered() {
                let Some(workspace) = self.core.monitor_manager.get_focused_workspace(monitor.handle)
                else {
                    continue;
                };
                if workspace.layout != gevrek_model::layout::LayoutType::Scrolling
                    || workspace.scroll_whole_columns
                {
                    continue;
                }
                let display = (
                    monitor.x,
                    monitor.y,
                    monitor.x + monitor.width,
                    monitor.y + monitor.height,
                );
                for container in workspace.containers.iter() {
                    if container.offscreen {
                        continue;
                    }
                    if let Some(window) = container.focused_window() {
                        windows.push((window, display));
                    }
                }
            }
        }
        self.window_manager.cut_at_displays(&windows);
    }

    /// The wallpaper `desktop.wallpaper` names is the system's to draw:
    /// it is given to the system, and what was there given back
    /// (`wallpaper.rs`). Nothing is asked of the system while the
    /// configuration names none and nothing was given.
    fn follow_wallpaper(&mut self) {
        if !self.wallpaper.has_work(&self.config.desktop.wallpaper) {
            return;
        }
        let Some(mut desktop) = crate::wallpaper::system::Desktop::open() else {
            return;
        };
        let known: Vec<(String, String, String)> = crate::dwm::monitor::all_displays()
            .into_iter()
            .map(|(device, info)| (device, info.name, info.device_path))
            .collect();
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        if self.wallpaper.follow(
            &self.config.desktop.wallpaper,
            &mut desktop,
            &|display| crate::wallpaper::names_of(display, &known),
            seconds,
        ) {
            self.refresh_notices();
        }
    }

    /// The accent the desktop has from the system's wallpaper, where the
    /// theme asks for that: when it is another than before, the borders
    /// are made again with it and the programs that follow it are told.
    fn follow_wallpaper_colour(&mut self) {
        let accent =
            if gevrek_model::theme::follows_wallpaper(&self.config.desktop.appearance.theme) {
                self.wallpaper_colour.look()
            } else {
                self.wallpaper_colour.forget();
                None
            };
        if accent == self.core.wallpaper_accent {
            return;
        }
        // As after a change of the configuration's appearance.
        let before = gevrek_engine::reload::border_snapshot(&self.core.monitor_manager);
        self.core.wallpaper_accent = accent;
        self.apply_appearance_profile();
        self.core
            .refresh_changed_borders(&mut self.window_manager, &before, true);
        self.send_debug_overlay_data(Some("accent".to_string()));
    }

    /// Make the displays what `desktop.outputs` asks for (their places,
    /// modes and turns: `crate::display_settings`). `Ok(None)`: they are so
    /// already, or no entry asks anything of them. What comes back puts
    /// them back as they were.
    fn apply_display_settings(&mut self) -> Result<Option<crate::display_settings::Saved>, String> {
        let displays = crate::display_settings::displays();
        let wanted = crate::display_settings::plan(&self.config.desktop.outputs, &displays)?;
        if wanted.is_empty() {
            return Ok(None);
        }
        for display in &wanted {
            log::info!(
                "Display {}: {}x{} at {} Hz at ({}, {}), {:?}{}",
                display.device,
                display.width,
                display.height,
                display.hertz,
                display.x,
                display.y,
                display.turned,
                if display.primary {
                    ", the main one"
                } else {
                    ""
                }
            );
        }
        let saved = crate::display_settings::set(&wanted)?;
        // Windows tells of the change as of any other: the monitors are
        // looked at again when it has settled.
        self.display_change_pending = Some(std::time::Instant::now());
        self.displays_own.push(saved.clone());
        Ok(Some(saved))
    }

    /// The displays' scales as `desktop.outputs` gives them
    /// (`display_scale.rs`): set where an entry gives one that its
    /// display has not, and given back where an entry gives none any
    /// more. One the display does not offer is said and nothing is set.
    fn apply_display_scales(&mut self) {
        let displays: Vec<_> = crate::display_settings::displays()
            .into_iter()
            .filter_map(|display| {
                let scale = crate::display_scale::now(&display.device)?;
                Some((display, scale))
            })
            .collect();
        let changes = match crate::display_scale::plan(
            &self.config.desktop.outputs,
            &displays,
            &self.scales_own,
        ) {
            Ok(changes) => changes,
            Err(why) => {
                log::warn!("desktop.outputs: {why}; the scales are left as they are");
                return;
            }
        };
        for change in changes {
            let had = displays
                .iter()
                .find(|(display, _)| display.device == change.device)
                .map(|(_, scale)| scale.current);
            match crate::display_scale::set(&change.device, change.steps) {
                Ok(()) => {
                    log::info!(
                        "Display {}: its scale is {} %{}",
                        change.name,
                        change.to,
                        if change.back { ", as it was before" } else { "" }
                    );
                    if change.back {
                        self.scales_own.remove(&change.device);
                    } else if let Some(had) = had {
                        // (What it had before the first change, not
                        // before this one.)
                        self.scales_own.entry(change.device.clone()).or_insert(had);
                    }
                    // Windows tells of the change as of any other: the
                    // monitors are looked at again when it has settled.
                    self.display_change_pending = Some(std::time::Instant::now());
                }
                Err(why) => log::warn!(
                    "Display {}: its scale could not be set to {} %: {why}",
                    change.name,
                    change.to
                ),
            }
        }
    }

    /// `desktop.outputs` asks nothing of the displays any more, and
    /// gevrek had changed them: they get the settings they had before.
    /// (Seen on the real displays, 2026-11-15: a mode on trial stayed
    /// when the studio put the old configuration back, and when its
    /// entry was taken out.)
    fn give_displays_back_when_nothing_is_asked(&mut self) {
        if self.displays_own.is_empty()
            || crate::display_settings::arranges_any(
                &self.config.desktop.outputs,
                &crate::display_settings::displays(),
            )
        {
            return;
        }
        log::info!("desktop.outputs asks nothing of the displays any more: they are as they were before");
        // The newest change first, back to the oldest.
        for saved in std::mem::take(&mut self.displays_own).iter().rev() {
            if !crate::display_settings::restore(saved) {
                log::warn!("A display did not take its settings from before back");
            }
        }
        self.display_change_pending = Some(std::time::Instant::now());
    }

    /// The displays as the configuration asks, when the window manager
    /// starts: without a trial (what the file says was kept before).
    fn apply_display_settings_at_start(&mut self) {
        match self.apply_display_settings() {
            Ok(_) => self.display_refused = None,
            Err(why) => {
                log::warn!("desktop.outputs: {why}; the displays are left as they are");
                self.display_refused = Some(why);
            }
        }
        self.apply_display_scales();
    }

    /// `desktop.outputs` changed from `old` to what the configuration has
    /// now: the displays are made so, and a change that could leave a dark
    /// screen is on trial.
    pub(super) fn outputs_changed(&mut self, old: &[gevrek_model::outputs::OutputProfile]) {
        let saved = match self.apply_display_settings() {
            Ok(saved) => {
                self.display_refused = None;
                saved
            }
            Err(why) => {
                log::warn!("desktop.outputs: {why}; the displays are left as they are");
                self.display_refused = Some(why);
                None
            }
        };
        self.notices_checked = None;
        self.give_displays_back_when_nothing_is_asked();
        self.apply_display_scales();
        // A change on top of one that is on trial: what counts is still
        // what was there before the first.
        let (before, mut kept) = match self.display_trial.take() {
            Some(trial) => (trial.previous, trial.saved),
            None => (old.to_vec(), Vec::new()),
        };
        kept.extend(saved);
        if kept.is_empty()
            || !gevrek_model::outputs::needs_trial(&before, &self.config.desktop.outputs)
        {
            return;
        }
        let length = display_trial_length();
        log::info!(
            "New display settings are on trial for {:.0} seconds: `gevrek output-trial keep` keeps them",
            length.as_secs_f64()
        );
        self.display_trial = Some(DisplayTrial {
            previous: before,
            saved: kept,
            until: std::time::Instant::now() + length,
        });
        self.emit_runtime_event(
            RuntimeEventKind::OutputTrial,
            None,
            [
                ("state".to_string(), "started".to_string()),
                (
                    "seconds".to_string(),
                    format!("{:.0}", length.as_secs_f64()),
                ),
            ],
        );
    }

    /// Keep the settings on trial, or put the ones from before back.
    /// `false` when nothing is on trial.
    fn answer_display_trial(&mut self, answer: gevrek_model::command::TrialAnswer) -> bool {
        let Some(trial) = self.display_trial.take() else {
            return false;
        };
        let state = match answer {
            gevrek_model::command::TrialAnswer::Keep => {
                log::info!("The new display settings are kept");
                "kept"
            }
            gevrek_model::command::TrialAnswer::Revert => {
                // The newest change first, back to the oldest.
                for saved in trial.saved.iter().rev() {
                    if !crate::display_settings::restore(saved) {
                        log::warn!("A display did not take its settings from before back");
                    }
                }
                self.config.desktop.outputs = trial.previous;
                self.config_dirty = true;
                if crate::dwm::monitor::set_unmanaged_outputs(&self.config.desktop.outputs) {
                    self.handle_display_change();
                }
                self.display_change_pending = Some(std::time::Instant::now());
                self.give_displays_back_when_nothing_is_asked();
                "reverted"
            }
        };
        self.emit_runtime_event(
            RuntimeEventKind::OutputTrial,
            None,
            [("state".to_string(), state.to_string())],
        );
        true
    }

    /// A window of gevrek's that was maximised (a floating one: a tiled
    /// one is put back in its tile) is given the room the windows have on
    /// its display, not the one Windows knows of: the panel's room is the
    /// window manager's to keep, no application bar asks Windows for it,
    /// and the window would lie under the panel. Windows gevrek does not
    /// manage are left where Windows puts them.
    fn keep_maximised_clear_of_panel(&mut self, window: crate::dwm::WindowHandle) {
        let Some((monitor, _)) = self.core.monitor_manager.find_window(window) else {
            return;
        };
        if !self.core.monitor_manager.bar_is_kept(&self.window_manager) {
            return;
        }
        let Some(room) = self
            .core
            .monitor_manager
            .get_adjusted_work_area(monitor, &self.window_manager)
        else {
            return;
        };
        if self.window_manager.keep_maximised_within(window, room) {
            log::debug!("A maximised window was given the room the panel leaves: {room:?}");
        }
    }

    /// Look for what there is to tell the person at the machine, and
    /// publish it when it changed. A notice that is new is also logged.
    fn refresh_notices(&mut self) {
        let mut notices = crate::conflicts::notices(&self.config.desktop);
        notices.extend(crate::idle::notice(&self.config.desktop.session));
        notices.extend(crate::night_light::notice(
            &self.config.desktop.night_light,
            self.night_light.impossible,
        ));
        notices.extend(self.wallpaper.notice());
        notices.extend(self.display_refused.as_ref().map(|why| {
            gevrek_engine::overlay_data::Notice {
                id: "display-settings-refused".to_string(),
                title: "The displays are not as desktop.outputs asks".to_string(),
                text: format!(
                    "Windows does not take what the configuration asks of the displays, and \
                 nothing was changed: {why}."
                ),
            }
        }));
        if notices == self.notices {
            return;
        }
        for notice in &notices {
            if !self.notices.iter().any(|known| known.id == notice.id) {
                log::warn!("{}. {}", notice.title, notice.text);
            }
        }
        let count = notices.len();
        self.notices = notices;
        self.send_debug_overlay_data(Some("Notices changed".to_string()));
        // Whoever shows them (the panel) asks for the state at once.
        self.emit_runtime_event(
            RuntimeEventKind::NoticesChanged,
            None,
            [("count".to_string(), count.to_string())],
        );
    }

    /// Re-enumerate monitors and carry workspaces and windows over to the
    /// new topology (see `MonitorManager::reconcile_monitors`).
    /// The modes each display offers and the one it is in, for whoever
    /// shows them (the studio's Displays page, `gevrek state`).
    fn tell_display_modes(&mut self) {
        for (device, info) in crate::dwm::monitor::all_displays() {
            if self.core.monitor_manager.get_monitor(info.handle).is_none() {
                continue;
            }
            let Some(now) = crate::display_settings::displays()
                .into_iter()
                .find(|display| display.device == device)
            else {
                continue;
            };
            // (As the display has them: a turned picture has its sides
            // the other way round.)
            let turned = matches!(
                now.turned,
                gevrek_model::outputs::OutputTransform::Left
                    | gevrek_model::outputs::OutputTransform::Right
            );
            let (width, height) = if turned {
                (now.height, now.width)
            } else {
                (now.width, now.height)
            };
            let modes = now
                .modes
                .iter()
                .map(
                    |(mode_width, mode_height, hertz)| gevrek_engine::outputs::OutputMode {
                        width: *mode_width as i32,
                        height: *mode_height as i32,
                        refresh_millihertz: *hertz as i32 * 1000,
                        preferred: false,
                        current: (*mode_width, *mode_height, *hertz) == (width, height, now.hertz),
                    },
                )
                .collect();
            self.core.monitor_manager.set_detail(
                info.handle,
                gevrek_engine::outputs::OutputDetail {
                    refresh_millihertz: now.hertz as i32 * 1000,
                    modes,
                },
            );
        }
    }

    fn handle_display_change(&mut self) {
        self.tell_display_modes();
        let changes = match self.core.monitor_manager.reconcile_monitors() {
            Ok(changes) => changes,
            Err(error) => {
                log::error!("Could not re-enumerate monitors: {}", error);
                return;
            }
        };
        if changes.is_empty() {
            log::debug!("Display notification without a monitor change");
            return;
        }
        log::info!(
            "Display configuration changed: {} remapped, {} attached, {} reattached, {} detached",
            changes.remapped.len(),
            changes.attached.len(),
            changes.reattached.len(),
            changes.detached.len()
        );
        self.follow_scrolling_room();
        // Profiles are keyed by display name/index, so they re-resolve
        // against the new topology (reserved areas, index preferences).
        Self::configure_monitor_profiles(&mut self.core.monitor_manager, &self.config.desktop);
        if !changes.attached.is_empty() {
            // The new display gets the workspaces the configuration has
            // for it and shows the one it is to start with; the displays
            // that were there go on showing what they show. (Only a
            // display the configuration gives nothing gets one made up.)
            self.apply_appearance_profile();
            self.apply_workspace_profile_set(|_| true, false);
            if let Err(error) = self.ensure_default_workspaces() {
                log::warn!("Could not create workspaces for a new monitor: {}", error);
            }
        }
        self.core.zone_manager =
            Self::build_zone_manager(&self.config.desktop.zones, &self.core.monitor_manager);
        self.core.executor.apply_monitor_changes(
            &mut self.window_manager,
            &self.core.monitor_manager,
            &changes,
        );
        self.send_debug_overlay_data(Some("Display configuration changed".to_string()));
    }

    fn build_zone_manager(
        settings: &ZoneSettings,
        monitor_manager: &MonitorManager,
    ) -> ZoneManager {
        gevrek_engine::config_apply::build_zone_manager(
            settings,
            monitor_manager,
            zone_modifier_held,
        )
    }

    fn configure_monitor_profiles(monitor_manager: &mut MonitorManager, config: &DesktopConfig) {
        gevrek_engine::config_apply::configure_monitor_profiles(monitor_manager, config);
        // The bar is a program of this backend: its place is kept here.
        monitor_manager.set_bar(Some(config.shell.panel.clone()));
        // (Windows measures a display in its own pixels: the panel is as
        // many of them high as the display's scale makes it.)
        monitor_manager.set_bar_in_display_pixels(true);
    }

    fn apply_workspace_profiles(&mut self) {
        self.apply_appearance_profile();
        self.apply_workspace_profile_set(|_| true, true);
    }

    /// Apply the enabled workspace profiles selected by `include`.
    /// `focus_default` switches each monitor to its `default` profile's
    /// workspace, which only startup should do.
    fn apply_workspace_profile_set(
        &mut self,
        include: impl Fn(&WorkspaceProfile) -> bool,
        focus_default: bool,
    ) {
        let created = self.core.apply_workspace_profile_set(
            &self.config.desktop.workspaces,
            &crate::system_look::appearance(&self.config.desktop.appearance),
            include,
            focus_default,
        );
        // The host tells listeners and starts the on-created-empty program.
        for workspace in created {
            self.emit_runtime_event(
                RuntimeEventKind::WorkspaceCreated,
                None,
                [
                    ("monitor".to_string(), workspace.monitor.0.to_string()),
                    ("name".to_string(), workspace.profile.name.clone()),
                    (
                        "persistent".to_string(),
                        workspace.profile.persistent.to_string(),
                    ),
                ],
            );
            self.run_workspace_created_empty_hook(workspace.monitor, &workspace.profile);
        }
    }

    fn run_workspace_created_empty_hook(
        &mut self,
        monitor: crate::dwm::MonitorHandle,
        profile: &WorkspaceProfile,
    ) {
        let Some(launch) = &profile.on_created_empty else {
            return;
        };
        let key = (monitor, profile.name.trim().to_ascii_lowercase());
        if !self.workspace_creation_hooks_fired.insert(key) {
            return;
        }
        match process_manager::spawn_launch(launch) {
            Ok(child) => log::info!(
                "Workspace '{}' created empty; launched process {}",
                profile.name,
                child.id()
            ),
            Err(error) => log::error!(
                "Workspace '{}' on-created-empty launch failed: {}",
                profile.name,
                error
            ),
        }
    }

    fn apply_appearance_profile(&mut self) {
        self.window_manager
            .set_window_effects(self.config.desktop.appearance.effects.clone());
        self.core
            .apply_workspace_appearance(&crate::system_look::appearance(
                &self.config.desktop.appearance,
            ));

        let handles = self
            .window_manager
            .get_all_windows()
            .iter()
            .map(|window| window.handle)
            .collect::<Vec<_>>();
        for handle in handles {
            self.apply_appearance_to_window(handle);
        }
        self.apply_focus_effects();
    }

    fn apply_appearance_to_window(&mut self, handle: crate::dwm::WindowHandle) {
        self.core.apply_appearance_to_window(
            &mut self.window_manager,
            &self.config.desktop,
            handle,
        );
    }

    fn apply_focus_effects(&mut self) {
        self.core
            .apply_focus_effects(&mut self.window_manager, &self.config.desktop);
    }

    /// Ensure each monitor has at least one workspace
    /// Creates a default BSP workspace for any monitor without workspaces
    fn ensure_default_workspaces(&mut self) -> Result<()> {
        self.core.ensure_default_workspaces();
        Ok(())
    }

    fn adopt_existing_windows(&mut self) -> Result<()> {
        let adopted = self.core.adopt_existing_windows(&mut self.window_manager)?;
        log::info!("Adopted {} existing windows at startup", adopted);
        Ok(())
    }

    /// The output whose visible workspace is about to lose its focused
    /// window, when `closed` (a window that is going away) is that window.
    ///
    /// Windows gives the foreground to another window as soon as the
    /// focused one is hidden on its way out, so the news of the new focus
    /// may arrive before the news of the closing: a window the focus left
    /// a moment ago (`focus_left`) counts as the one that had it.
    fn output_that_lost_its_focus(
        &self,
        closed: crate::dwm::WindowHandle,
        focus_left: Option<(std::time::Instant, Option<crate::dwm::WindowHandle>)>,
    ) -> Option<crate::dwm::MonitorHandle> {
        if !self.core.focus_manager.config().focus_next_on_close {
            return None;
        }
        let had_focus = self.core.focus_manager.current() == Some(closed)
            || focus_left.is_some_and(|(at, from)| {
                from == Some(closed) && at.elapsed() < FOCUS_LEFT_A_MOMENT_AGO
            });
        if !had_focus {
            return None;
        }
        let (output, index) = self.core.monitor_manager.find_window(closed)?;
        let visible = self
            .core
            .monitor_manager
            .get_workspaces(output)
            .is_some_and(|workspaces| workspaces.focused_idx() == index);
        visible.then_some(output)
    }

    /// The focused window of `output`'s visible workspace has closed: the
    /// window used before it there gets the focus, unless the system has
    /// already given it to a window of that workspace. Left to itself,
    /// Windows goes by its own order of windows, across displays.
    fn focus_successor_on(&mut self, output: crate::dwm::MonitorHandle) {
        let on_that_workspace = |runtime: &Self, window: crate::dwm::WindowHandle| {
            runtime
                .core
                .monitor_manager
                .find_window(window)
                .is_some_and(|(at, index)| {
                    at == output
                        && runtime
                            .core
                            .monitor_manager
                            .get_workspaces(output)
                            .is_some_and(|workspaces| workspaces.focused_idx() == index)
                })
        };
        if self
            .window_manager
            .get_active_window()
            .is_some_and(|foreground| on_that_workspace(self, foreground))
        {
            return;
        }
        let next = gevrek_engine::focus::successor(
            &self.core.monitor_manager,
            &self.core.executor.window_state,
            &self.core.focus_manager,
            output,
            |candidate| {
                self.window_manager.window_exists(candidate)
                    && self.window_manager.is_window_visible(candidate)
            },
        );
        let Some(next) = next else {
            return;
        };
        log::info!("The focused window closed: focus goes to {next:?} on its workspace");
        match self.window_manager.focus_window(next) {
            Ok(_) => {
                self.core.focus_manager.record_focus(next);
            }
            Err(error) => log::warn!("Could not focus {next:?} after a window closed: {error}"),
        }
    }

    /// Send authoritative all-workspace state to the debug overlay and IPC.
    fn send_debug_overlay_data(&mut self, last_event: Option<String>) {
        self.state_stale.set(false);
        if let Some(ref overlay_data_mutex) = self.debug_overlay_data {
            let (monitor_data, total_windows) = self
                .core
                .monitor_data(&self.window_manager, &self.config.desktop);

            for (kind, fields) in
                gevrek_engine::overlay_data::changes(&self.published_monitors, &monitor_data)
            {
                self.emit_runtime_event(kind, None, fields);
            }
            self.published_monitors = monitor_data.clone();
            let overlay_data = OverlayData {
                timestamp: std::time::SystemTime::now(),
                monitors: monitor_data,
                graph_node_count: self.config.node_graph.nodes.len(),
                total_windows,
                event_loop_fps: self.event_loop_fps,
                last_event,
                active_animations: self.window_manager.active_animation_count(),
                active_binding_submap: self.active_binding_submap.clone(),
                tiling_suspended: self.core.executor.tiling_suspended(),
                gesture: {
                    let backend = self.hotkey_manager.gesture_backend_status();
                    crate::dwm::GestureStateData {
                        available: backend.available,
                        backend: backend.backend,
                        device_count: backend.device_count,
                        configured_count: self
                            .config
                            .desktop
                            .gestures
                            .profiles
                            .iter()
                            .filter(|profile| {
                                self.config.desktop.gestures.enabled && profile.enabled
                            })
                            .count() as u32,
                        active: self.gesture_progress.clone(),
                        last_error: backend.last_error,
                    }
                },
                rule_states: self
                    .rule_engine
                    .lock()
                    .map(|engine| {
                        engine
                            .rule_states()
                            .into_iter()
                            .map(|(name, enabled, dynamic)| crate::dwm::RuleStateData {
                                name,
                                enabled,
                                dynamic,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                scratchpad_states: self.core.scratchpad_data(),
                startup_states: self
                    .startup_process_manager
                    .states()
                    .into_iter()
                    .map(|state| crate::dwm::StartupStateData {
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
                metrics: None, // Performance metrics can be added later
                notices: self.notices.clone(),
                // (The system draws the wallpaper here; where the theme
                // asks for its colour, it is read from the system's.)
                accent: self.core.wallpaper_accent,
                keyboard_layout: None,
                // (Windows' own compositor shows the frames.)
                frames: Vec::new(),
            };

            // Write to shared state (handle poisoned mutex gracefully)
            match overlay_data_mutex.lock() {
                Ok(mut data) => {
                    *data = Some(overlay_data);
                }
                Err(poisoned) => {
                    // Recover from poisoned mutex
                    let mut data = poisoned.into_inner();
                    *data = Some(overlay_data);
                }
            }
        }
    }

    /// The command of the hot corner the pointer has been in for long
    /// enough, if one is due now.
    fn hot_corner_due(&mut self) -> Option<DispatchCommand> {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
        };
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut point = POINT::default();
        // SAFETY: plain questions with a valid place for the answer.
        let held = unsafe {
            GetCursorPos(&mut point).ok()?;
            [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
                .iter()
                .any(|button| GetAsyncKeyState(button.0 as i32) < 0)
        };
        let displays: Vec<gevrek_model::geometry::Rect> = self
            .core
            .monitor_manager
            .get_all_monitors()
            .iter()
            .map(|monitor| {
                gevrek_model::geometry::Rect::new(
                    monitor.x,
                    monitor.y,
                    monitor.width,
                    monitor.height,
                )
            })
            .collect();
        let command = self.hot_corners.step(
            &self.config.desktop.hot_corners,
            &displays,
            (point.x, point.y),
            held,
            std::time::Instant::now(),
        )?;
        // Not over a program that fills the display (a game, a film): a
        // pointer in its corner means something there.
        let fills = gevrek_engine::contract::WindowQueries::active_window(&self.window_manager)
            .is_some_and(|window| {
                gevrek_engine::contract::WindowQueries::is_fullscreen(&self.window_manager, window)
            });
        (!fills).then_some(command)
    }

    /// Get the window under the current cursor position (for focus-follows-mouse).
    fn get_window_under_cursor() -> Option<crate::dwm::WindowHandle> {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, WindowFromPoint};

        unsafe {
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt).is_ok() {
                let hwnd = WindowFromPoint(pt);
                if !hwnd.is_invalid() {
                    // Walk up to the top-level owner to avoid focusing child controls
                    use windows::Win32::UI::WindowsAndMessaging::GetAncestor;
                    use windows::Win32::UI::WindowsAndMessaging::GA_ROOT;
                    let root = GetAncestor(hwnd, GA_ROOT);
                    let target = if root.is_invalid() { hwnd } else { root };
                    return Some(crate::dwm::WindowHandle::from_hwnd(target));
                }
            }
        }
        None
    }

    /// Register all OnKeyPress nodes as hotkeys
    fn register_hotkeys(&mut self) -> Result<()> {
        if self.active_binding_submap != "default"
            && !self.config.desktop.bindings.iter().any(|binding| {
                binding.enabled && binding.normalized_submap() == self.active_binding_submap
            })
        {
            log::warn!(
                "Binding submap '{}' no longer exists; returning to default",
                self.active_binding_submap
            );
            self.active_binding_submap = "default".to_string();
        }
        log::info!(
            "Registering graph inputs and declarative bindings for submap '{}'",
            self.active_binding_submap
        );

        for id in self.registered_hotkey_ids.drain(..) {
            if let Err(e) = self.hotkey_manager.unregister(id) {
                log::warn!("Failed to unregister previous hotkey {}: {}", id, e);
            }
        }

        // Collect hotkeys first to avoid borrow issues
        let mut hotkeys_to_register = Vec::new();
        for (graph_index, (node_id, node)) in
            self.core.executor.get_graph().nodes.iter().enumerate()
        {
            if let crate::node_editor::NodeType::OnKeyPress {
                key,
                ctrl,
                alt,
                shift,
                win,
            } = &node.node_type
            {
                if !key.is_empty() {
                    let hotkey = Hotkey::new(key.clone(), *ctrl, *alt, *shift, *win);
                    // Negative IDs are reserved for global automation-graph
                    // inputs; declarative binding IDs start at 1,000,000.
                    let id = (-1i32).saturating_sub(graph_index as i32);
                    hotkeys_to_register.push((
                        hotkey,
                        id,
                        key.clone(),
                        *ctrl,
                        *alt,
                        *shift,
                        *win,
                        *node_id,
                    ));
                }
            }
        }

        // Now register them
        let mut hotkey_count = 0;
        for (hotkey, id, key, ctrl, alt, shift, win, node_id) in hotkeys_to_register {
            if let Err(e) = self.hotkey_manager.register(hotkey, id) {
                log::warn!("Failed to register hotkey for node {}: {}", node_id, e);
            } else {
                log::info!(
                    "Registered hotkey: key={}, ctrl={}, alt={}, shift={}, win={} (node_id={})",
                    key,
                    ctrl,
                    alt,
                    shift,
                    win,
                    node_id
                );
                self.registered_hotkey_ids.push(id);
                hotkey_count += 1;
            }
        }

        for (index, binding) in self.config.desktop.bindings.iter().enumerate() {
            if !binding.enabled
                || binding.key.trim().is_empty()
                || binding.normalized_submap() != self.active_binding_submap
            {
                continue;
            }
            let id = 1_000_000i32.saturating_add(index as i32);
            let hotkey = Hotkey {
                key: binding.key.clone(),
                ctrl: binding.ctrl,
                alt: binding.alt,
                shift: binding.shift,
                win: binding.win,
                device: binding.device,
                trigger: binding.trigger,
                consume: binding.consume,
                long_press_ms: binding.long_press_ms,
            };
            if let Err(error) = self.hotkey_manager.register(hotkey, id) {
                log::warn!(
                    "Failed to register declarative binding '{}': {}",
                    binding.name,
                    error
                );
            } else {
                self.registered_hotkey_ids.push(id);
                hotkey_count += 1;
            }
        }

        let gesture_registrations = if self.config.desktop.gestures.enabled {
            self.config
                .desktop
                .gestures
                .profiles
                .iter()
                .enumerate()
                .filter(|(_, gesture)| gesture.enabled)
                .map(|(index, gesture)| GestureRegistration {
                    id: 2_000_000i32.saturating_add(index as i32),
                    fingers: gesture.fingers,
                    direction: gesture.direction,
                    distance_percent: gesture.distance_percent,
                })
                .collect()
        } else {
            Vec::new()
        };
        self.hotkey_manager
            .configure_gestures(
                gesture_registrations,
                self.config.desktop.gestures.idle_timeout_ms,
                self.config.desktop.gestures.axis_lock_ratio,
            )
            .map_err(anyhow::Error::msg)?;

        log::info!("Registered {} hotkeys", hotkey_count);
        Ok(())
    }

    fn process_gesture_event(&mut self, event: GestureEvent) {
        let direction = event.direction.map(|direction| direction.config_name());
        let kind = match event.phase {
            GesturePhase::Started => RuntimeEventKind::GestureStarted,
            GesturePhase::Updated => RuntimeEventKind::GestureUpdated,
            GesturePhase::Completed => RuntimeEventKind::GestureCompleted,
            GesturePhase::Cancelled => RuntimeEventKind::GestureCancelled,
        };
        self.gesture_progress = Some(crate::dwm::GestureProgressData {
            phase: event.phase.config_name().to_string(),
            fingers: event.fingers,
            direction: direction.map(str::to_string),
            progress: event.progress,
            delta_x: event.delta_x,
            delta_y: event.delta_y,
        });
        self.gesture_progress_updated = Some(std::time::Instant::now());
        self.emit_runtime_event(
            kind,
            None,
            [
                ("fingers".to_string(), event.fingers.to_string()),
                (
                    "direction".to_string(),
                    direction.unwrap_or("pending").to_string(),
                ),
                ("progress".to_string(), format!("{:.4}", event.progress)),
                ("delta_x".to_string(), format!("{:.4}", event.delta_x)),
                ("delta_y".to_string(), format!("{:.4}", event.delta_y)),
            ],
        );
        log::debug!(
            "Gesture {}: {} finger{} {} {:.0}%",
            event.phase.config_name(),
            event.fingers,
            if event.fingers == 1 { "" } else { "s" },
            direction.unwrap_or("pending"),
            event.progress * 100.0,
        );
        if event.phase == GesturePhase::Completed {
            let commands = event
                .registration_ids
                .iter()
                .filter_map(|id| {
                    let index = id.checked_sub(2_000_000)? as usize;
                    self.config
                        .desktop
                        .gestures
                        .profiles
                        .get(index)
                        .map(|gesture| gesture.command.clone())
                })
                .collect::<Vec<_>>();
            for command in commands {
                if let Err(error) = self.execute_runtime_dispatch(command) {
                    log::error!("Error executing gesture action: {error}");
                }
            }
        }
        self.send_debug_overlay_data(Some(format!(
            "Gesture {}: {} finger{} {}",
            event.phase.config_name(),
            event.fingers,
            if event.fingers == 1 { "" } else { "s" },
            direction.unwrap_or("pending"),
        )));
    }

    fn execute_runtime_dispatch(&mut self, command: DispatchCommand) -> Result<()> {
        self.state_stale.set(true);
        let command_name = command.display_name().to_string();
        let command_json = serde_json::to_string(&command).unwrap_or_else(|_| command_name.clone());
        let result = match command {
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
                if self.active_binding_submap != name {
                    self.active_binding_submap = name;
                    self.register_hotkeys()?;
                    log::info!(
                        "Active binding submap is now '{}'",
                        self.active_binding_submap
                    );
                    // Told at once: who shows the keys' mode (the panel)
                    // asks for the state as soon as the command is done.
                    self.send_debug_overlay_data(Some("keys' mode".to_string()));
                }
                Ok(())
            }
            DispatchCommand::ResetBindingSubmap => {
                if self.active_binding_submap != "default" {
                    self.active_binding_submap = "default".to_string();
                    self.register_hotkeys()?;
                    log::info!("Returned to the default binding submap");
                    self.send_debug_overlay_data(Some("keys' mode".to_string()));
                }
                Ok(())
            }
            DispatchCommand::Quit => {
                log::info!("Quit: the window manager stops");
                *self
                    .stop_flag
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
                Ok(())
            }
            // (Where a new window opens is Windows' own to say.)
            DispatchCommand::ExpectWindow { .. } => Ok(()),
            DispatchCommand::PickWindow => {
                crate::pick::toggle();
                Ok(())
            }
            // (Windows shows only the windows of one workspace: the
            // others are put away where nothing can draw them.)
            // The shell draws it on Windows (a window that is put away
            // is drawn by nobody here: the shell remembers pictures).
            DispatchCommand::WorkspaceOverview => {
                if crate::conflicts::shell_runs() {
                    self.emit_runtime_event(
                        RuntimeEventKind::ShellCommand,
                        None,
                        [(
                            "action".to_string(),
                            gevrek_model::command::SHELL_WORKSPACE_OVERVIEW.to_string(),
                        )],
                    );
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "The overview of every workspace is gevrek-shell's on Windows, and it does not run"
                    ))
                }
            }
            // The system owns the keyboard layouts and the keys here.
            DispatchCommand::KeyboardLayout { .. } => Err(anyhow::anyhow!(
                "Keyboard layouts are switched by Windows itself (Win+Space)"
            )),
            DispatchCommand::ShortcutsInhibit => {
                Err(anyhow::anyhow!("No program takes gevrek's keys on Windows"))
            }
            DispatchCommand::OutputTrial { answer } => {
                if self.answer_display_trial(answer) {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("No display settings are on trial"))
                }
            }
            DispatchCommand::Shell { action } => {
                self.emit_runtime_event(
                    RuntimeEventKind::ShellCommand,
                    None,
                    [("action".to_string(), action.id())],
                );
                // The night light is the window manager's: its key
                // switches it on or off (within its hours, when the
                // configuration gives it hours), the configuration says so
                // afterwards, and it is said on the screen: outside its
                // hours nothing turns warm yet, and that must not look
                // like a key that does nothing.
                if action == gevrek_model::command::ShellAction::NightLight {
                    let on = !self.config.desktop.night_light.enabled;
                    self.config.desktop.night_light.enabled = on;
                    self.config_dirty = true;
                    // (Looked at with the next pass, not in two seconds.)
                    self.wallpaper_looked = None;
                    log::info!(
                        "The night light is switched {}",
                        if on { "on" } else { "off" }
                    );
                    let said = DispatchCommand::Osd {
                        text: self
                            .config
                            .desktop
                            .night_light
                            .said_when_switched(crate::night_light::minutes_now()),
                        level: None,
                    };
                    self.emit_runtime_event(
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
                self.night_light.preview(temperature, seconds);
                // (Looked at with the next pass.)
                self.wallpaper_looked = None;
                Ok(())
            }
            // What the shell shows for someone else: told as its keys are.
            command @ (DispatchCommand::Notify { .. }
            | DispatchCommand::Osd { .. }
            | DispatchCommand::AppShortcut { .. }) => {
                self.emit_runtime_event(
                    RuntimeEventKind::ShellCommand,
                    None,
                    command.shell_fields().unwrap_or_default(),
                );
                Ok(())
            }
            // The shell shows the list where it runs (a surface, no window
            // to keep floating); the settings program's window is for when
            // it does not, and the same key closes that one while it is open.
            DispatchCommand::ShowShortcuts => {
                if !gevrek_host::shortcuts::is_open() && crate::conflicts::shell_runs() {
                    self.emit_runtime_event(
                        RuntimeEventKind::ShellCommand,
                        None,
                        [(
                            "action".to_string(),
                            gevrek_model::command::SHELL_SHORTCUTS.to_string(),
                        )],
                    );
                    Ok(())
                } else {
                    gevrek_host::shortcuts::toggle()
                }
            }
            DispatchCommand::Exec { program, args } => {
                let launch = gevrek_model::settings::LaunchProfile {
                    program,
                    args,
                    ..Default::default()
                };
                gevrek_engine::Desktop::launch(&mut self.window_manager, &launch).map(|pid| {
                    log::info!("Started '{}' as PID {pid}", launch.program);
                })
            }
            DispatchCommand::LaunchStartup { id } => self.startup_process_manager.launch(&id),
            DispatchCommand::StopStartup { id } => self.startup_process_manager.stop(&id),
            DispatchCommand::RestartStartup { id } => self.startup_process_manager.restart(&id),
            DispatchCommand::ToggleMouseFollowsFocus => {
                let enabled = !self.core.focus_manager.mouse_follows_focus();
                self.core.focus_manager.set_mouse_follows_focus(enabled);
                // Mirror the toggle into the live config so a later hot
                // reload (which re-applies `desktop.focus`) does not revert
                // it; the flag is also serialized with the config document.
                self.config.desktop.focus.mouse_follows_focus = enabled;
                self.config_dirty = true;
                log::info!(
                    "Mouse-follows-focus {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                Ok(())
            }
            DispatchCommand::WatchConfiguration { enabled } => {
                if let Some(watcher) = &mut self.config_watcher {
                    watcher.set_enabled(enabled);
                } else {
                    log::warn!(
                        "Configuration watch toggled but the runtime has no config file path"
                    );
                }
                // Mirror the toggle into the live config like
                // mouse-follows-focus so a later hot reload does not revert
                // it; the flag is also serialized with the config document.
                self.config.desktop.watch_configuration = enabled;
                self.config_dirty = true;
                log::info!(
                    "Configuration file watch {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                Ok(())
            }
            DispatchCommand::ToggleStackbar => {
                // Runtime-owned on/off toggle,
                // mirrored into the live config like mouse-follows-focus so a
                // later hot reload does not revert it.
                let enabled = !self.config.desktop.appearance.stackbar.enabled;
                self.config.desktop.appearance.stackbar.enabled = enabled;
                self.config_dirty = true;
                self.apply_appearance_profile();
                // Re-layout every monitor so container rects reserve (or
                // release) the stackbar strip; the layout pass also creates
                // or destroys the stackbar windows themselves.
                for monitor in self.core.monitor_manager.get_all_monitors_ordered() {
                    self.core.executor.mark_layout_dirty(monitor.handle);
                }
                self.core.flush_pending_layouts(&mut self.window_manager)?;
                log::info!("Stackbar {}", if enabled { "enabled" } else { "disabled" });
                Ok(())
            }
            DispatchCommand::ToggleTransparency => {
                // Runtime-owned on/off
                // toggle, mirrored into the live config like
                // mouse-follows-focus so a later hot reload does not revert
                // it. When off, every window renders fully opaque.
                let enabled = !self.config.desktop.appearance.transparency_enabled;
                self.config.desktop.appearance.transparency_enabled = enabled;
                self.config_dirty = true;
                self.apply_focus_effects();
                log::info!(
                    "Transparency {}",
                    if enabled { "enabled" } else { "disabled" }
                );
                Ok(())
            }
            DispatchCommand::SetTransparencyAlpha { alpha } => {
                // Set the inactive-window
                // alpha at runtime, mirrored into the live config.
                let alpha = alpha.max(20);
                self.config.desktop.appearance.inactive_window_opacity = alpha;
                self.config_dirty = true;
                self.apply_focus_effects();
                log::info!("Transparency alpha set to {alpha}");
                Ok(())
            }
            DispatchCommand::SetGlobalWorkAreaOffset {
                left,
                top,
                right,
                bottom,
            } => {
                // Mirrored into the live config
                // and applied to every monitor after the Windows work area
                // and monitor reservations.
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
                // Re-layout every monitor so the new offset takes effect.
                for monitor in self.core.monitor_manager.get_all_monitors_ordered() {
                    self.core.executor.mark_layout_dirty(monitor.handle);
                }
                self.core.flush_pending_layouts(&mut self.window_manager)?;
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
                let result = self.core.dispatch(&mut self.window_manager, command);
                if result.is_ok() {
                    self.apply_focus_effects();
                }
                result
            }
        };
        let mut fields = vec![
            ("command".to_string(), command_name),
            ("payload".to_string(), command_json),
        ];
        let kind = match &result {
            Ok(()) => RuntimeEventKind::DispatchSucceeded,
            Err(error) => {
                fields.push(("error".to_string(), error.to_string()));
                RuntimeEventKind::DispatchFailed
            }
        };
        self.emit_runtime_event(kind, None, fields);
        result
    }

    /// Save current window manager state to disk for restoration on next startup.
    fn save_state(&self) -> Result<()> {
        let state = self.core.session_state(&self.window_manager);
        self.persistence.save_state(&state)?;
        log::info!("Saved {} windows to state file", state.windows.len());
        Ok(())
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::{effective_window_alpha, run_dispatch_batch, RuntimeEventKind};

    #[test]
    fn focus_opacity_composes_with_rule_opacity() {
        assert_eq!(
            effective_window_alpha(true, 230, false, true, 200, false, false),
            200
        );
        assert_eq!(
            effective_window_alpha(true, 180, false, true, 200, false, false),
            180
        );
        assert_eq!(
            effective_window_alpha(true, 230, true, true, 200, false, false),
            230
        );
    }

    #[test]
    fn fullscreen_dimming_is_explicit() {
        assert_eq!(
            effective_window_alpha(true, 255, false, true, 180, true, false),
            255
        );
        assert_eq!(
            effective_window_alpha(true, 255, false, true, 180, true, true),
            180
        );
    }

    #[test]
    fn disabled_transparency_forces_full_opacity() {
        // Opaque regardless of the
        // configured base, dimming, or focus state.
        assert_eq!(
            effective_window_alpha(false, 230, false, true, 200, false, false),
            255
        );
        assert_eq!(
            effective_window_alpha(false, 180, true, false, 200, false, false),
            255
        );
        assert_eq!(
            effective_window_alpha(false, 255, false, true, 180, true, true),
            255
        );
    }

    #[test]
    fn dispatch_batch_stop_policy_is_deterministic() {
        let commands = [1, 2, 3];
        let stopped = run_dispatch_batch(&commands, true, |command| {
            if *command == 2 {
                Err("failed".to_string())
            } else {
                Ok(())
            }
        });
        assert!(stopped[0].success);
        assert_eq!(stopped[1].error.as_deref(), Some("failed"));
        assert!(stopped[2].skipped);

        let continued = run_dispatch_batch(&commands, false, |command| {
            if *command == 2 {
                Err("failed".to_string())
            } else {
                Ok(())
            }
        });
        assert!(continued[2].success);
        assert!(!continued[2].skipped);
    }

    #[test]
    fn extension_event_names_are_unique_and_namespaced() {
        let kinds = RuntimeEventKind::ALL;
        let names = kinds
            .into_iter()
            .map(RuntimeEventKind::as_str)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(names.len(), kinds.len());
        assert!(names.iter().all(|name| name.contains('.')));
    }
}
