//! Phase 2 step 0: behaviour characterization.
//!
//! These tests pin what the runtime does today for the main window flows,
//! so the engine extraction (docs/PHASE2_PLAN.md) can prove it changed
//! nothing. They drive a hook-less runtime with real, never-shown windows
//! and assert workspace membership, focus, and which windows the WM hides.
//!
//! `drive` mirrors the per-event sequence of `Runtime::run` (coalesce, then
//! per event: registry update, focus bookkeeping, executor; then one layout
//! flush). Keep it in step with the loop until the engine replaces both.

use super::event_coalescing::coalesce_window_events;
use super::test_support::{finish, test_runtime, TestWindow, MONITOR};
use super::Runtime;
use gevrek_host::wm::commands::Commands as _;
use gevrek_host::wm::configuration::Configuration as _;
use crate::config::Config;
use crate::dwm::layouts::LayoutType;
use crate::dwm::{hidden_journal, MonitorHandle, MonitorInfo, WindowEvent, WindowHandle};
use crate::runtime::{DispatchCommand, WindowSelector, WorkspaceSelector};
use std::mem::ManuallyDrop;

fn drive(runtime: &mut Runtime, events: Vec<WindowEvent>) {
    for event in coalesce_window_events(events) {
        runtime.window_manager.apply_window_event(&event);
        match event {
            WindowEvent::Focused(handle) => {
                runtime.core.focus_manager.record_focus(handle);
            }
            WindowEvent::Destroyed(handle) => {
                runtime.core.focus_manager.remove_window(handle);
            }
            _ => {}
        }
        runtime.core.executor.clear_state();
        runtime
            .core
            .executor
            .execute_window_event(
                &mut runtime.window_manager,
                &mut runtime.core.monitor_manager,
                &mut runtime.core.snap_zone_manager,
                &mut runtime.core.focus_manager,
                event,
            )
            .unwrap();
    }
    runtime
        .core
        .executor
        .flush_pending_layouts(
            &mut runtime.window_manager,
            &mut runtime.core.monitor_manager,
        )
        .unwrap();
}

fn dispatch(runtime: &mut Runtime, command: DispatchCommand) {
    runtime.run_command(command).unwrap();
    runtime
        .core
        .executor
        .flush_pending_layouts(
            &mut runtime.window_manager,
            &mut runtime.core.monitor_manager,
        )
        .unwrap();
}

/// A runtime whose monitor has workspaces "one" (visible) and "two".
fn two_workspaces() -> ManuallyDrop<Runtime> {
    let mut runtime = test_runtime(Config::new());
    for name in ["one", "two"] {
        runtime
            .core
            .monitor_manager
            .create_workspace(MONITOR, name.to_string(), LayoutType::BSP)
            .unwrap();
    }
    runtime.core.executor.set_focused_monitor_for_tests(MONITOR);
    runtime
}

/// Membership of every workspace on `monitor`, the visible index, and each
/// workspace's focused window.
fn layout(runtime: &Runtime, monitor: MonitorHandle) -> String {
    let workspaces = runtime
        .core
        .monitor_manager
        .get_workspaces(monitor)
        .unwrap();
    let mut out = format!("visible={}", workspaces.focused_idx());
    for workspace in workspaces.iter() {
        let tiled: Vec<Vec<isize>> = workspace
            .containers
            .iter()
            .map(|container| container.windows.iter().map(|window| window.0).collect())
            .collect();
        let floating: Vec<isize> = workspace.floating_windows.iter().map(|w| w.0).collect();
        out.push_str(&format!(
            " | {} tiled={:?} floating={:?} focus={:?}",
            workspace.name,
            tiled,
            floating,
            workspace.focused_window().map(|w| w.0)
        ));
    }
    out
}

fn hidden(windows: &[&TestWindow]) -> Vec<bool> {
    windows
        .iter()
        .map(|window| hidden_journal::is_hidden(window.0))
        .collect()
}

fn id(window: &TestWindow) -> isize {
    window.0 .0
}

/// Windows appearing on screen: registered (as the admission filter would
/// for a real shown window), then their show events.
fn created(runtime: &mut Runtime, windows: &[&TestWindow]) {
    for window in windows {
        runtime
            .window_manager
            .register_for_tests(window.0, "characterization window");
    }
    drive(
        runtime,
        windows.iter().map(|w| WindowEvent::Created(w.0)).collect(),
    );
}

#[test]
fn new_windows_are_adopted_into_the_visible_workspace() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);

    // Each new window gets its own container, appended in order. Adoption
    // does not move the workspace focus: in real use the OS focus event of
    // the new window does that (see `a_focus_event_moves_the_workspace_focus`).
    assert_eq!(
        layout(&runtime, MONITOR),
        format!(
            "visible=0 | one tiled=[[{}], [{}]] floating=[] focus=Some({}) | two tiled=[] floating=[] focus=None",
            id(&a),
            id(&b),
            id(&a)
        )
    );
    assert_eq!(hidden(&[&a, &b]), vec![false, false]);
    finish(runtime);
}

#[test]
fn a_destroyed_window_leaves_its_workspace() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);
    drive(&mut runtime, vec![WindowEvent::Destroyed(a.0)]);

    assert_eq!(
        layout(&runtime, MONITOR),
        format!(
            "visible=0 | one tiled=[[{}]] floating=[] focus=Some({}) | two tiled=[] floating=[] focus=None",
            id(&b),
            id(&b)
        )
    );
    finish(runtime);
}

#[test]
fn a_focus_event_moves_the_workspace_focus() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);
    drive(&mut runtime, vec![WindowEvent::Focused(a.0)]);

    let workspace = runtime
        .core
        .monitor_manager
        .get_focused_workspace(MONITOR)
        .unwrap();
    assert_eq!(workspace.focused_window(), Some(a.0));
    finish(runtime);
}

#[test]
fn switching_workspaces_hides_and_shows_their_windows() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);

    dispatch(
        &mut runtime,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        },
    );
    assert!(layout(&runtime, MONITOR).starts_with("visible=1 "));
    assert_eq!(hidden(&[&a, &b]), vec![true, true]);

    dispatch(
        &mut runtime,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(0),
        },
    );
    assert!(layout(&runtime, MONITOR).starts_with("visible=0 "));
    assert_eq!(hidden(&[&a, &b]), vec![false, false]);
    finish(runtime);
}

#[test]
fn moving_a_window_to_a_hidden_workspace_hides_it() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);

    dispatch(
        &mut runtime,
        DispatchCommand::MoveWindowToWorkspace {
            window: WindowSelector::Handle(id(&a) as i64),
            workspace: WorkspaceSelector::Index(1),
        },
    );
    assert_eq!(
        layout(&runtime, MONITOR),
        format!(
            "visible=0 | one tiled=[[{}]] floating=[] focus=Some({}) | two tiled=[[{}]] floating=[] focus=Some({})",
            id(&b),
            id(&b),
            id(&a),
            id(&a)
        )
    );
    assert_eq!(hidden(&[&a, &b]), vec![true, false]);

    // Showing that workspace shows the moved window.
    dispatch(
        &mut runtime,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        },
    );
    assert_eq!(hidden(&[&a, &b]), vec![false, true]);
    let _ = runtime.window_manager.show_window(b.0);
    finish(runtime);
}

#[test]
fn minimize_and_restore_round_trip_through_the_loop() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);
    let before = layout(&runtime, MONITOR);

    drive(&mut runtime, vec![WindowEvent::Minimized(a.0)]);
    assert_eq!(runtime.core.monitor_manager.find_window(a.0), None);

    drive(&mut runtime, vec![WindowEvent::Restored(a.0)]);
    assert_eq!(
        runtime.core.monitor_manager.find_window(a.0),
        Some((MONITOR, 0))
    );
    // It rejoins as the last container; the workspace focus is not moved to
    // it (the OS focus event of the restored window does that in real use).
    assert_ne!(layout(&runtime, MONITOR), before);
    assert_eq!(
        layout(&runtime, MONITOR),
        format!(
            "visible=0 | one tiled=[[{}], [{}]] floating=[] focus=Some({}) | two tiled=[] floating=[] focus=None",
            id(&b),
            id(&a),
            id(&b)
        )
    );
    finish(runtime);
}

#[test]
fn an_app_hidden_window_is_readopted_when_shown_again() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);

    // Adoption just showed the window, so a hide within the 1 s grace period
    // is taken as the WM's own (its posted show/hide events can arrive late).
    drive(&mut runtime, vec![WindowEvent::Unmapped(a.0)]);
    assert_eq!(
        runtime.core.monitor_manager.find_window(a.0),
        Some((MONITOR, 0))
    );

    // After the grace period the hide is the app's own: it leaves the layout.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    drive(&mut runtime, vec![WindowEvent::Unmapped(a.0)]);
    assert_eq!(runtime.core.monitor_manager.find_window(a.0), None);

    drive(&mut runtime, vec![WindowEvent::Created(a.0)]);
    assert_eq!(
        runtime.core.monitor_manager.find_window(a.0),
        Some((MONITOR, 0))
    );
    finish(runtime);
}

fn display(handle: isize, name: &str, primary: bool, x: i32) -> MonitorInfo {
    MonitorInfo {
        handle: MonitorHandle(handle),
        name: name.to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: primary,
        x,
        y: 0,
        width: 1920,
        height: 1080,
        work_x: x,
        work_y: 0,
        work_width: 1920,
        work_height: 1040,
        dpi_scale: 1.0,
    }
}

#[test]
fn undock_evacuates_and_redock_restores_through_the_runtime() {
    let (a, b) = (TestWindow::create(), TestWindow::create());
    let mut runtime = two_workspaces();
    let external = MonitorHandle(2);
    runtime.core.monitor_manager.reconcile_with(vec![
        display(MONITOR.0, "Primary", true, 0),
        display(external.0, "External", false, 1920),
    ]);
    for name in ["web", "chat"] {
        runtime
            .core
            .monitor_manager
            .create_workspace(external, name.to_string(), LayoutType::BSP)
            .unwrap();
    }
    {
        let workspaces = runtime
            .core
            .monitor_manager
            .get_workspaces_mut(external)
            .unwrap();
        workspaces.get_mut(0).unwrap().add_window(a.0, true);
        workspaces.get_mut(1).unwrap().add_window(b.0, true);
    }
    // "chat" is hidden, so its window is WM-hidden.
    runtime.window_manager.hide_window(b.0).unwrap();

    let apply = |runtime: &mut Runtime, monitors: Vec<MonitorInfo>| {
        let changes = runtime.core.monitor_manager.reconcile_with(monitors);
        runtime.core.executor.apply_monitor_changes(
            &mut runtime.window_manager,
            &runtime.core.monitor_manager,
            &changes,
        );
        runtime
            .core
            .executor
            .flush_pending_layouts(
                &mut runtime.window_manager,
                &mut runtime.core.monitor_manager,
            )
            .unwrap();
    };

    apply(&mut runtime, vec![display(MONITOR.0, "Primary", true, 0)]);
    assert_eq!(
        runtime.core.monitor_manager.find_window(a.0),
        Some((MONITOR, 0))
    );
    assert_eq!(
        runtime.core.monitor_manager.find_window(b.0),
        Some((MONITOR, 0))
    );
    assert_eq!(
        hidden(&[&a, &b]),
        vec![false, false],
        "an evacuated window stayed hidden"
    );

    let redocked = MonitorHandle(7);
    apply(
        &mut runtime,
        vec![
            display(MONITOR.0, "Primary", true, 0),
            display(redocked.0, "External", false, 1920),
        ],
    );
    assert_eq!(
        runtime.core.monitor_manager.find_window(a.0),
        Some((redocked, 0))
    );
    assert_eq!(
        runtime.core.monitor_manager.find_window(b.0),
        Some((redocked, 1))
    );
    assert_eq!(hidden(&[&a, &b]), vec![false, true]);
    let _ = runtime.window_manager.show_window(b.0);
    finish(runtime);
}

#[test]
fn a_window_that_was_never_shown_by_its_handle_is_not_confused_with_another() {
    // Guard for D2 (WindowId transition): two windows, two distinct members.
    let (a, b) = (TestWindow::create(), TestWindow::create());
    assert_ne!(a.0, b.0);
    let mut runtime = two_workspaces();
    created(&mut runtime, &[&a, &b]);
    let members: Vec<WindowHandle> = runtime
        .core
        .monitor_manager
        .get_focused_workspace(MONITOR)
        .unwrap()
        .all_windows();
    assert_eq!(members, vec![a.0, b.0]);
    finish(runtime);
}

/// A full configuration apply names the revision it was edited from. A
/// toggle made in the runtime after the server's check (not yet published)
/// must not be overwritten: the runtime publishes it and refuses the apply.
#[test]
fn an_apply_based_on_a_revision_the_runtime_has_moved_past_is_refused() {
    use super::config_store::{config_channel, ConfigRequest, FIRST_REVISION, REFUSED};

    let mut runtime = test_runtime(Config::new());
    let (sender, receiver) = std::sync::mpsc::channel();
    runtime.host.config_receiver = Some(receiver);
    let (publisher, watch) = config_channel(runtime.config.clone());
    runtime.set_config_publisher(publisher);
    let published = watch.borrow().revision;
    assert!(published >= FIRST_REVISION);

    // A hotkey toggles the stackbar: changed here, not yet published.
    let stackbar = !runtime.config.desktop.appearance.stackbar.enabled;
    runtime.config.desktop.appearance.stackbar.enabled = stackbar;
    runtime.host.config_dirty = true;

    // The editor's configuration was made from `published`.
    let mut edited = Config::new();
    edited.desktop.focus.history_size += 5;
    let wanted = edited.desktop.focus.history_size;
    sender
        .send(ConfigRequest::ReplaceFrom {
            config: Box::new(edited.clone()),
            base_revision: published,
        })
        .unwrap();
    runtime.apply_configuration_changes();

    let snapshot = watch.borrow().clone();
    assert_eq!(
        snapshot.revision,
        published + 1,
        "the toggle got its revision"
    );
    assert_eq!(
        (snapshot.requests_applied, snapshot.last_request_revision),
        (1, REFUSED)
    );
    assert_eq!(runtime.config.desktop.appearance.stackbar.enabled, stackbar);
    assert_ne!(runtime.config.desktop.focus.history_size, wanted);

    // Made again from the revision that runs now, it is applied.
    edited.desktop.appearance.stackbar.enabled = stackbar;
    sender
        .send(ConfigRequest::ReplaceFrom {
            config: Box::new(edited),
            base_revision: published + 1,
        })
        .unwrap();
    runtime.apply_configuration_changes();
    let snapshot = watch.borrow().clone();
    assert_eq!(
        (snapshot.requests_applied, snapshot.last_request_revision),
        (2, published + 2)
    );
    assert_eq!(runtime.config.desktop.focus.history_size, wanted);
    assert_eq!(runtime.config.desktop.appearance.stackbar.enabled, stackbar);
    finish(runtime);
}
