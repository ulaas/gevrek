//! Test support: a runtime on one test monitor without OS hooks, and real
//! (never shown) windows. Used by runtime-level tests (reload, behaviour
//! characterization).

use super::Runtime;
use crate::config::Config;
use crate::dwm::HwndExt;
use crate::dwm::{
    FocusManager, MonitorHandle, MonitorManager, WindowHandle, WindowManager, WindowMonitor,
};
use crate::hotkey::HotkeyManager;
use crate::persistence::PersistenceManager;
use crate::runtime::executor::Executor;
use std::collections::HashSet;
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex};

pub(crate) const MONITOR: MonitorHandle = MonitorHandle(1);

/// The journal of hidden windows is one for the whole process, and the
/// tests run side by side in it: a test that shows everything the journal
/// names would show the windows another test has just hidden (and count
/// them), and that other test would find its window shown. Tests with real
/// windows share the journal; the one that empties it has it alone.
static JOURNAL_TURN: std::sync::RwLock<()> = std::sync::RwLock::new(());

thread_local! {
    /// Held for as long as the test's thread lives.
    static SHARES_JOURNAL: std::cell::RefCell<Option<std::sync::RwLockReadGuard<'static, ()>>> =
        const { std::cell::RefCell::new(None) };
}

/// This test hides and shows real windows.
fn shares_the_journal() {
    SHARES_JOURNAL.with(|held| {
        let mut held = held.borrow_mut();
        if held.is_none() {
            *held = Some(
                JOURNAL_TURN
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
        }
    });
}

/// This test shows whatever the journal names: no other test's windows
/// are in it while the answer is held.
pub(crate) fn alone_with_the_journal() -> std::sync::RwLockWriteGuard<'static, ()> {
    JOURNAL_TURN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A real, never-shown window (reload drops windows that do not exist).
pub(crate) struct TestWindow(pub(crate) WindowHandle);

impl TestWindow {
    pub(crate) fn create() -> Self {
        shares_the_journal();
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("STATIC"),
                windows::core::w!("gevrek reload test"),
                // A program's own window, as Windows makes one: with a
                // title bar alone it would be a dialog, which floats
                // (`window_kind`).
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                10,
                10,
                None,
                None,
                None,
                None,
            )
        }
        .expect("create test window");
        Self(WindowHandle(hwnd.0 as isize))
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0.to_hwnd());
        }
    }
}

/// A runtime on one test monitor, without OS hooks.
///
/// It is never dropped (`ManuallyDrop`), not even when a test panics:
/// `Drop for Runtime` runs the emergency restore over the process-wide hidden
/// journal, which would synchronously show windows that other tests created
/// on their own (non-pumping) threads and hang the test process.
pub(crate) fn test_runtime(config: Config) -> ManuallyDrop<Runtime> {
    let rule_engine = Arc::new(Mutex::new(Runtime::build_rule_engine(&config.desktop)));
    let mut monitor_manager = MonitorManager::with_workspace_count(0);
    monitor_manager.add_test_monitor(MONITOR, "Primary");
    let log_entries = Arc::new(Mutex::new(Vec::new()));
    let mut executor = Executor::new_with_events(
        config.node_graph.clone(),
        log_entries.clone(),
        None,
        None,
        rule_engine.clone(),
    );
    executor.set_scratchpad_profiles(config.desktop.scratchpads.clone());
    executor.set_workspace_profiles(
        config.desktop.workspaces.clone(),
        config.desktop.appearance.clone(),
    );
    let zone_manager = Runtime::build_zone_manager(&config.desktop.zones, &monitor_manager);
    let (_config_sender, config_receiver) = std::sync::mpsc::channel();
    let (_command_sender, manual_command_receiver) = std::sync::mpsc::channel();
    ManuallyDrop::new(Runtime {
        window_manager: WindowManager::new_with_rules(rule_engine.clone()),
        window_monitor: WindowMonitor::inert(),
        core: super::core::WmCore::new(
            executor,
            monitor_manager,
            FocusManager::with_config(config.desktop.focus.clone()),
            zone_manager,
        ),
        hotkey_manager: HotkeyManager::inert(),
        registered_hotkey_ids: Vec::new(),
        active_binding_submap: "default".to_string(),
        rule_engine,
        startup_process_manager: super::process_manager::StartupProcessManager::new(),
        workspace_creation_hooks_fired: HashSet::new(),
        persistence: PersistenceManager::new(),
        config_watcher: None,
        debug_overlay_data: None,
        stop_flag: Arc::new(Mutex::new(false)),
        pause_flag: Arc::new(Mutex::new(false)),
        config_receiver,
        manual_command_receiver,
        node_event_broadcast: None,
        event_loop_fps: 0.0,
        gesture_progress: None,
        published_monitors: Vec::new(),
        state_stale: std::cell::Cell::new(false),
        hot_corners: Default::default(),
        gesture_progress_updated: None,
        display_change_pending: None,
        held_openings: Default::default(),
        elevated_helper: false,
        elevated_helper_asked: false,
        config_publisher: None,
        config_dirty: false,
        notices: Vec::new(),
        // Tests do not look for other programs on the desktop they run on.
        notices_checked: Some(std::time::Instant::now()),
        wallpaper_colour: Default::default(),
        // (Nor at the wallpaper of that desktop.)
        wallpaper_looked: Some(std::time::Instant::now()),
        night_light: Default::default(),
        // (Nor is that desktop's wallpaper anybody's to change.)
        wallpaper: crate::wallpaper::Wallpaper::nowhere(),
        display_trial: None,
        taskbar: Default::default(),
        transitions_off: false,
        display_refused: None,
        displays_own: Vec::new(),
        scales_own: std::collections::HashMap::new(),
        config,
    })
}

/// Run the graph's startup automation, as `Runtime::run` does at startup.
pub(crate) fn run_startup(runtime: &mut Runtime) {
    runtime
        .core
        .executor
        .execute_startup_workspaces_only(
            &mut runtime.window_manager,
            &mut runtime.core.monitor_manager,
        )
        .unwrap();
}

/// End of a test. The runtime is leaked rather than dropped (see
/// `test_runtime`); this marks the end explicitly in each test.
pub(crate) fn finish(runtime: ManuallyDrop<Runtime>) {
    let _ = runtime;
}
