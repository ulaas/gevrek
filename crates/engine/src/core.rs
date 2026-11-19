//! The core of the window manager: everything it knows and decides
//! (docs/PHASE2_PLAN.md, the end of step 4).
//!
//! `WmCore` owns the state: the outputs with their workspaces, the focus
//! history, the zones, and the executor with the per-window state, the
//! rule metadata, the scratchpads and the automation graph. The host (the Win32 `Runtime`)
//! is its host: it owns the loop, the hooks, IPC and the backend
//! (on Windows the `WindowManager`), and hands the core each event together with the
//! backend to act on.
//!
//! The core and the executor take the backend as `&mut dyn Desktop`, the
//! interface this crate defines, so they name no particular backend.

use crate::contract::Desktop;
use crate::events::WindowEvent;
use crate::executor::Executor;
use crate::focus_manager::FocusManager;
use crate::outputs::Outputs as MonitorManager;
use crate::snap_zones::SnapZoneManager;
use anyhow::Result;

pub struct WmCore {
    pub executor: Executor,
    /// Outputs and their workspaces.
    pub monitor_manager: MonitorManager,
    pub focus_manager: FocusManager,
    pub snap_zone_manager: SnapZoneManager,
    /// The colour that stands out in the wallpaper, where the backend
    /// draws one and has looked: a theme whose accent is `wallpaper`
    /// takes it.
    pub wallpaper_accent: Option<(u8, u8, u8)>,
}

impl WmCore {
    pub fn new(
        executor: Executor,
        monitor_manager: MonitorManager,
        focus_manager: FocusManager,
        snap_zone_manager: SnapZoneManager,
    ) -> Self {
        Self {
            executor,
            monitor_manager,
            focus_manager,
            snap_zone_manager,
            wallpaper_accent: None,
        }
    }

    /// React to one window event from the backend.
    pub fn handle_window_event(
        &mut self,
        backend: &mut dyn Desktop,
        event: WindowEvent,
    ) -> Result<()> {
        // The focus history: the focus rules ("hand focus back to the
        // previous window") read it while the event is handled.
        match event {
            WindowEvent::Focused(id) => {
                self.focus_manager.record_focus(id);
            }
            WindowEvent::Destroyed(id) => self.focus_manager.remove_window(id),
            _ => {}
        }
        // Values left by the previous event's graph run do not carry over.
        self.executor.clear_state();
        self.executor.execute_window_event(
            backend,
            &mut self.monitor_manager,
            &mut self.snap_zone_manager,
            &mut self.focus_manager,
            event,
        )
    }

    /// Carry out one command (from a binding, the CLI, gRPC or the editor).
    pub fn dispatch(
        &mut self,
        backend: &mut dyn Desktop,
        command: gevrek_model::command::DispatchCommand,
    ) -> Result<()> {
        self.executor.execute_dispatch(
            backend,
            &mut self.monitor_manager,
            &mut self.snap_zone_manager,
            &mut self.focus_manager,
            command,
        )
    }

    /// Lay out the workspaces that events marked as changed.
    pub fn flush_pending_layouts(&mut self, backend: &mut dyn Desktop) -> Result<()> {
        self.executor
            .flush_pending_layouts(backend, &mut self.monitor_manager)
    }

    /// The floating windows that were moved to another display without a
    /// drag and have come to rest there join that display's workspace
    /// (`Executor::settle_moved_windows`). The host calls this every now
    /// and then, and publishes the state when a window is returned.
    pub fn settle_moved_windows(
        &mut self,
        backend: &mut dyn Desktop,
        now: std::time::Instant,
    ) -> Vec<crate::WindowHandle> {
        self.executor
            .settle_moved_windows(backend, &mut self.monitor_manager, now)
    }

    /// The keyboard goes back to the window that had it before `from`
    /// took it: a window the backend has put out of sight as it opened
    /// (it belongs to a window that is put away). Returns whether there
    /// was a window to give it to.
    pub fn hand_keyboard_back(
        &mut self,
        backend: &mut dyn Desktop,
        from: crate::WindowHandle,
    ) -> Result<bool> {
        self.executor.hand_keyboard_back(
            backend,
            &mut self.monitor_manager,
            &mut self.focus_manager,
            from,
        )
    }

    /// Every output has at least one workspace.
    pub fn ensure_default_workspaces(&mut self) {
        for monitor in self.monitor_manager.get_all_monitors() {
            let has_workspaces = self
                .monitor_manager
                .get_workspaces(monitor.handle)
                .is_some_and(|workspaces| !workspaces.is_empty());
            if has_workspaces {
                continue;
            }
            log::info!(
                "Monitor '{}' has no workspaces, creating default BSP workspace",
                monitor.name
            );
            if let Some(index) = self.monitor_manager.create_workspace(
                monitor.handle,
                "1".to_string(),
                crate::layouts::LayoutType::BSP,
            ) {
                log::info!(
                    "Created default workspace at index {} for monitor '{}'",
                    index,
                    monitor.name
                );
            }
        }
    }

    /// Take over the windows that are already open when the WM starts.
    /// Returns how many were adopted.
    pub fn adopt_existing_windows(&mut self, backend: &mut dyn Desktop) -> Result<usize> {
        let handles: Vec<crate::WindowHandle> = backend
            .windows()
            .iter()
            .map(|window| window.handle)
            .collect();
        let mut adopted = 0usize;
        for handle in handles {
            if backend.ignored_by_rules(handle)
                || backend.is_decoration(handle)
                || backend.is_minimized(handle)
            {
                continue;
            }
            match self
                .executor
                .adopt_window_by_default(backend, &mut self.monitor_manager, handle)
            {
                Ok(true) => adopted += 1,
                Ok(false) => {}
                Err(error) => {
                    log::warn!("Could not adopt existing window {:?}: {}", handle, error)
                }
            }
        }
        // Adoption only marks monitors dirty; lay each one out once.
        self.flush_pending_layouts(backend)?;
        Ok(adopted)
    }

    /// What to save when the WM stops, so the next start can put windows
    /// back: every window with its workspace, the focused workspace of
    /// every output, and the remembered window sizes.
    pub fn session_state(&self, backend: &dyn Desktop) -> crate::persistence::WindowManagerState {
        let mut state = crate::persistence::WindowManagerState::new();
        for window in backend.windows() {
            if backend.is_decoration(window.handle) || !backend.exists(window.handle) {
                continue;
            }
            let placement = self.monitor_manager.find_window(window.handle);
            let workspace = placement.and_then(|(monitor, index)| {
                self.monitor_manager
                    .get_workspaces(monitor)
                    .and_then(|workspaces| workspaces.get(index))
            });
            state.windows.push(crate::persistence::WindowState {
                handle: window.handle,
                title: window.title.clone(),
                exe: backend.exe(window.handle),
                class: backend.class(window.handle),
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
                is_minimized: window.is_minimized,
                is_maximized: window.is_maximized,
                workspace_id: placement.map(|(_, index)| index),
                workspace_name: workspace.map(|workspace| workspace.name.clone()),
                monitor: placement.map(|(monitor, _)| monitor),
                monitor_known_as: placement
                    .and_then(|(monitor, _)| self.monitor_manager.get_monitor(monitor))
                    .map(crate::executor::display_known_as),
                is_floating: workspace.is_some_and(|workspace| {
                    workspace
                        .floating_windows
                        .iter()
                        .any(|w| *w == window.handle)
                }),
                container_index: workspace.and_then(|workspace| {
                    workspace
                        .containers
                        .elements()
                        .iter()
                        .position(|container| container.windows.elements().contains(&window.handle))
                }),
            });
        }
        for (handle, info) in self.monitor_manager.get_monitors_with_handles() {
            if let Some(workspaces) = self.monitor_manager.get_workspaces(handle) {
                state
                    .focused_workspaces
                    .insert(handle, workspaces.focused_idx());
                if let Some(shown) = workspaces.get(workspaces.focused_idx()) {
                    state
                        .shown_workspaces
                        .push(crate::persistence::ShownWorkspace {
                            monitor: handle,
                            monitor_known_as: crate::executor::display_known_as(&info),
                            workspace_name: shown.name.clone(),
                        });
                }
            }
        }
        state.persistent_window_sizes = self.executor.persistent_window_sizes();
        state
    }

    /// Put a saved session's windows back on their workspaces (see
    /// `Executor::restore_session`) and lay the outputs out.
    pub fn restore_session(
        &mut self,
        backend: &mut dyn Desktop,
        state: &crate::persistence::WindowManagerState,
    ) -> Result<usize> {
        let moved = self
            .executor
            .restore_session(backend, &mut self.monitor_manager, state);
        for monitor in self.monitor_manager.get_all_monitors() {
            self.executor.mark_layout_dirty(monitor.handle);
        }
        self.flush_pending_layouts(backend)?;
        Ok(moved)
    }
}
