//! The window manager on macOS: the loop around the core.
//!
//! It runs on the main thread (AppKit's screens are read there, and the
//! observers' events come on its run loop). Each pass runs the run loop a
//! moment (the observers' events and the keys come then), hands what
//! happened to the core as window events, and runs the host every backend
//! shares (`gevrek_host::wm`): the server's commands and configuration,
//! the state it publishes.
//!
//! Programs that start and end are found by looking at the list of
//! programs twice a second; windows the observers missed (one that opened
//! before its program was watched) by looking at the window server's list
//! every second.

use crate::apps::{self, App};
use crate::ax::Element;
use crate::desktop::{handle_of, MacDesktop};
use crate::observer::{self, AxEvent, AxEventKind, Observer};
use gevrek_engine::core::WmCore;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::executor::{
    Executor, LogBroadcast, NodeEventBroadcast, RuntimeEventKind,
};
use gevrek_engine::focus_manager::FocusManager;
use gevrek_engine::input::Hotkey;
use gevrek_engine::outputs::Outputs;
use gevrek_engine::rules::RuleEngine;
use gevrek_engine::contract::WindowQueries as _;
use gevrek_engine::{Desktop, WindowHandle};
use gevrek_host::wm::bindings::Bindings as _;
use gevrek_host::wm::commands::Commands as _;
use gevrek_host::wm::configuration::Configuration as _;
use gevrek_host::wm::published::{Published as _, SystemState};
use gevrek_host::wm::reload::Reload as _;
use gevrek_host::wm::{HostLinks, HostState, Parts, WmHost};
use gevrek_model::config::Config;
use objc2::MainThreadMarker;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the run loop runs in one pass: the events that come meanwhile
/// are handled at once, so this is only how often the host looks at the
/// server when nothing happens.
const PASS: Duration = Duration::from_millis(16);

/// The macOS window manager.
pub struct MacRuntime {
    config: Config,
    core: WmCore,
    desktop: MacDesktop,
    host: HostState,
    rules: Arc<Mutex<RuleEngine>>,
    mtm: MainThreadMarker,
    events: Receiver<AxEvent>,
    sender: Sender<AxEvent>,
    /// The programs watched, by process id.
    observers: HashMap<i32, Observer>,
    /// What the keyboard tap shares with the loop: the live hotkeys, and
    /// the keys that hit them.
    keys: std::rc::Rc<std::cell::RefCell<crate::keys::Shared>>,
    /// The keyboard tap, once it is installed.
    tap: Option<crate::keys::KeyTap>,
    /// A window being moved or resized with the mouse.
    drag: Option<Drag>,
    /// The touchpads, listened to while a gesture profile is on.
    touchpad: Option<crate::touchpad::Touchpad>,
    /// What the touchpads' fingers make: the configuration's gestures.
    gestures: gevrek_engine::gesture::GestureRecognizer,
    looked_at_programs: Instant,
    looked_at_windows: Instant,
    /// Passes of the loop a second, measured.
    passes_per_second: f64,
    running: bool,
    /// The desktop is being read at the start: windows found are only
    /// known and watched (no workspace exists yet); adopting them comes
    /// after the configuration made the workspaces.
    starting: bool,
}

impl MacRuntime {
    pub fn new(
        config: Config,
        mtm: MainThreadMarker,
        log_broadcast: Option<LogBroadcast>,
        node_events: Option<NodeEventBroadcast>,
    ) -> Self {
        let rules = Arc::new(Mutex::new(gevrek_engine::config_apply::build_rule_engine(
            &config.desktop,
        )));
        let mut executor = Executor::new_with_events(
            config.node_graph.clone(),
            Arc::new(Mutex::new(Vec::new())),
            log_broadcast,
            node_events.clone(),
            Arc::clone(&rules),
        );
        executor.set_scratchpad_profiles(config.desktop.scratchpads.clone());
        executor.set_workspace_profiles(
            config.desktop.workspaces.clone(),
            config.desktop.appearance.clone(),
        );
        let outputs = Outputs::new();
        let snap_zones = gevrek_engine::config_apply::build_snap_zone_manager(
            &config.desktop.snap_zones,
            &outputs,
            |_| false,
        );
        let core = WmCore::new(
            executor,
            outputs,
            FocusManager::with_config(config.desktop.focus.clone()),
            snap_zones,
        );
        let (sender, events) = std::sync::mpsc::channel();
        // AppKit, for the windows gevrek draws its borders in: an
        // application without a Dock icon or a menu bar of its own.
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        // (Without it no click on gevrek's own windows reaches its queue.)
        app.finishLaunching();
        let mut desktop = MacDesktop::new(
            Arc::clone(&rules),
            crate::journal::Journal::open(crate::journal::Journal::default_path()),
        );
        desktop.borders = Some(crate::borders::Borders::new(mtm));
        desktop.stackbars = Some(crate::stackbars::Stackbars::new(mtm));
        Self {
            desktop,
            config,
            core,
            host: HostState {
                events: node_events,
                ..HostState::default()
            },
            rules,
            mtm,
            events,
            sender,
            observers: HashMap::new(),
            keys: Default::default(),
            tap: None,
            drag: None,
            touchpad: None,
            gestures: gevrek_engine::gesture::GestureRecognizer::default(),
            looked_at_programs: Instant::now(),
            looked_at_windows: Instant::now(),
            passes_per_second: 0.0,
            running: true,
            starting: true,
        }
    }

    /// Connect to the IPC server.
    pub fn attach(&mut self, links: HostLinks) {
        self.host
            .attach(links, self.config.desktop.watch_configuration);
    }

    /// The desktop as it is when gevrek starts: the displays, the windows a
    /// previous gevrek left parked put back, the programs and their windows,
    /// then the configuration's startup, profiles and bindings, and every
    /// window that is there adopted.
    pub fn start(&mut self) -> anyhow::Result<()> {
        if !crate::ax::trusted(true) {
            // macOS asks the person (the call above shows its question).
            // The permission is the application's when gevrek runs as one;
            // a bare gevrek-wm started from a terminal gets the terminal's.
            let whose = if std::env::current_exe()
                .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
            {
                "\"gevrek\""
            } else {
                "the terminal gevrek-wm was started from"
            };
            anyhow::bail!(
                "gevrek may not use the Accessibility API yet: allow {whose} in System Settings, \
                 Privacy & Security, Accessibility, then start gevrek again"
            );
        }
        self.keys.borrow_mut().drag_modifier = drag_modifier();
        // Without the keyboard gevrek still runs (the CLI and the editor
        // drive it); it says why the keys do nothing.
        match crate::keys::KeyTap::install(self.keys.clone()) {
            Ok(tap) => self.tap = Some(tap),
            Err(error) => log::error!("{error}"),
        }
        self.follow_displays();
        let restored = self.desktop.restore_journaled_windows();
        if restored > 0 {
            log::warn!("Put back {restored} window(s) a previous gevrek left parked");
        }
        self.follow_programs();
        self.starting = false;
        self.core.executor.clear_state();
        self.core
            .executor
            .execute_startup(&mut self.desktop, &mut self.core.monitor_manager)?;
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
        self.apply_workspace_profile_set(|_| true, true);
        self.refresh_bindings()?;
        self.core.ensure_default_workspaces();
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
        let adopted = self.core.adopt_existing_windows(&mut self.desktop)?;
        log::info!("Adopted {adopted} window(s)");
        let startup = self.config.desktop.startup.clone();
        self.host.startup.configure(&startup, false);
        self.host.emit(
            RuntimeEventKind::RuntimeStarted,
            None,
            [("config_version".to_string(), self.config.version.clone())],
        );
        self.settle();
        self.publish_state(Some("started".to_string()));
        Ok(())
    }

    /// Run until asked to stop. Every window gevrek parked is put back
    /// before it returns.
    pub fn run(&mut self) -> anyhow::Result<()> {
        let mut passes = 0u32;
        let mut counted = Instant::now();
        while self.running {
            gevrek_host::heartbeat::beat();
            observer::run_for(PASS.as_secs_f64());
            if self.host.stop_requested() {
                log::info!("Stop requested");
                break;
            }
            self.handle_ax_events();
            self.handle_keys();
            self.handle_gestures();
            self.handle_clicks();
            self.run_queued_commands();
            self.apply_configuration_changes();
            if self.looked_at_programs.elapsed() >= Duration::from_millis(500) {
                self.looked_at_programs = Instant::now();
                self.follow_programs();
                self.follow_displays();
            }
            if self.looked_at_windows.elapsed() >= Duration::from_secs(1) {
                self.looked_at_windows = Instant::now();
                self.follow_windows();
                if let Some(tap) = &self.tap {
                    tap.keep_on();
                }
                // (A program can raise its window after macOS said the
                // focus changed: its border and bar are put back above it.)
                if let Some(focused) = self.desktop.focused {
                    self.desktop.raise_overlays(focused);
                }
            }
            if !self.host.paused() {
                self.host.startup.tick();
            }
            self.publish_config_if_changed();
            passes += 1;
            if counted.elapsed() >= Duration::from_secs(1) {
                self.passes_per_second = f64::from(passes) / counted.elapsed().as_secs_f64();
                passes = 0;
                counted = Instant::now();
            }
            let due = self.host.last_published.elapsed() >= Duration::from_secs(1);
            if due || self.host.state_stale.get() {
                self.publish_state(None);
            }
        }
        self.shutdown();
        Ok(())
    }

    /// Stop: the programs started for the user end, and every parked
    /// window comes back where it was.
    fn shutdown(&mut self) {
        self.host.startup.shutdown();
        self.host
            .emit(RuntimeEventKind::RuntimeStopping, None, std::iter::empty());
        let parked: Vec<WindowHandle> = self
            .desktop
            .windows
            .iter()
            .filter(|(_, window)| window.parked)
            .map(|(id, _)| *id)
            .collect();
        for id in parked {
            let _ = self.desktop.apply(&gevrek_engine::Effect::SetVisible { id, visible: true });
        }
        log::info!("gevrek stopped cleanly");
    }

    /// Tell the core what happened to a window, then bring the desktop to
    /// what the core decided.
    fn window_event(&mut self, event: WindowEvent) {
        let (kind, window, fields) = RuntimeEventKind::of_window_event(&event);
        self.host.emit(kind, Some(window), fields);
        if let Err(error) = self.core.handle_window_event(&mut self.desktop, event.clone()) {
            log::warn!("{event:?}: {error}");
        }
        self.settle();
    }

    /// Start knowing a window and watching it, and tell the core it came.
    fn window_came(&mut self, element: Element, app: &App) {
        let Some(id) = self.desktop.track(element.clone(), app) else {
            return;
        };
        if let Some(observer) = self.observers.get_mut(&app.pid) {
            if let Err(error) = observer.watch_window(&element) {
                log::debug!("window {id:?} not watched: {error}");
            }
        }
        if !self.starting {
            self.window_event(WindowEvent::Created(id));
        }
    }

    fn window_went(&mut self, id: WindowHandle) {
        let Some(pid) = self.desktop.get(id).map(|window| window.app.pid) else {
            return;
        };
        if let Some(observer) = self.observers.get_mut(&pid) {
            observer.unwatch_window(id.0 as u32);
        }
        self.window_event(WindowEvent::Destroyed(id));
        // (The core untracks it; a window it never managed is let go here.)
        self.desktop.untrack_window(id);
    }

    /// The observers' events, as the core's window events.
    fn handle_ax_events(&mut self) {
        let events: Vec<AxEvent> = self.events.try_iter().collect();
        // A title that changes many times in a pass (a terminal's) is told
        // once.
        let mut retitled = Vec::new();
        for event in events {
            let id = event.window.map(handle_of);
            let known = id.filter(|id| self.desktop.exists(*id));
            match event.kind {
                AxEventKind::WindowCreated => {
                    if let (Some(element), Some(app)) = (event.element, apps::app(event.pid)) {
                        self.window_came(element, &app);
                    }
                }
                AxEventKind::Destroyed => {
                    if let Some(id) = known {
                        self.window_went(id);
                    }
                }
                AxEventKind::FocusedWindowChanged | AxEventKind::Activated => {
                    let focused = match event.kind {
                        AxEventKind::Activated => Element::application(event.pid)
                            .focused_window()
                            .ok()
                            .and_then(|window| window.window_id())
                            .map(handle_of),
                        _ => id,
                    };
                    if let Some(id) = focused.filter(|id| self.desktop.exists(*id)) {
                        self.desktop.raise_overlays(id);
                        if self.desktop.focused != Some(id) {
                            self.desktop.focused = Some(id);
                            self.window_event(WindowEvent::Focused(id));
                        }
                    }
                }
                AxEventKind::Moved | AxEventKind::Resized => {
                    let Some(id) = known else { continue };
                    let parked = self.desktop.get(id).is_some_and(|window| window.parked);
                    if parked || self.desktop.moved_by_gevrek(id) {
                        continue;
                    }
                    self.desktop.refresh(id);
                    self.desktop.follow_border(id);
                    let rect = self.desktop.get(id).map(|window| window.rect).unwrap_or_default();
                    self.window_event(if event.kind == AxEventKind::Moved {
                        WindowEvent::Moved(id, rect.x, rect.y)
                    } else {
                        WindowEvent::Resized(id, rect.width, rect.height)
                    });
                }
                AxEventKind::Minimized | AxEventKind::Unminimized => {
                    let Some(id) = known else { continue };
                    self.desktop.refresh(id);
                    self.window_event(if event.kind == AxEventKind::Minimized {
                        WindowEvent::Minimized(id)
                    } else {
                        WindowEvent::Restored(id)
                    });
                }
                AxEventKind::TitleChanged => {
                    if let Some(id) = known {
                        if !retitled.contains(&id) {
                            retitled.push(id);
                        }
                    }
                }
                // Command-H: the program's windows leave the layout until
                // it is shown again, as a window an application hides.
                AxEventKind::Hidden | AxEventKind::Shown => {
                    let hidden = event.kind == AxEventKind::Hidden;
                    for id in self.desktop.windows_of(event.pid) {
                        if let Some(window) = self.desktop.windows.get_mut(&id) {
                            window.app_hidden = hidden;
                        }
                        self.window_event(if hidden {
                            WindowEvent::Unmapped(id)
                        } else {
                            WindowEvent::Created(id)
                        });
                    }
                }
                AxEventKind::Deactivated | AxEventKind::Other(_) => {}
            }
        }
        for id in retitled {
            self.desktop.refresh(id);
            self.window_event(WindowEvent::MetadataChanged(id));
        }
    }

    /// What happened to gevrek's own windows since the last pass, from
    /// AppKit's queue (gevrek does not run AppKit's loop): a click on a
    /// stackbar's tab shows that tab's window. Everything else goes on to
    /// AppKit as it would in its own loop.
    /// What the touchpads' fingers did since the last pass, as gestures.
    fn handle_gestures(&mut self) {
        let Some(touchpad) = &self.touchpad else {
            return;
        };
        let mut events = Vec::new();
        for (frame, at) in touchpad.frames() {
            events.extend(self.gestures.feed_frame(frame, at));
        }
        events.extend(self.gestures.tick(crate::touchpad::clock_ms()));
        for event in events {
            self.gesture(&event);
        }
    }

    fn handle_clicks(&mut self) {
        use objc2_app_kit::{NSEventMask, NSEventType};
        let app = objc2_app_kit::NSApplication::sharedApplication(self.mtm);
        let past = objc2_foundation::NSDate::distantPast();
        // SAFETY: the run loop mode is the system's constant.
        let mode = unsafe { objc2_foundation::NSDefaultRunLoopMode };
        while let Some(event) =
            app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&past), mode, true)
        {
            if event.r#type() == NSEventType::LeftMouseDown {
                let at = event.locationInWindow();
                if let Some((key, index)) =
                    self.desktop.stackbar_tab_at(event.windowNumber(), at.x)
                {
                    if let Err(error) = self.core.executor.focus_stackbar_tab(
                        &mut self.desktop,
                        &mut self.core.monitor_manager,
                        key,
                        index,
                    ) {
                        log::warn!("stackbar tab: {error}");
                    }
                    self.settle();
                    continue;
                }
            }
            app.sendEvent(&event);
        }
    }

    /// The keys and buttons that hit bindings since the last pass, run;
    /// and the drags of windows with the mouse, carried out.
    fn handle_keys(&mut self) {
        let (hits, drags) = {
            let mut keys = self.keys.borrow_mut();
            keys.paused = self.host.paused();
            keys.managed = self.desktop.shown_window_ids();
            keys.tick(Instant::now());
            (std::mem::take(&mut keys.hits), std::mem::take(&mut keys.drags))
        };
        for hit in hits {
            self.bindings_triggered(&hit);
        }
        self.handle_drags(drags);
    }

    /// A window moved or resized with the mouse (the drag modifier and
    /// the left or the right button). Only where the pointer is last in a
    /// pass is the window put: a program takes a while to take a new size.
    fn handle_drags(&mut self, drags: Vec<crate::keys::DragEvent>) {
        use crate::keys::DragEvent;
        let mut to = None;
        for drag in drags {
            match drag {
                DragEvent::Start { window, resize, at } => {
                    let id = handle_of(window);
                    let Some(rect) = self.desktop.get(id).map(|window| window.rect) else {
                        continue;
                    };
                    let _ = self.desktop.apply(&gevrek_engine::Effect::Focus(Some(id)));
                    self.drag = Some(Drag {
                        id,
                        resize,
                        // The right button resizes from the corner nearest
                        // to the pointer.
                        from_left: at.0 < rect.x + rect.width / 2,
                        from_top: at.1 < rect.y + rect.height / 2,
                        start_pointer: at,
                        start_rect: rect,
                    });
                    self.window_event(WindowEvent::DragStarted(id));
                }
                DragEvent::To(at) => to = Some(at),
                DragEvent::End => {
                    if let Some(at) = to.take() {
                        self.drag_to(at);
                    }
                    if let Some(drag) = self.drag.take() {
                        // The engine decides what the drop means: a floating
                        // window stays, a tiled one swaps places or goes back.
                        self.window_event(WindowEvent::DragEnded(drag.id));
                    }
                }
            }
        }
        if let Some(at) = to {
            self.drag_to(at);
        }
    }

    fn drag_to(&mut self, at: (i32, i32)) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let rect = drag.rect_at(at);
        let Some(landed) = self.desktop.drag_to(drag.id, rect, drag.resize) else {
            return;
        };
        self.window_event(if drag.resize {
            WindowEvent::Resized(drag.id, landed.width, landed.height)
        } else {
            WindowEvent::Moved(drag.id, landed.x, landed.y)
        });
    }

    /// Watch the programs that started since the last look, and let go of
    /// those that ended (a program that is killed tells nothing itself).
    fn follow_programs(&mut self) {
        let only = only_these_programs();
        let running: Vec<App> = apps::running()
            .into_iter()
            .filter(|app| app.regular && app.pid != apps::own_pid())
            .filter(|app| only.as_ref().is_none_or(|only| only.contains(&app.app_id())))
            .collect();
        let gone: Vec<i32> = self
            .observers
            .keys()
            .copied()
            .filter(|pid| !running.iter().any(|app| app.pid == *pid))
            .collect();
        for pid in gone {
            for id in self.desktop.windows_of(pid) {
                self.window_went(id);
            }
            self.observers.remove(&pid);
        }
        for app in running {
            if self.observers.contains_key(&app.pid) {
                continue;
            }
            match Observer::new(app.pid, self.sender.clone()) {
                Ok(observer) => {
                    self.observers.insert(app.pid, observer);
                }
                // (One that is still starting answers next time.)
                Err(error) => {
                    log::debug!("{} ({}) not watched yet: {error}", app.name, app.pid);
                    continue;
                }
            }
            let windows = crate::window_list::elements_of(app.pid).unwrap_or_default();
            for element in windows.into_values() {
                self.window_came(element, &app);
            }
        }
    }

    /// The windows the observers missed: one that is there and unknown is
    /// taken, one that is known and gone is let go.
    fn follow_windows(&mut self) {
        let all = crate::window_list::server_windows(false);
        let gone: Vec<WindowHandle> = self
            .desktop
            .windows
            .keys()
            .copied()
            .filter(|id| !all.iter().any(|window| handle_of(window.id) == *id))
            .collect();
        for id in gone {
            self.window_went(id);
        }
        let unknown: Vec<i32> = all
            .iter()
            .filter(|window| {
                window.on_screen
                    && window.layer == crate::window_list::NORMAL_LAYER
                    && self.observers.contains_key(&window.pid)
                    && !self.desktop.exists(handle_of(window.id))
            })
            .map(|window| window.pid)
            .collect();
        let mut asked = Vec::new();
        for pid in unknown {
            if asked.contains(&pid) {
                continue;
            }
            asked.push(pid);
            let Some(app) = apps::app(pid) else { continue };
            let windows = crate::window_list::elements_of(pid).unwrap_or_default();
            for element in windows.into_values() {
                self.window_came(element, &app);
            }
        }
    }

    /// The displays: read again, and where they changed, the core carries
    /// the workspaces and windows over.
    fn follow_displays(&mut self) {
        let displays = crate::displays::displays(self.mtm);
        if displays == self.desktop.displays {
            return;
        }
        self.desktop.displays = displays;
        let monitors = crate::displays::monitors(self.mtm);
        if self.core.monitor_manager.get_all_monitors().is_empty() {
            self.core.monitor_manager.replace_monitors(monitors);
            self.configure_monitors();
        } else {
            let changes = self.core.monitor_manager.reconcile_with(monitors);
            self.core.executor.apply_monitor_changes(
                &mut self.desktop,
                &self.core.monitor_manager,
                &changes,
            );
            self.configure_monitors();
            for monitor in self.core.monitor_manager.get_all_monitors() {
                self.core.executor.mark_layout_dirty(monitor.handle);
            }
            self.settle();
        }
        log::info!("{} display(s)", self.desktop.displays.len());
    }
}

impl WmHost for MacRuntime {
    type Desktop = MacDesktop;

    fn parts(&mut self) -> Parts<'_, MacDesktop> {
        Parts {
            config: &mut self.config,
            core: &mut self.core,
            desktop: &mut self.desktop,
            host: &mut self.host,
        }
    }

    fn host(&self) -> &HostState {
        &self.host
    }

    fn config(&self) -> &Config {
        &self.config
    }

    fn settle(&mut self) {
        if let Err(error) = self.core.flush_pending_layouts(&mut self.desktop) {
            log::warn!("layout: {error}");
        }
        // A window that would not take the size it was given: its output is
        // laid out again around the size it has (once: what it keeps now is
        // known).
        let learned = self.desktop.take_learned();
        if !learned.is_empty() {
            for id in learned {
                if let Some(output) = self.desktop.output_of(id) {
                    self.core.executor.mark_layout_dirty(output);
                }
            }
            if let Err(error) = self.core.flush_pending_layouts(&mut self.desktop) {
                log::warn!("layout: {error}");
            }
            self.desktop.take_learned();
        }
        self.core
            .apply_focus_effects(&mut self.desktop, &self.config.desktop);
    }

    fn install_bindings(&mut self, bindings: Vec<(i32, Hotkey)>) -> anyhow::Result<()> {
        self.keys.borrow_mut().bindings = bindings;
        // The gestures go with the bindings (both are the configuration's
        // ways of running a command).
        let registrations = gevrek_host::wm::bindings::gesture_registrations(&self.config);
        let gestures = &self.config.desktop.gestures;
        let wanted = !registrations.is_empty();
        self.gestures
            .configure(registrations, gestures.idle_timeout_ms, gestures.axis_lock_ratio);
        if wanted && self.touchpad.is_none() {
            self.touchpad = crate::touchpad::Touchpad::start();
        } else if !wanted {
            self.touchpad = None;
        }
        Ok(())
    }

    fn apply_appearance(&mut self) {
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

    fn stop_without_server(&mut self) {
        self.running = false;
    }

    // (Animations are not drawn on macOS: `MacDesktop`.)
    fn set_animations(&mut self, _animations: &gevrek_model::settings::AnimationProfile) {}

    // (The menu bar and the Dock are left out of each display's work area
    // already: `displays.rs`.) The room of gevrek's own panel is kept here,
    // as on Windows: macOS lets no program keep room free of the others,
    // and the shell's panel is put where the window manager says
    // (`gevrek_shellkit::mac`, `Control::open_at`).
    fn configure_monitors(&mut self) {
        gevrek_engine::config_apply::configure_monitor_profiles(
            &mut self.core.monitor_manager,
            &self.config.desktop,
        );
        self.core
            .monitor_manager
            .set_bar(Some(self.config.desktop.shell.panel.clone()));
    }

    fn rules(&self) -> &Arc<Mutex<RuleEngine>> {
        &self.rules
    }

    fn system_state(&self) -> SystemState {
        SystemState {
            event_loop_fps: self.passes_per_second,
            gesture_backend: if self.touchpad.is_some() {
                "multitouch".to_string()
            } else {
                "none".to_string()
            },
            ..SystemState::default()
        }
    }
}

/// A move or resize of a window with the mouse, from where it began.
#[derive(Debug, Clone)]
struct Drag {
    id: WindowHandle,
    resize: bool,
    /// When resizing: the left edge moves (else the right), the top edge
    /// moves (else the bottom).
    from_left: bool,
    from_top: bool,
    start_pointer: (i32, i32),
    start_rect: gevrek_engine::Rect,
}

impl Drag {
    /// Where the window is with the pointer at `at`: moved with it, or the
    /// edges nearest to where the button went down following it, the
    /// others staying.
    fn rect_at(&self, at: (i32, i32)) -> gevrek_engine::Rect {
        let (dx, dy) = (at.0 - self.start_pointer.0, at.1 - self.start_pointer.1);
        let start = self.start_rect;
        if !self.resize {
            return gevrek_engine::Rect::new(start.x + dx, start.y + dy, start.width, start.height);
        }
        let min = crate::keys::MIN_DRAG_SIZE;
        let width = if self.from_left { start.width - dx } else { start.width + dx }.max(min);
        let height = if self.from_top { start.height - dy } else { start.height + dy }.max(min);
        let x = if self.from_left { start.x + start.width - width } else { start.x };
        let y = if self.from_top { start.y + start.height - height } else { start.y };
        gevrek_engine::Rect::new(x, y, width, height)
    }
}

/// What, held alone, lets the mouse move (left button) and resize (right
/// button) windows: gevrek's Win key, Option on a Mac, or what
/// `GEVREK_MACOS_DRAG_MODIFIER` names (`option`, `control`, `shift`,
/// `command`, several with commas; `none`: the mouse drags no window). Some
/// programs use Option with the mouse themselves (Finder copies what is
/// dragged with it).
fn drag_modifier() -> Option<crate::keys::Held> {
    let named = std::env::var("GEVREK_MACOS_DRAG_MODIFIER").unwrap_or_else(|_| "option".to_string());
    let mut held = crate::keys::Held::default();
    for name in named.split(',') {
        match name.trim().to_ascii_lowercase().as_str() {
            "option" | "alt" => held.option = true,
            "control" | "ctrl" => held.control = true,
            "shift" => held.shift = true,
            "command" | "cmd" => held.command = true,
            "none" | "" => {}
            other => log::warn!("GEVREK_MACOS_DRAG_MODIFIER: no modifier '{other}'"),
        }
    }
    (held != crate::keys::Held::default()).then_some(held)
}

/// The programs gevrek may manage, where `GEVREK_MACOS_ONLY` names them
/// (bundle ids, separated by commas: `com.apple.calculator,com.apple.dictionary`):
/// to try gevrek on a few programs without it taking over the desktop.
/// The others are not even watched. `None`: every program.
fn only_these_programs() -> Option<Vec<String>> {
    let only = std::env::var("GEVREK_MACOS_ONLY").ok()?;
    let ids: Vec<String> = only
        .split(',')
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// Run the window manager on this thread, which must be the main one,
/// connected to the IPC server through `links`, until it is asked to stop.
/// `ready` is called once it has started (the server then says it is
/// healthy).
pub fn run(
    config: Config,
    links: HostLinks,
    log_broadcast: Option<LogBroadcast>,
    node_events: Option<NodeEventBroadcast>,
    ready: impl FnOnce(),
) -> anyhow::Result<()> {
    let Some(mtm) = MainThreadMarker::new() else {
        anyhow::bail!("the macOS window manager runs on the main thread");
    };
    if let Some(only) = only_these_programs() {
        log::warn!("GEVREK_MACOS_ONLY: only these programs are managed: {}", only.join(", "));
    }
    let mut runtime = MacRuntime::new(config, mtm, log_broadcast, node_events);
    runtime.attach(links);
    runtime.start()?;
    ready();
    runtime.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(resize: bool, from_left: bool, from_top: bool) -> Drag {
        Drag {
            id: WindowHandle(1),
            resize,
            from_left,
            from_top,
            start_pointer: (500, 400),
            start_rect: gevrek_engine::Rect::new(100, 100, 600, 500),
        }
    }

    #[test]
    fn a_dragged_window_moves_with_the_pointer() {
        assert_eq!(
            drag(false, false, false).rect_at((530, 380)),
            gevrek_engine::Rect::new(130, 80, 600, 500)
        );
    }

    #[test]
    fn a_resize_moves_the_edges_nearest_to_where_it_began() {
        // From the bottom-right: the top-left corner stays.
        assert_eq!(
            drag(true, false, false).rect_at((550, 420)),
            gevrek_engine::Rect::new(100, 100, 650, 520)
        );
        // From the top-left: the bottom-right corner stays.
        assert_eq!(
            drag(true, true, true).rect_at((450, 380)),
            gevrek_engine::Rect::new(50, 80, 650, 520)
        );
        // Never smaller than the least size.
        let tiny = drag(true, false, false).rect_at((-2000, -2000));
        assert_eq!((tiny.width, tiny.height), (crate::keys::MIN_DRAG_SIZE, crate::keys::MIN_DRAG_SIZE));
    }
}
