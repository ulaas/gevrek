//! Scenarios that drive the real core (`WmCore` with the executor) on a
//! fake desktop: what the WM does with windows, without a windowing system.

use crate::contract::{Desktop, Effect};
use crate::core::WmCore;
use crate::events::WindowEvent;
use crate::executor::Executor;
use crate::focus_manager::FocusManager;
use crate::layouts::LayoutType;
use crate::outputs::{MonitorHandle, Outputs};
use crate::rules::RuleEngine;
use crate::snap_zones::SnapZoneManager;
use crate::testing::fake_desktop::FakeDesktop;
use crate::{Rect, WindowHandle};
use gevrek_model::command::{DispatchCommand, WorkspaceSelector};
use gevrek_model::graph::NodeGraph;
use std::sync::{Arc, Mutex};

const OUTPUT: MonitorHandle = MonitorHandle(1);

/// A core with one 1920x1080 output and the workspaces `names`, and a fake
/// desktop for it.
fn setup(names: &[&str]) -> (WmCore, FakeDesktop) {
    let rules = Arc::new(Mutex::new(RuleEngine::new()));
    let executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
    let mut outputs = Outputs::with_workspace_count(0);
    outputs.add_test_monitor(OUTPUT, "Primary");
    for name in names {
        outputs
            .create_workspace(OUTPUT, name.to_string(), LayoutType::BSP)
            .expect("workspace created");
    }
    let core = WmCore::new(
        executor,
        outputs,
        FocusManager::new(),
        SnapZoneManager::new(Default::default()),
    );
    (core, FakeDesktop::new(OUTPUT))
}

fn open(core: &mut WmCore, desktop: &mut FakeDesktop, title: &str) -> WindowHandle {
    let id = desktop.open(title, "app.exe");
    core.handle_window_event(desktop, WindowEvent::Created(id))
        .expect("created");
    core.flush_pending_layouts(desktop).expect("layout");
    id
}

fn tiled(core: &WmCore) -> Vec<WindowHandle> {
    core.monitor_manager
        .get_focused_workspace(OUTPUT)
        .map(|workspace| {
            workspace
                .containers
                .iter()
                .flat_map(|container| container.windows.iter().copied())
                .collect()
        })
        .unwrap_or_default()
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

#[test]
fn new_windows_are_tiled_side_by_side_on_the_focused_workspace() {
    let (mut core, mut desktop) = setup(&["main"]);
    let first = open(&mut core, &mut desktop, "one");
    let alone = desktop.window(first).rect;
    assert!(
        alone.width > 1500 && alone.height > 800,
        "a single window takes the work area: {alone:?}"
    );

    let second = open(&mut core, &mut desktop, "two");
    assert_eq!(tiled(&core), [first, second]);
    let (a, b) = (desktop.window(first).rect, desktop.window(second).rect);
    assert!(!overlap(a, b), "{a:?} and {b:?} overlap");
    assert!(a.width < alone.width, "the first window made room: {a:?}");
    for rect in [a, b] {
        assert!(
            rect.x >= 0
                && rect.y >= 0
                && rect.x + rect.width <= 1920
                && rect.y + rect.height <= 1080
        );
    }
}

#[test]
fn a_destroyed_window_leaves_the_layout_and_the_other_takes_its_room() {
    let (mut core, mut desktop) = setup(&["main"]);
    let first = open(&mut core, &mut desktop, "one");
    let second = open(&mut core, &mut desktop, "two");
    let shared = desktop.window(first).rect;

    desktop.close(second);
    core.handle_window_event(&mut desktop, WindowEvent::Destroyed(second))
        .expect("destroyed");
    core.flush_pending_layouts(&mut desktop).expect("layout");

    assert_eq!(tiled(&core), [first]);
    assert!(core.monitor_manager.find_window(second).is_none());
    assert!(
        desktop.window(first).rect.width > shared.width,
        "the remaining window grew: {:?} -> {:?}",
        shared,
        desktop.window(first).rect
    );
}

/// A window that has the keyboard and is on a workspace that is not
/// shown (it took the keyboard by itself) floats where it is. It used to
/// be put on the workspace that is shown as well: on two from then on.
#[test]
fn a_window_on_another_workspace_floats_where_it_is() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let on_main = open(&mut core, &mut desktop, "one");
    core.dispatch(
        &mut desktop,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        },
    )
    .expect("switched");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    desktop.focused = Some(on_main);

    core.dispatch(&mut desktop, DispatchCommand::ToggleFloating)
        .expect("toggled");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    let workspaces = core
        .monitor_manager
        .get_workspaces(OUTPUT)
        .unwrap()
        .elements();
    let (main, two) = (&workspaces[0], &workspaces[1]);
    assert_eq!(main.floating_windows.elements(), [on_main]);
    assert!(main.containers.is_empty());
    assert!(two.floating_windows.is_empty() && two.containers.is_empty());

    // And tiles again where it is.
    core.dispatch(&mut desktop, DispatchCommand::ToggleFloating)
        .expect("toggled back");
    let workspaces = core
        .monitor_manager
        .get_workspaces(OUTPUT)
        .unwrap()
        .elements();
    assert!(workspaces[0].floating_windows.is_empty());
    assert_eq!(workspaces[0].containers.len(), 1);
    assert!(workspaces[1].floating_windows.is_empty() && workspaces[1].containers.is_empty());
}

/// A layout command is for the workspace that is shown. Where the window
/// with the keyboard is on another one (a rule sent it there as it
/// opened), the command used to lay out that workspace and bring its
/// window into sight.
#[test]
fn a_layout_command_leaves_a_workspace_that_is_not_shown_out_of_sight() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let on_main = open(&mut core, &mut desktop, "one");
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        },
    );
    assert!(!desktop.window(on_main).visible);
    // (It has the keyboard all the same.)
    desktop.focused = Some(on_main);

    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayoutGap { gap: 20 },
    );
    assert!(!desktop.window(on_main).visible, "it stays out of sight");
    let workspaces = core
        .monitor_manager
        .get_workspaces(OUTPUT)
        .unwrap()
        .elements();
    assert_eq!(workspaces[1].container_padding, crate::Padding::uniform(20));
    assert_ne!(workspaces[0].container_padding, crate::Padding::uniform(20));
}

/// A window moved to another workspace "and follow" is followed on its
/// own display. Where another display had the keyboard, that one used
/// to change, to whatever it had at the same place, and the window was
/// left out of sight.
#[test]
fn a_window_is_followed_on_its_own_display() {
    use crate::outputs::MonitorInfo;
    use gevrek_model::command::MonitorTarget;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    for name in ["side", "more"] {
        core.monitor_manager
            .create_workspace(SECOND, name.to_string(), LayoutType::BSP)
            .expect("workspace on the second output");
    }
    desktop.more_outputs.push((SECOND, screen));

    let editor = open(&mut core, &mut desktop, "editor");
    // The second display has the keyboard; the editor, on the first, is
    // still the window that keys go to.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::FocusMonitor {
            target: MonitorTarget::Name("Second".to_string()),
        },
    );
    desktop.focused = Some(editor);

    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceAndFollow {
            selector: WorkspaceSelector::Name("two".to_string()),
        },
    );
    let shown = |core: &WmCore, output| {
        core.monitor_manager
            .get_focused_workspace(output)
            .map(|workspace| workspace.name.clone())
    };
    assert_eq!(
        shown(&core, OUTPUT).as_deref(),
        Some("two"),
        "its display follows"
    );
    assert_eq!(
        shown(&core, SECOND).as_deref(),
        Some("side"),
        "the other is left"
    );
    assert!(desktop.window(editor).visible, "and the window is in sight");
    assert_eq!(tiled(&core), [editor]);
}

#[test]
fn switching_the_workspace_hides_its_windows_and_shows_the_other_ones() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let on_main = open(&mut core, &mut desktop, "one");

    core.dispatch(
        &mut desktop,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        },
    )
    .expect("switched");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(
        core.monitor_manager
            .get_focused_workspace(OUTPUT)
            .map(|ws| ws.name.clone()),
        Some("two".to_string())
    );
    assert!(
        !desktop.window(on_main).visible,
        "the window of 'main' is hidden"
    );

    let on_two = open(&mut core, &mut desktop, "three");
    assert_eq!(tiled(&core), [on_two]);

    core.dispatch(
        &mut desktop,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(0),
        },
    )
    .expect("switched back");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert!(desktop.window(on_main).visible);
    assert!(!desktop.window(on_two).visible);
}

#[test]
fn the_close_command_asks_the_desktop_to_close_the_active_window() {
    let (mut core, mut desktop) = setup(&["main"]);
    let first = open(&mut core, &mut desktop, "one");
    let second = open(&mut core, &mut desktop, "two");
    desktop.apply(&Effect::Focus(Some(second))).unwrap();

    core.dispatch(&mut desktop, DispatchCommand::CloseActive)
        .expect("close");
    assert!(desktop.applied.contains(&Effect::Close(second)));
    assert!(!desktop.exists_window(second) && desktop.exists_window(first));
}

#[test]
fn the_session_state_lists_every_window_with_its_workspace() {
    let (mut core, mut desktop) = setup(&["main"]);
    let first = open(&mut core, &mut desktop, "one");
    let second = open(&mut core, &mut desktop, "two");

    let state = core.session_state(&desktop);
    let saved: Vec<_> = state
        .windows
        .iter()
        .map(|window| (window.handle, window.workspace_id, window.is_floating))
        .collect();
    assert_eq!(saved, [(first, Some(0), false), (second, Some(0), false)]);
    assert_eq!(state.focused_workspaces.get(&OUTPUT), Some(&0));
}

#[test]
fn a_minimized_window_leaves_the_layout_until_it_is_restored() {
    let (mut core, mut desktop) = setup(&["main"]);
    let first = open(&mut core, &mut desktop, "one");
    let second = open(&mut core, &mut desktop, "two");
    let shared = desktop.window(first).rect;

    desktop.windows.get_mut(&second).unwrap().minimized = true;
    core.handle_window_event(&mut desktop, WindowEvent::Minimized(second))
        .expect("minimized");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(tiled(&core), [first]);
    assert!(desktop.window(first).rect.width > shared.width);

    desktop.windows.get_mut(&second).unwrap().minimized = false;
    core.handle_window_event(&mut desktop, WindowEvent::Restored(second))
        .expect("restored");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    let mut back = tiled(&core);
    back.sort();
    assert_eq!(back, [first, second]);
    assert!(!overlap(
        desktop.window(first).rect,
        desktop.window(second).rect
    ));
}

#[test]
fn the_state_snapshot_reports_the_workspaces_and_their_windows() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let first = open(&mut core, &mut desktop, "one");
    let (monitors, total) = core.monitor_data(&desktop, &Default::default());
    assert_eq!(total, 1);
    assert_eq!(monitors.len(), 1);
    let names: Vec<_> = monitors[0]
        .workspaces
        .iter()
        .map(|ws| ws.name.as_str())
        .collect();
    assert_eq!(names, ["main", "two"]);
    assert_eq!(monitors[0].workspace.name, "main");
    let window = &monitors[0].workspace.detailed_windows[0];
    assert_eq!(
        (window.handle, window.is_tiled, window.title.as_str()),
        (first, true, "one")
    );
}

/// Windows tells that a new window has the keyboard before it tells that
/// the window opened. The workspace's focused window is the new one all
/// the same, and its border is told that it is focused once it has one.
#[test]
fn a_window_that_has_the_keyboard_before_it_opened_is_the_focused_one() {
    use crate::contract::BorderChange;

    let (mut core, mut desktop) = setup(&["main"]);
    let first = open(&mut core, &mut desktop, "one");
    desktop.apply(&Effect::Focus(Some(first))).unwrap();
    core.handle_window_event(&mut desktop, WindowEvent::Focused(first))
        .expect("focused");

    let second = desktop.open("two", "app.exe");
    desktop.apply(&Effect::Focus(Some(second))).unwrap();
    core.handle_window_event(&mut desktop, WindowEvent::Focused(second))
        .expect("focused before it opened");
    desktop.applied.clear();
    core.handle_window_event(&mut desktop, WindowEvent::Created(second))
        .expect("created");
    core.flush_pending_layouts(&mut desktop).expect("layout");

    assert_eq!(
        core.monitor_manager
            .get_focused_workspace(OUTPUT)
            .and_then(|workspace| workspace.focused_window()),
        Some(second)
    );
    assert!(
        desktop.applied.contains(&Effect::Border {
            id: second,
            change: BorderChange::Focused(true),
        }),
        "the border is told of the focus after the window has its place"
    );
}

// ---- the scenarios of the older `Engine` harness, on the real core ---------

fn focus(core: &mut WmCore, desktop: &mut FakeDesktop, id: WindowHandle) {
    desktop.apply(&Effect::Focus(Some(id))).unwrap();
    core.handle_window_event(desktop, WindowEvent::Focused(id))
        .expect("focused");
    core.flush_pending_layouts(desktop).expect("layout");
}

fn command(core: &mut WmCore, desktop: &mut FakeDesktop, command: DispatchCommand) {
    core.dispatch(desktop, command).expect("command");
    core.flush_pending_layouts(desktop).expect("layout");
}

fn switch_to(core: &mut WmCore, desktop: &mut FakeDesktop, name: &str) {
    command(
        core,
        desktop,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Name(name.to_string()),
        },
    );
}

#[test]
fn closing_a_window_gives_exactly_its_space_back() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let alone = desktop.window(editor).rect;
    let terminal = open(&mut core, &mut desktop, "terminal");
    assert_ne!(desktop.window(editor).rect, alone);
    assert_eq!(
        desktop.window(editor).rect.height,
        desktop.window(terminal).rect.height
    );

    desktop.close(terminal);
    core.handle_window_event(&mut desktop, WindowEvent::Destroyed(terminal))
        .unwrap();
    core.flush_pending_layouts(&mut desktop).unwrap();
    assert_eq!(desktop.window(editor).rect, alone);
}

#[test]
fn events_for_unknown_windows_change_nothing() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let before = desktop.window(editor).rect;
    let stranger = WindowHandle(999);
    for event in [
        WindowEvent::Destroyed(stranger),
        WindowEvent::Minimized(stranger),
        WindowEvent::Unmapped(stranger),
    ] {
        core.handle_window_event(&mut desktop, event)
            .expect("ignored");
    }
    core.flush_pending_layouts(&mut desktop).unwrap();
    assert_eq!(tiled(&core), [editor]);
    assert_eq!(desktop.window(editor).rect, before);
}

#[test]
fn a_window_the_app_hides_leaves_the_layout_and_is_adopted_again_when_shown() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let chat = open(&mut core, &mut desktop, "chat");

    desktop.windows.get_mut(&chat).unwrap().visible = false;
    core.handle_window_event(&mut desktop, WindowEvent::Unmapped(chat))
        .unwrap();
    core.flush_pending_layouts(&mut desktop).unwrap();
    assert_eq!(tiled(&core), [editor]);

    desktop.windows.get_mut(&chat).unwrap().visible = true;
    core.handle_window_event(&mut desktop, WindowEvent::Created(chat))
        .unwrap();
    core.flush_pending_layouts(&mut desktop).unwrap();
    assert_eq!(tiled(&core), [editor, chat]);
}

#[test]
fn a_window_sent_to_another_workspace_is_hidden_until_that_workspace_is_shown() {
    let (mut core, mut desktop) = setup(&["one", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let alone = desktop.window(editor).rect;
    let terminal = open(&mut core, &mut desktop, "terminal");
    focus(&mut core, &mut desktop, terminal);

    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceSelector {
            selector: WorkspaceSelector::Name("two".to_string()),
        },
    );
    assert!(
        !desktop.window(terminal).visible,
        "moved to a hidden workspace"
    );
    assert_eq!(tiled(&core), [editor]);
    assert_eq!(desktop.window(editor).rect, alone);

    switch_to(&mut core, &mut desktop, "two");
    assert!(!desktop.window(editor).visible && desktop.window(terminal).visible);
    assert_eq!(
        desktop.window(terminal).rect,
        alone,
        "alone on its workspace"
    );

    switch_to(&mut core, &mut desktop, "one");
    assert!(desktop.window(editor).visible && !desktop.window(terminal).visible);
}

#[test]
fn switching_to_the_current_workspace_moves_nothing() {
    let (mut core, mut desktop) = setup(&["one", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let before = desktop.window(editor).rect;
    desktop.applied.clear();
    switch_to(&mut core, &mut desktop, "one");
    assert!(desktop.window(editor).visible);
    assert_eq!(desktop.window(editor).rect, before);
    assert!(
        !desktop
            .applied
            .iter()
            .any(|effect| matches!(effect, Effect::SetVisible { visible: false, .. })),
        "nothing is hidden: {:?}",
        desktop.applied
    );
}

#[test]
fn a_window_destroyed_on_a_hidden_workspace_is_gone_when_it_is_shown() {
    let (mut core, mut desktop) = setup(&["one", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let terminal = open(&mut core, &mut desktop, "terminal");
    focus(&mut core, &mut desktop, terminal);
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceSelector {
            selector: WorkspaceSelector::Name("two".to_string()),
        },
    );
    desktop.close(terminal);
    core.handle_window_event(&mut desktop, WindowEvent::Destroyed(terminal))
        .unwrap();

    switch_to(&mut core, &mut desktop, "two");
    assert!(tiled(&core).is_empty());
    assert!(!desktop.window(editor).visible);
}

#[test]
fn a_stacked_workspace_shows_one_tab_at_a_time() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let full = desktop.window(editor).rect;
    let terminal = open(&mut core, &mut desktop, "terminal");
    let browser = open(&mut core, &mut desktop, "browser");
    focus(&mut core, &mut desktop, browser);
    let visible = |desktop: &FakeDesktop| -> Vec<WindowHandle> {
        [editor, terminal, browser]
            .into_iter()
            .filter(|id| desktop.window(*id).visible)
            .collect()
    };

    command(&mut core, &mut desktop, DispatchCommand::StackAll);
    assert_eq!(
        visible(&desktop),
        [browser],
        "the focused window stays in front"
    );
    assert_eq!(
        desktop.window(browser).rect,
        full,
        "one group takes the work area"
    );

    command(&mut core, &mut desktop, DispatchCommand::UnstackAll);
    assert_eq!(visible(&desktop), [editor, terminal, browser]);
    let rects = [editor, terminal, browser].map(|id| desktop.window(id).rect);
    assert!(
        !overlap(rects[0], rects[1])
            && !overlap(rects[1], rects[2])
            && !overlap(rects[0], rects[2])
    );
}

#[test]
fn layout_commands_change_the_tiling_of_the_active_workspace() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let terminal = open(&mut core, &mut desktop, "terminal");
    let before = (desktop.window(editor).rect, desktop.window(terminal).rect);
    let layout_before = core
        .monitor_manager
        .get_focused_workspace(OUTPUT)
        .unwrap()
        .layout;

    command(&mut core, &mut desktop, DispatchCommand::CycleNextLayout);
    let layout_after = core
        .monitor_manager
        .get_focused_workspace(OUTPUT)
        .unwrap()
        .layout;
    assert_ne!(layout_after, layout_before);
    let after = (desktop.window(editor).rect, desktop.window(terminal).rect);
    assert!(!overlap(after.0, after.1));

    command(
        &mut core,
        &mut desktop,
        DispatchCommand::CyclePreviousLayout,
    );
    assert_eq!(
        core.monitor_manager
            .get_focused_workspace(OUTPUT)
            .unwrap()
            .layout,
        layout_before
    );
    assert_eq!(
        (desktop.window(editor).rect, desktop.window(terminal).rect),
        before,
        "back to the same tiling"
    );
}

#[test]
fn a_stay_focused_window_takes_focus_back() {
    let (mut core, mut desktop) = setup(&["main"]);
    let player = open(&mut core, &mut desktop, "player");
    let editor = open(&mut core, &mut desktop, "editor");
    core.executor
        .window_state
        .get_mut(player)
        .stay_focused
        .static_value = Some(true);

    focus(&mut core, &mut desktop, player);
    focus(&mut core, &mut desktop, editor);
    assert_eq!(desktop.focused, Some(player));
}

#[test]
fn a_no_focus_window_hands_focus_back_to_the_previous_window() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let osd = open(&mut core, &mut desktop, "osd");
    core.executor
        .window_state
        .get_mut(osd)
        .no_focus
        .static_value = Some(true);

    focus(&mut core, &mut desktop, editor);
    focus(&mut core, &mut desktop, osd);
    assert_eq!(desktop.focused, Some(editor));
}

#[test]
fn a_window_tiles_on_the_output_it_opened_on() {
    use crate::outputs::MonitorInfo;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main"]);
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height - 40,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    core.monitor_manager
        .create_workspace(SECOND, "side".to_string(), LayoutType::BSP)
        .expect("workspace on the second output");
    desktop.more_outputs.push((SECOND, screen));

    let viewer = desktop.open_at("viewer", Rect::new(2200, 300, 800, 600));
    core.handle_window_event(&mut desktop, WindowEvent::Created(viewer))
        .unwrap();
    core.flush_pending_layouts(&mut desktop).unwrap();

    assert_eq!(
        core.monitor_manager
            .find_window(viewer)
            .map(|(output, _)| output),
        Some(SECOND)
    );
    let rect = desktop.window(viewer).rect;
    assert!(
        rect.x >= screen.x && rect.x + rect.width <= screen.x + screen.width,
        "placed on the second output: {rect:?}"
    );
    assert!(tiled(&core).is_empty(), "nothing on the first output");
}

#[test]
fn going_to_an_output_with_nothing_on_it_takes_the_keyboard_along() {
    use crate::outputs::MonitorInfo;
    use gevrek_model::command::MonitorTarget;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main"]);
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    core.monitor_manager
        .create_workspace(SECOND, "side".to_string(), LayoutType::BSP)
        .expect("workspace on the second output");
    desktop.more_outputs.push((SECOND, screen));

    let editor = open(&mut core, &mut desktop, "editor");
    focus(&mut core, &mut desktop, editor);
    assert_eq!(desktop.focused, Some(editor));

    // The second output shows an empty workspace: the keys must not stay
    // with the editor on the first.
    let to = |target: &str| DispatchCommand::FocusMonitor {
        target: MonitorTarget::Name(target.to_string()),
    };
    command(&mut core, &mut desktop, to("Second"));
    assert_eq!(core.executor.focused_monitor(), Some(SECOND));
    assert_eq!(desktop.focused, None, "nobody has the keyboard");
    assert!(desktop.window(editor).visible, "the editor stays shown");

    // And back: the editor has them again.
    command(&mut core, &mut desktop, to("Primary"));
    assert_eq!(core.executor.focused_monitor(), Some(OUTPUT));
    assert_eq!(desktop.focused, Some(editor));

    // By the workspace's name, which is how a panel asks.
    switch_to(&mut core, &mut desktop, "side");
    assert_eq!(core.executor.focused_monitor(), Some(SECOND));
    assert_eq!(desktop.focused, None);
}

/// A display that changes to a workspace with no window keeps the
/// keyboard. The window of the workspace that went had it, and is hidden
/// a moment later; Windows then gives the keyboard to a window of its own
/// choosing, here the one on the other display, which became the display
/// with the keyboard (reported by the person, 2026-11-20).
#[test]
fn a_workspace_with_no_window_keeps_the_keyboard_on_its_display() {
    use crate::outputs::MonitorInfo;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main", "empty"]);
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    core.monitor_manager
        .create_workspace(SECOND, "side".to_string(), LayoutType::BSP)
        .expect("workspace on the second output");
    desktop.more_outputs.push((SECOND, screen));

    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = open(&mut core, &mut desktop, "viewer");
    core.monitor_manager.find_and_remove_window(viewer);
    core.monitor_manager
        .get_focused_workspace_mut(SECOND)
        .expect("the second output's workspace")
        .add_window(viewer, true);
    focus(&mut core, &mut desktop, editor);
    assert_eq!(core.executor.focused_monitor(), Some(OUTPUT));

    desktop.hides_later = true;
    switch_to(&mut core, &mut desktop, "empty");
    // (The editor is hidden now, a moment after it was asked to be.)
    desktop.settle();
    assert_eq!(
        desktop.focused, None,
        "the keyboard is nobody's, not the other display's window's"
    );
    assert_eq!(core.executor.focused_monitor(), Some(OUTPUT));

    // Back: the editor has it again.
    switch_to(&mut core, &mut desktop, "main");
    desktop.settle();
    assert_eq!(desktop.focused, Some(editor));
    assert_eq!(core.executor.focused_monitor(), Some(OUTPUT));
}

#[test]
fn a_window_chosen_in_the_overview_ends_it_and_has_the_keyboard() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let terminal = open(&mut core, &mut desktop, "terminal");
    focus(&mut core, &mut desktop, editor);
    let before = (desktop.window(editor).rect, desktop.window(terminal).rect);

    // The overview: both windows shown smaller, each at its own shape,
    // side by side. It stays while nothing is chosen.
    command(&mut core, &mut desktop, DispatchCommand::ToggleExpose);
    let shown = (desktop.window(editor).rect, desktop.window(terminal).rect);
    assert!(shown.0.width < before.0.width && shown.1.width < before.1.width);
    assert!(shown.0.x + shown.0.width <= shown.1.x || shown.1.x + shown.1.width <= shown.0.x);
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(
        desktop.window(editor).rect,
        shown.0,
        "nothing lays it out meanwhile"
    );

    // The other window is chosen: the overview is over, both are where
    // they were, and it has the keyboard.
    focus(&mut core, &mut desktop, terminal);
    assert_eq!(
        (desktop.window(editor).rect, desktop.window(terminal).rect),
        before
    );
    assert_eq!(desktop.focused, Some(terminal));
    // Its key shows it again (it was over, not only looked over).
    command(&mut core, &mut desktop, DispatchCommand::ToggleExpose);
    assert_eq!(desktop.window(editor).rect, shown.0);
    // And puts it away, with nothing chosen.
    command(&mut core, &mut desktop, DispatchCommand::ToggleExpose);
    assert_eq!(
        (desktop.window(editor).rect, desktop.window(terminal).rect),
        before
    );
}

#[test]
fn a_window_dropped_with_the_pointer_on_another_output_goes_there() {
    use crate::outputs::MonitorInfo;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main"]);
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    core.monitor_manager
        .create_workspace(SECOND, "side".to_string(), LayoutType::BSP)
        .expect("workspace on the second output");
    desktop.more_outputs.push((SECOND, screen));

    let editor = open(&mut core, &mut desktop, "editor");
    assert_eq!(core.monitor_manager.find_window(editor), Some((OUTPUT, 0)));
    // Taken by its title and let go just beyond the edge: the window is
    // where it was (most of it on the first output, as the test's desktop
    // has it), and the pointer is on the second. That is where it was put.
    core.handle_window_event(&mut desktop, WindowEvent::DragStarted(editor))
        .expect("taken");
    assert_eq!(
        crate::contract::Desktop::output_of(&desktop, editor),
        Some(OUTPUT)
    );
    desktop.output = SECOND;
    core.handle_window_event(&mut desktop, WindowEvent::DragEnded(editor))
        .expect("let go");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(core.monitor_manager.find_window(editor), Some((SECOND, 0)));
    assert!(tiled(&core).is_empty(), "it has left the first output");
    // It is laid out there: within that output.
    let rect = desktop.window(editor).rect;
    assert!(rect.x >= screen.x && rect.x + rect.width <= screen.x + screen.width);
}

#[test]
fn a_layout_is_set_on_the_output_the_user_went_to_not_where_the_window_is() {
    use crate::outputs::MonitorInfo;
    use gevrek_model::command::MonitorTarget;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main"]);
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    core.monitor_manager
        .create_workspace(SECOND, "side".to_string(), LayoutType::BSP)
        .expect("workspace on the second output");
    desktop.more_outputs.push((SECOND, screen));
    let layout = |core: &WmCore, output| {
        core.monitor_manager
            .get_focused_workspace(output)
            .map(|workspace| workspace.layout)
    };

    let editor = open(&mut core, &mut desktop, "editor");
    focus(&mut core, &mut desktop, editor);
    // With the window focused, a layout is its workspace's.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Columns,
        },
    );
    assert_eq!(layout(&core, OUTPUT), Some(LayoutType::Columns));
    assert_eq!(layout(&core, SECOND), Some(LayoutType::BSP));

    // On the second output, whose workspace has no window: the layout is
    // that workspace's. Also should the editor still have the keyboard
    // (a desktop that would not let go of it).
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::FocusMonitor {
            target: MonitorTarget::Name("Second".to_string()),
        },
    );
    desktop.apply(&Effect::Focus(Some(editor))).unwrap();
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Grid,
        },
    );
    assert_eq!(layout(&core, SECOND), Some(LayoutType::Grid));
    assert_eq!(
        layout(&core, OUTPUT),
        Some(LayoutType::Columns),
        "untouched"
    );
}

#[test]
fn the_floating_layout_floats_what_is_there_and_centres_what_comes() {
    let (mut core, mut desktop) = setup(&["one", "two"]);
    let floating = |core: &WmCore| -> Vec<WindowHandle> {
        core.monitor_manager
            .get_focused_workspace(OUTPUT)
            .map(|workspace| workspace.floating_windows.iter().copied().collect())
            .unwrap_or_default()
    };
    let centred = |desktop: &FakeDesktop, id: WindowHandle| {
        let rect = desktop.window(id).rect;
        // The fake output is 1920x1080, with 40 kept free at its foot.
        (
            (rect.x + rect.width / 2 - 960).abs() <= 1,
            (rect.y + rect.height / 2 - 520).abs() <= 1,
            rect,
        )
    };
    let editor = open(&mut core, &mut desktop, "editor");
    let terminal = open(&mut core, &mut desktop, "terminal");
    focus(&mut core, &mut desktop, terminal);
    let tiles = (desktop.window(editor).rect, desktop.window(terminal).rect);
    assert_eq!(tiled(&core).len(), 2);

    // Chosen: the two float, where they are.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Floating,
        },
    );
    assert!(tiled(&core).is_empty(), "nothing is tiled");
    assert_eq!(floating(&core).len(), 2);
    assert_eq!(
        (desktop.window(editor).rect, desktop.window(terminal).rect),
        tiles,
        "the windows that were there stay where they are"
    );

    // A window that opens there floats in the middle, at its own size.
    let viewer = open(&mut core, &mut desktop, "viewer");
    assert!(floating(&core).contains(&viewer) && tiled(&core).is_empty());
    let (across, down, rect) = centred(&desktop, viewer);
    assert!(across && down, "in the middle: {rect:?}");
    assert_eq!((rect.width, rect.height), (640, 480), "at its own size");

    // One that is moved there from another workspace too.
    switch_to(&mut core, &mut desktop, "two");
    let player = open(&mut core, &mut desktop, "player");
    focus(&mut core, &mut desktop, player);
    assert_eq!(tiled(&core), vec![player], "tiled on the other workspace");
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceAndFollow {
            selector: WorkspaceSelector::Name("one".to_string()),
        },
    );
    assert!(floating(&core).contains(&player) && tiled(&core).is_empty());
    let (across, down, rect) = centred(&desktop, player);
    assert!(across && down, "the moved one is in the middle: {rect:?}");
    // The others were not moved by its coming.
    assert_eq!(desktop.window(editor).rect, tiles.0);

    // Moved away to a workspace that tiles, a window that only floated
    // because of the layout is tiled there, and obeys that layout.
    focus(&mut core, &mut desktop, player);
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceAndFollow {
            selector: WorkspaceSelector::Name("two".to_string()),
        },
    );
    assert_eq!(
        tiled(&core),
        vec![player],
        "tiled on the workspace that tiles"
    );
    assert!(floating(&core).is_empty());
    // And back under the Floating layout it floats again, in the middle.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceAndFollow {
            selector: WorkspaceSelector::Name("one".to_string()),
        },
    );
    assert!(floating(&core).contains(&player) && tiled(&core).is_empty());
    assert!(centred(&desktop, player).0, "in the middle again");

    // Another layout: all of them are tiled again.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Columns,
        },
    );
    assert!(
        floating(&core).is_empty(),
        "nothing floats: {:?}",
        floating(&core)
    );
    assert_eq!(tiled(&core).len(), 4);
}

#[test]
fn floating_windows_go_out_of_the_way_and_come_back() {
    let (mut core, mut desktop) = setup(&["one", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let left = open(&mut core, &mut desktop, "left");
    let right = open(&mut core, &mut desktop, "right");
    for (window, x) in [(left, 100), (right, 1200)] {
        focus(&mut core, &mut desktop, window);
        command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
        desktop
            .apply(&Effect::Configure {
                id: window,
                rect: Rect::new(x, 200, 600, 400),
                states: Default::default(),
            })
            .unwrap();
    }
    let tile = desktop.window(editor).rect;
    let before = (desktop.window(left).rect, desktop.window(right).rect);
    focus(&mut core, &mut desktop, right);

    command(
        &mut core,
        &mut desktop,
        DispatchCommand::ToggleFloatingAside,
    );
    let (l, r) = (desktop.window(left).rect, desktop.window(right).rect);
    // A tenth of each stays in sight, at the side it was nearer to.
    assert_eq!((l.x + l.width, l.width, l.y), (60, 600, 200), "left: {l:?}");
    assert_eq!((1920 - r.x, r.width, r.y), (60, 600, 200), "right: {r:?}");
    assert_eq!(desktop.window(editor).rect, tile, "the tiled window stays");
    assert_eq!(desktop.focused, Some(editor), "and has the keyboard");

    // Again: back where they were.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::ToggleFloatingAside,
    );
    assert_eq!(
        (desktop.window(left).rect, desktop.window(right).rect),
        before
    );

    // Put away, then another workspace shown: they are back when this
    // one is seen again.
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::ToggleFloatingAside,
    );
    assert_ne!(desktop.window(left).rect, before.0);
    switch_to(&mut core, &mut desktop, "two");
    switch_to(&mut core, &mut desktop, "one");
    assert_eq!(
        (desktop.window(left).rect, desktop.window(right).rect),
        before
    );
    // And the next press puts them away, it does not "bring back".
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::ToggleFloatingAside,
    );
    assert_ne!(desktop.window(left).rect, before.0);
}

#[test]
fn an_app_that_refuses_its_size_is_not_placed_again_and_again() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let fixed = desktop.open("fixed", "fixed.exe");
    desktop.windows.get_mut(&fixed).unwrap().min_width = Some(1200);
    core.handle_window_event(&mut desktop, WindowEvent::Created(fixed))
        .unwrap();
    core.flush_pending_layouts(&mut desktop).unwrap();
    assert!(desktop.window(fixed).rect.width >= 1200);
    assert_eq!(tiled(&core), [editor, fixed]);

    // The application reports the size it kept; that must not start a
    // loop of new placements.
    let placed = desktop.placements;
    let kept = desktop.window(fixed).rect;
    for _ in 0..5 {
        core.handle_window_event(
            &mut desktop,
            WindowEvent::Resized(fixed, kept.width, kept.height),
        )
        .unwrap();
        core.flush_pending_layouts(&mut desktop).unwrap();
    }
    assert!(
        desktop.placements <= placed + 2,
        "placed {} more times",
        desktop.placements - placed
    );
}

#[test]
fn a_floating_window_moves_with_its_workspace_by_the_distance_between_the_outputs() {
    use crate::outputs::MonitorInfo;
    use gevrek_model::command::MonitorTarget;
    const SECOND: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&["main", "spare"]);
    let screen = Rect::new(1920, 0, 1920, 1080);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height - 40,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    core.monitor_manager
        .create_workspace(SECOND, "side".to_string(), LayoutType::BSP)
        .expect("workspace on the second output");
    desktop.more_outputs.push((SECOND, screen));

    let note = open(&mut core, &mut desktop, "note");
    focus(&mut core, &mut desktop, note);
    command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
    assert!(core.monitor_manager.is_window_floating(note));
    let before = desktop.window(note).rect;

    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveWorkspaceToMonitor {
            target: MonitorTarget::Index(1),
        },
    );
    let after = desktop.window(note).rect;
    assert_eq!(
        core.monitor_manager
            .find_window(note)
            .map(|(output, _)| output),
        Some(SECOND)
    );
    assert_eq!(
        (after.x - before.x, after.y - before.y),
        (screen.x, screen.y),
        "shifted by exactly the distance between the outputs: {before:?} -> {after:?}"
    );
    assert_eq!((after.width, after.height), (before.width, before.height));
}

#[test]
fn after_a_restart_the_saved_session_puts_windows_back_on_their_workspaces() {
    let (mut core, mut desktop) = setup(&["one", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let terminal = open(&mut core, &mut desktop, "terminal");
    focus(&mut core, &mut desktop, terminal);
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceSelector {
            selector: WorkspaceSelector::Name("two".to_string()),
        },
    );
    let saved = core.session_state(&desktop);

    // The WM stops (hidden windows are shown again) and starts anew: every
    // open window is adopted onto the visible workspace.
    desktop
        .apply(&Effect::SetVisible {
            id: terminal,
            visible: true,
        })
        .unwrap();
    let (mut core, _) = setup(&["one", "two"]);
    assert_eq!(core.adopt_existing_windows(&mut desktop).unwrap(), 2);
    assert_eq!(tiled(&core).len(), 2);

    let moved = core.restore_session(&mut desktop, &saved).unwrap();
    assert_eq!(moved, 1);
    assert_eq!(tiled(&core), [editor]);
    assert_eq!(
        core.monitor_manager.find_window(terminal),
        Some((OUTPUT, 1)),
        "back on workspace 'two'"
    );
    assert!(!desktop.window(terminal).visible && desktop.window(editor).visible);

    // The saved position is wrong but the name is right: the name decides.
    let (mut core, _) = setup(&["one", "two"]);
    core.adopt_existing_windows(&mut desktop).unwrap();
    let mut reordered = saved.clone();
    for window in &mut reordered.windows {
        if window.handle == terminal {
            assert_eq!(window.workspace_name.as_deref(), Some("two"));
            window.workspace_id = Some(0);
        }
    }
    assert_eq!(core.restore_session(&mut desktop, &reordered).unwrap(), 1);
    assert_eq!(
        core.monitor_manager.find_window(terminal),
        Some((OUTPUT, 1))
    );
    // A workspace that no longer exists by that name: the window stays.
    let mut gone = saved.clone();
    for window in &mut gone.windows {
        window.workspace_name = Some("removed".to_string());
    }
    assert_eq!(core.restore_session(&mut desktop, &gone).unwrap(), 0);

    // The display has another handle than when the session was saved
    // (the machine was started again): it is found by what it is known
    // as, and the window goes back, and the workspace it showed is shown.
    let (mut core, _) = setup(&["one", "two"]);
    core.adopt_existing_windows(&mut desktop).unwrap();
    let mut elsewhere = saved.clone();
    let other = MonitorHandle(OUTPUT.0 + 4711);
    for window in &mut elsewhere.windows {
        assert!(window.monitor_known_as.is_some());
        window.monitor = window.monitor.map(|_| other);
    }
    for shown in &mut elsewhere.shown_workspaces {
        shown.monitor = other;
    }
    elsewhere.focused_workspaces.clear();
    assert_eq!(core.restore_session(&mut desktop, &elsewhere).unwrap(), 1);
    assert_eq!(
        core.monitor_manager.find_window(terminal),
        Some((OUTPUT, 1))
    );
    assert_eq!(
        core.monitor_manager
            .get_workspaces(OUTPUT)
            .map(|workspaces| workspaces.focused_idx()),
        saved.focused_workspaces.get(&OUTPUT).copied()
    );
    // A display that is known as nothing there is, is none: nothing moves.
    let (mut core, _) = setup(&["one", "two"]);
    core.adopt_existing_windows(&mut desktop).unwrap();
    let mut nowhere = elsewhere.clone();
    for window in &mut nowhere.windows {
        window.monitor_known_as = Some("a display that is not there".to_string());
    }
    nowhere.shown_workspaces.clear();
    assert_eq!(core.restore_session(&mut desktop, &nowhere).unwrap(), 0);

    // A window that is no longer the same window is left where it is
    // (the other one is back on its workspace first).
    assert_eq!(core.restore_session(&mut desktop, &saved).unwrap(), 1);
    let mut stale = saved.clone();
    stale.windows[0].class = "SomethingElse".to_string();
    stale.windows[0].workspace_id = Some(1);
    assert_eq!(core.restore_session(&mut desktop, &stale).unwrap(), 0);
    assert_eq!(tiled(&core), [editor]);
}

#[test]
fn a_rule_on_the_application_id_matches_on_any_backend() {
    use crate::rules::{MatchKind, MatchStrategy, WindowRule};
    let rule = WindowRule::new("id".to_string(), MatchKind::AppId, MatchStrategy::Equals);
    let (_core, mut desktop) = setup(&["one"]);
    // The fake desktop's windows are `app.exe`: their id is `app`.
    let id = desktop.open("note", "app.exe");
    let facts = crate::contract::WindowQueries::rule_facts(&desktop, id).expect("facts");
    assert_eq!(facts.app_id, "app");
    assert_eq!(crate::contract::WindowQueries::app_id(&desktop, id), "app");
    let mut engine = RuleEngine::new();
    let mut by_id = rule;
    by_id.id = "app".to_string();
    engine.add_ignore_rule(crate::rules::IgnoreRule::new(by_id));
    assert!(engine.should_ignore(&facts));
    let mut other = facts.clone();
    other.app_id = "something-else".to_string();
    assert!(!engine.should_ignore(&other));
}

/// A workspace that every display has is given another title on one of
/// them (as the panel does: an entry of that display's own, in the
/// configuration, for the same workspace).
#[test]
fn a_workspace_titled_on_one_display_keeps_its_name_and_its_windows() {
    use gevrek_model::settings::{AppearanceProfile, MonitorSelector, WorkspaceProfile};
    const SIDE: MonitorHandle = MonitorHandle(2);
    let (mut core, mut desktop) = setup(&[]);
    core.monitor_manager.add_test_monitor(SIDE, "Side");
    let profile = |name: &str| WorkspaceProfile {
        name: name.to_string(),
        monitor: MonitorSelector::Any,
        persistent: true,
        ..WorkspaceProfile::default()
    };
    let mut profiles = vec![profile("1"), profile("2")];
    let appearance = AppearanceProfile::default();
    core.executor
        .set_workspace_profiles(profiles.clone(), appearance.clone());
    core.apply_workspace_profile_set(&profiles, &appearance, |_| true, true);
    let names = |core: &WmCore, output: MonitorHandle| -> Vec<String> {
        core.monitor_manager
            .get_workspaces(output)
            .map(|workspaces| workspaces.iter().map(|w| w.name.clone()).collect())
            .unwrap_or_default()
    };
    let labels = |core: &WmCore, output: MonitorHandle| -> Vec<String> {
        core.monitor_manager
            .get_workspaces(output)
            .map(|workspaces| workspaces.iter().map(|w| w.label().to_string()).collect())
            .unwrap_or_default()
    };
    assert_eq!(names(&core, OUTPUT), ["1", "2"]);
    assert_eq!(names(&core, SIDE), ["1", "2"]);
    let window = open(&mut core, &mut desktop, "a window");

    // The first display's workspace 1 is shown as "web" from now on, and
    // then, titled a second time, as "mail".
    profiles.insert(
        1,
        WorkspaceProfile {
            title: Some("web".to_string()),
            monitor: MonitorSelector::Name("Primary".to_string()),
            ..profile("1")
        },
    );
    for title in ["web", "mail"] {
        profiles[1].title = Some(title.to_string());
        core.executor
            .set_workspace_profiles(profiles.clone(), appearance.clone());
        core.apply_workspace_profile_set(&profiles, &appearance, |_| true, false);
        // Where it was, with its window, and by the name it had; no
        // second workspace made beside it; the other display's as before.
        assert_eq!(labels(&core, OUTPUT), [title, "2"]);
        assert_eq!(names(&core, OUTPUT), ["1", "2"]);
        assert_eq!(labels(&core, SIDE), ["1", "2"]);
        assert_eq!(core.monitor_manager.find_window(window), Some((OUTPUT, 0)));
    }

    // The key for workspace 1, pressed on the first display, shows it;
    // and so does asking for it by what it is shown as.
    for asked in ["1", "mail"] {
        core.dispatch(
            &mut desktop,
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("2".to_string()),
            },
        )
        .expect("to 2");
        core.dispatch(
            &mut desktop,
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name(asked.to_string()),
            },
        )
        .expect("to 1");
        assert_eq!(
            core.monitor_manager
                .get_focused_workspace(OUTPUT)
                .map(|workspace| workspace.label().to_string()),
            Some("mail".to_string())
        );
        assert_eq!(names(&core, OUTPUT), ["1", "2"]);
    }
}

/// What the configuration says of a display's workspaces is how they are
/// after it is applied: one taken out is gone, another order is theirs.
#[test]
fn a_displays_workspaces_follow_the_configuration() {
    use gevrek_model::settings::{AppearanceProfile, MonitorSelector, WorkspaceProfile};
    let (mut core, mut desktop) = setup(&[]);
    let profile = |name: &str| WorkspaceProfile {
        name: name.to_string(),
        monitor: MonitorSelector::Any,
        persistent: true,
        ..WorkspaceProfile::default()
    };
    let appearance = AppearanceProfile::default();
    let apply = |core: &mut WmCore, profiles: &[WorkspaceProfile], first: bool| {
        core.executor
            .set_workspace_profiles(profiles.to_vec(), appearance.clone());
        core.apply_workspace_profile_set(profiles, &appearance, |_| true, first);
    };
    let names = |core: &WmCore| -> Vec<String> {
        core.monitor_manager
            .get_workspaces(OUTPUT)
            .map(|workspaces| workspaces.iter().map(|w| w.name.clone()).collect())
            .unwrap_or_default()
    };
    let shown = |core: &WmCore| {
        core.monitor_manager
            .get_focused_workspace(OUTPUT)
            .map(|workspace| workspace.name.clone())
    };
    let mut first = profile("1");
    first.default = true;
    apply(
        &mut core,
        &[first.clone(), profile("2"), profile("3"), profile("4")],
        true,
    );
    assert_eq!(names(&core), ["1", "2", "3", "4"]);
    let window = open(&mut core, &mut desktop, "a window");
    assert_eq!(shown(&core).as_deref(), Some("1"));

    // 2 is taken out: empty and not shown, it goes.
    apply(
        &mut core,
        &[first.clone(), profile("3"), profile("4")],
        false,
    );
    assert_eq!(names(&core), ["1", "3", "4"]);

    // Another order: theirs too, and the one shown stays shown.
    apply(
        &mut core,
        &[profile("4"), profile("3"), first.clone()],
        false,
    );
    assert_eq!(names(&core), ["4", "3", "1"]);
    assert_eq!(shown(&core).as_deref(), Some("1"));

    // The one that is shown and has a window is taken out: it stays, as a
    // workspace of the moment, after the configuration's.
    apply(&mut core, &[profile("4"), profile("3")], false);
    assert_eq!(names(&core), ["4", "3", "1"]);
    assert_eq!(core.monitor_manager.find_window(window), Some((OUTPUT, 2)));
    let left = core
        .monitor_manager
        .get_workspaces(OUTPUT)
        .and_then(|workspaces| workspaces.get(2))
        .map(|workspace| (workspace.configured, workspace.persistent));
    assert_eq!(left, Some((false, false)));
    // Back in the configuration, it is one of its own again.
    apply(&mut core, &[profile("4"), first, profile("3")], false);
    assert_eq!(names(&core), ["4", "1", "3"]);
    assert_eq!(core.monitor_manager.find_window(window), Some((OUTPUT, 1)));
}

/// A workspace nobody named is the next the configuration has for the
/// display, and after those a number of its own; no two of a display are
/// named alike; one of the moment goes when it is left empty.
#[test]
fn workspaces_of_the_moment_are_numbered_and_go_when_left_empty() {
    use gevrek_model::settings::{AppearanceProfile, MonitorSelector, WorkspaceProfile};
    let (mut core, mut desktop) = setup(&[]);
    let profile = |name: &str, persistent: bool| WorkspaceProfile {
        name: name.to_string(),
        monitor: MonitorSelector::Any,
        persistent,
        gaps_out: 17,
        ..WorkspaceProfile::default()
    };
    let profiles = vec![profile("1", true), profile("2", true), profile("3", false)];
    let appearance = AppearanceProfile::default();
    core.executor
        .set_workspace_profiles(profiles.clone(), appearance.clone());
    core.apply_workspace_profile_set(&profiles, &appearance, |_| true, true);
    let names = |core: &WmCore| -> Vec<String> {
        core.monitor_manager
            .get_workspaces(OUTPUT)
            .map(|workspaces| workspaces.iter().map(|w| w.name.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(names(&core), ["1", "2"]);
    let new = |core: &mut WmCore, desktop: &mut FakeDesktop, name: Option<&str>| {
        core.dispatch(
            desktop,
            DispatchCommand::NewWorkspace {
                name: name.map(str::to_string),
            },
        )
        .expect("a new workspace");
    };
    // The one the configuration has waiting, then a number of its own.
    new(&mut core, &mut desktop, None);
    assert_eq!(names(&core), ["1", "2", "3"]);
    new(&mut core, &mut desktop, None);
    assert_eq!(names(&core), ["1", "2", "3", "4"]);
    // Asked for by a name that is there: no second one.
    new(&mut core, &mut desktop, Some("4"));
    new(&mut core, &mut desktop, Some(" 2 "));
    assert_eq!(names(&core), ["1", "2", "3", "4"]);
    // It is laid out as the display's others are, not as nothing says.
    let made = core
        .monitor_manager
        .get_workspaces(OUTPUT)
        .and_then(|workspaces| workspaces.get(3))
        .map(|workspace| {
            (
                workspace.padding.left,
                workspace.configured,
                workspace.persistent,
            )
        });
    assert_eq!(made, Some((17, false, false)));

    // Shown, and left empty: gone. With a window it stays.
    let to = |core: &mut WmCore, desktop: &mut FakeDesktop, name: &str| {
        core.dispatch(
            desktop,
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name(name.to_string()),
            },
        )
        .expect("shown");
    };
    to(&mut core, &mut desktop, "4");
    to(&mut core, &mut desktop, "1");
    assert_eq!(names(&core), ["1", "2", "3"]);
    to(&mut core, &mut desktop, "7");
    open(&mut core, &mut desktop, "a window");
    to(&mut core, &mut desktop, "1");
    assert_eq!(names(&core), ["1", "2", "3", "7"]);
    // The next one has the number of its place.
    new(&mut core, &mut desktop, None);
    assert_eq!(names(&core), ["1", "2", "3", "7", "5"]);
}

/// A second display beside the first, with the workspaces `names`.
fn second_display(core: &mut WmCore, desktop: &mut FakeDesktop, names: &[&str]) -> Rect {
    let screen = Rect::new(1920, 0, 2560, 1440);
    let mut monitors = core.monitor_manager.get_all_monitors();
    monitors.push(crate::outputs::MonitorInfo {
        handle: BESIDE,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: screen.x,
        y: screen.y,
        width: screen.width,
        height: screen.height,
        work_x: screen.x,
        work_y: screen.y,
        work_width: screen.width,
        work_height: screen.height,
        dpi_scale: 1.0,
    });
    core.monitor_manager.replace_monitors(monitors);
    for name in names {
        core.monitor_manager
            .create_workspace(BESIDE, name.to_string(), LayoutType::BSP)
            .expect("workspace on the second output");
    }
    desktop.more_outputs.push((BESIDE, screen));
    screen
}

const BESIDE: MonitorHandle = MonitorHandle(2);

/// What the window manager publishes of its outputs (what each panel
/// draws its own output's part of), after it was looked at for a window
/// listed twice.
fn published(core: &WmCore, desktop: &FakeDesktop) -> Vec<crate::overlay_data::MonitorData> {
    let (monitors, _) = core.monitor_data(desktop, &Default::default());
    assert_eq!(crate::overlay_data::listed_amiss(&monitors), None);
    monitors
}

/// The output and the workspace a window is published for.
fn listed(
    monitors: &[crate::overlay_data::MonitorData],
    window: WindowHandle,
) -> Option<(MonitorHandle, String)> {
    crate::overlay_data::homes(monitors)
        .get(&window)
        .map(|home| (home.output, home.workspace_name.to_string()))
}

/// The windows that were told as being on another workspace or output
/// between two published states.
fn told_moved(
    before: &[crate::overlay_data::MonitorData],
    after: &[crate::overlay_data::MonitorData],
) -> Vec<String> {
    crate::overlay_data::changes(before, after)
        .into_iter()
        .filter(|(kind, _)| *kind == crate::executor::RuntimeEventKind::WindowWorkspaceChanged)
        .filter_map(|(_, fields)| {
            fields
                .into_iter()
                .find(|(name, _)| name == "window")
                .map(|(_, window)| window)
        })
        .collect()
}

/// A window is published for the output its workspace is on, whatever
/// its rectangle says: across the edge with most of it on the other
/// output, somewhere else while its workspace is put away, far off
/// while it is minimized. (A panel that went by where a window lies
/// would show it on the other output's taskbar in each of these.)
#[test]
fn a_window_is_listed_for_its_workspaces_output_wherever_it_lies() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    second_display(&mut core, &mut desktop, &["side", "more"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let notes = open(&mut core, &mut desktop, "notes");
    let home = Some((OUTPUT, "main".to_string()));
    let first = published(&core, &desktop);
    assert_eq!(listed(&first, editor), home);
    assert_eq!(listed(&first, notes), home);

    // Across the edge, with most of it on the second output. Nobody
    // dragged it there (its program moved it): it is where it was.
    desktop.windows.get_mut(&notes).unwrap().rect = Rect::new(1700, 200, 1600, 900);
    core.handle_window_event(&mut desktop, WindowEvent::Moved(notes, 1700, 200))
        .expect("moved");
    assert_eq!(Desktop::output_of(&desktop, notes), Some(BESIDE));
    let across = published(&core, &desktop);
    assert_eq!(listed(&across, notes), home);
    assert!(told_moved(&first, &across).is_empty());

    // Its workspace is put away, and its windows lie on the other output
    // on their way (as they do while a workspace slides).
    switch_to(&mut core, &mut desktop, "two");
    for window in [editor, notes] {
        desktop.windows.get_mut(&window).unwrap().rect = Rect::new(2600, 1500, 900, 700);
    }
    let away = published(&core, &desktop);
    assert_eq!(listed(&away, editor), home);
    assert_eq!(listed(&away, notes), home);
    assert_eq!(away[0].workspace.name, "two");
    assert!(away[1]
        .workspaces
        .iter()
        .all(|workspace| workspace.detailed_windows.is_empty()));
    assert!(told_moved(&across, &away).is_empty());

    // Back, and minimized: its rectangle is nowhere on the desktop.
    switch_to(&mut core, &mut desktop, "main");
    {
        let window = desktop.windows.get_mut(&editor).unwrap();
        window.minimized = true;
        window.rect = Rect::new(-32000, -32000, 160, 28);
    }
    core.handle_window_event(&mut desktop, WindowEvent::Minimized(editor))
        .expect("minimized");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    let minimized = published(&core, &desktop);
    assert!(
        listed(&minimized, editor).is_none_or(|(output, _)| output == OUTPUT),
        "never the other output's: {:?}",
        listed(&minimized, editor)
    );
    assert_eq!(listed(&minimized, notes), home);
}

// ---- minimized windows on the panel ----------------------------------------

/// The person minimizes a window (the system tells it, as it does).
fn minimize(core: &mut WmCore, desktop: &mut FakeDesktop, window: WindowHandle) {
    desktop
        .windows
        .get_mut(&window)
        .expect("the window")
        .minimized = true;
    core.handle_window_event(desktop, WindowEvent::Minimized(window))
        .expect("minimized");
    core.flush_pending_layouts(desktop).expect("layout");
}

/// A press on a window's button in the taskbar: what the panel sends.
fn press(core: &mut WmCore, desktop: &mut FakeDesktop, window: WindowHandle) {
    command(
        core,
        desktop,
        DispatchCommand::FocusWindow {
            selector: gevrek_model::command::WindowSelector::Handle(window.0 as i64),
        },
    );
}

/// What is published of a window, wherever it is listed.
fn entry(
    monitors: &[crate::overlay_data::MonitorData],
    window: WindowHandle,
) -> Option<&crate::overlay_data::DetailedWindowInfo> {
    monitors
        .iter()
        .flat_map(|monitor| &monitor.workspaces)
        .flat_map(|workspace| &workspace.detailed_windows)
        .find(|listed| listed.handle == window)
}

/// The windows a workspace is published with, in their order.
fn listed_on(monitors: &[crate::overlay_data::MonitorData], name: &str) -> Vec<WindowHandle> {
    monitors
        .iter()
        .flat_map(|monitor| &monitor.workspaces)
        .find(|workspace| workspace.name == name)
        .map(|workspace| {
            workspace
                .detailed_windows
                .iter()
                .map(|w| w.handle)
                .collect()
        })
        .unwrap_or_default()
}

/// A tiled window the person minimized leaves the layout and is listed
/// still, once, under its workspace and where it stood (its button stays
/// on the panel, in its place, and says it is minimized). A press on it
/// brings it back to its tile and gives it the keyboard.
#[test]
fn a_minimized_tile_is_listed_in_its_place_and_a_press_brings_it_back() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let notes = open(&mut core, &mut desktop, "notes");
    let mail = open(&mut core, &mut desktop, "mail");
    minimize(&mut core, &mut desktop, notes);
    assert_eq!(tiled(&core), [editor, mail], "it left the layout");

    let minimized = published(&core, &desktop);
    assert_eq!(
        listed(&minimized, notes),
        Some((OUTPUT, "main".to_string()))
    );
    assert_eq!(listed_on(&minimized, "main"), [editor, notes, mail]);
    let listed_notes = entry(&minimized, notes).expect("listed");
    assert!(listed_notes.is_minimized && !listed_notes.is_focused);
    assert!(!entry(&minimized, editor).expect("listed").is_minimized);
    assert!(listed_on(&minimized, "two").is_empty());

    press(&mut core, &mut desktop, notes);
    let mut back = tiled(&core);
    back.sort();
    assert_eq!(back, [editor, notes, mail], "back in a tile");
    assert!(
        !desktop.window(notes).minimized,
        "the system shows it again"
    );
    assert_eq!(desktop.focused, Some(notes));
    let restored = published(&core, &desktop);
    let listed_notes = entry(&restored, notes).expect("listed");
    assert!(!listed_notes.is_minimized && listed_notes.is_focused);
    assert_eq!(listed_on(&restored, "main").len(), 3, "listed once");
    assert!(core
        .executor
        .window_state
        .get(notes)
        .minimized_origin
        .is_none());

    // The system telling of the restore afterwards changes nothing more.
    core.handle_window_event(&mut desktop, WindowEvent::Restored(notes))
        .expect("restored");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(tiled(&core).len(), 3);
    assert_eq!(listed_on(&published(&core, &desktop), "main").len(), 3);
}

/// One minimized on a workspace that is then put away is listed under
/// that workspace (as its other windows are), and a press shows its
/// workspace and brings it back there, not to the one shown.
#[test]
fn a_window_minimized_on_a_workspace_put_away_comes_back_to_its_own() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let notes = open(&mut core, &mut desktop, "notes");
    minimize(&mut core, &mut desktop, notes);
    switch_to(&mut core, &mut desktop, "two");
    let away = published(&core, &desktop);
    assert_eq!(away[0].workspace.name, "two");
    assert_eq!(listed_on(&away, "main"), [editor, notes]);
    assert!(entry(&away, notes).expect("listed").is_minimized);
    assert!(listed_on(&away, "two").is_empty());

    press(&mut core, &mut desktop, notes);
    let back = published(&core, &desktop);
    assert_eq!(back[0].workspace.name, "main", "its workspace is shown");
    let mut on_main = tiled(&core);
    on_main.sort();
    assert_eq!(on_main, [editor, notes]);
    assert!(!entry(&back, notes).expect("listed").is_minimized);
    assert_eq!(desktop.focused, Some(notes));
}

/// One minimized on the other display is that display's panel's alone,
/// and comes back there.
#[test]
fn a_window_minimized_on_the_other_display_is_listed_and_brought_back_there() {
    let (mut core, mut desktop) = setup(&["main"]);
    second_display(&mut core, &mut desktop, &["side"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = desktop.open_at("viewer", Rect::new(2200, 300, 800, 600));
    core.handle_window_event(&mut desktop, WindowEvent::Created(viewer))
        .expect("created");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(
        core.monitor_manager
            .find_window(viewer)
            .map(|(output, _)| output),
        Some(BESIDE)
    );
    focus(&mut core, &mut desktop, editor);
    minimize(&mut core, &mut desktop, viewer);
    let minimized = published(&core, &desktop);
    assert_eq!(
        listed(&minimized, viewer),
        Some((BESIDE, "side".to_string()))
    );
    assert_eq!(listed_on(&minimized, "main"), [editor]);

    press(&mut core, &mut desktop, viewer);
    assert_eq!(
        core.monitor_manager
            .find_window(viewer)
            .map(|(output, _)| output),
        Some(BESIDE),
        "back on its own display"
    );
    assert_eq!(core.executor.focused_monitor(), Some(BESIDE));
    assert_eq!(desktop.focused, Some(viewer));
    assert!(
        !entry(&published(&core, &desktop), viewer)
            .expect("listed")
            .is_minimized
    );
}

/// The display of the workspace a window was minimized on goes: it is
/// listed on one panel or on none, never on two, and a press brings it
/// back on a display that is there.
#[test]
fn a_window_minimized_on_a_display_that_went_is_listed_once_at_most() {
    let (mut core, mut desktop) = setup(&["main"]);
    second_display(&mut core, &mut desktop, &["side"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = desktop.open_at("viewer", Rect::new(2200, 300, 800, 600));
    core.handle_window_event(&mut desktop, WindowEvent::Created(viewer))
        .expect("created");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    minimize(&mut core, &mut desktop, viewer);

    let monitors: Vec<_> = core
        .monitor_manager
        .get_all_monitors()
        .into_iter()
        .filter(|monitor| monitor.handle != BESIDE)
        .collect();
    desktop.more_outputs.clear();
    let changes = core.monitor_manager.reconcile_with(monitors);
    core.executor
        .apply_monitor_changes(&mut desktop, &core.monitor_manager, &changes);
    core.flush_pending_layouts(&mut desktop).expect("layout");

    // (`published` holds that nothing is listed twice.)
    let one = published(&core, &desktop);
    assert_eq!(one.len(), 1);
    assert!(
        listed(&one, viewer).is_none_or(|(output, _)| output == OUTPUT),
        "{:?}",
        listed(&one, viewer)
    );
    assert_eq!(listed(&one, editor).map(|(output, _)| output), Some(OUTPUT));

    press(&mut core, &mut desktop, viewer);
    assert_eq!(
        core.monitor_manager
            .find_window(viewer)
            .map(|(output, _)| output),
        Some(OUTPUT)
    );
    assert!(!desktop.window(viewer).minimized);
    assert_eq!(
        listed(&published(&core, &desktop), viewer).map(|(o, _)| o),
        Some(OUTPUT)
    );
}

/// A floating window stays a member while it is minimized: listed once,
/// as minimized, and a press brings it back.
#[test]
fn a_minimized_floating_window_is_listed_as_minimized_and_brought_back() {
    let (mut core, mut desktop) = setup(&["main"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = open(&mut core, &mut desktop, "viewer");
    focus(&mut core, &mut desktop, viewer);
    command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
    minimize(&mut core, &mut desktop, viewer);
    let minimized = published(&core, &desktop);
    assert_eq!(listed_on(&minimized, "main"), [editor, viewer]);
    let listed_viewer = entry(&minimized, viewer).expect("listed");
    assert!(listed_viewer.is_minimized && listed_viewer.is_floating);

    press(&mut core, &mut desktop, viewer);
    assert!(!desktop.window(viewer).minimized);
    assert_eq!(desktop.focused, Some(viewer));
    let back = published(&core, &desktop);
    assert!(!entry(&back, viewer).expect("listed").is_minimized);
    assert_eq!(listed_on(&back, "main"), [editor, viewer]);
}

/// A window the system minimized while its workspace was put away is not
/// told as minimized: it comes back with its workspace.
#[test]
fn a_window_minimized_while_its_workspace_is_away_is_not_told_as_minimized() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    switch_to(&mut core, &mut desktop, "two");
    minimize(&mut core, &mut desktop, editor);
    let away = published(&core, &desktop);
    assert_eq!(listed_on(&away, "main"), [editor]);
    assert!(!entry(&away, editor).expect("listed").is_minimized);
}

/// What was reported. A floating window let go of on the other output is
/// that output's workspace's from then on: published for it and for no
/// other, and told as moved, so that both panels ask for the state.
#[test]
fn a_floating_window_let_go_of_on_the_other_output_is_listed_there_and_told() {
    let (mut core, mut desktop) = setup(&["main"]);
    second_display(&mut core, &mut desktop, &["side"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = open(&mut core, &mut desktop, "viewer");
    focus(&mut core, &mut desktop, viewer);
    command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
    assert!(core.monitor_manager.is_window_floating(viewer));
    let before = published(&core, &desktop);
    assert_eq!(listed(&before, viewer), Some((OUTPUT, "main".to_string())));

    core.handle_window_event(&mut desktop, WindowEvent::DragStarted(viewer))
        .expect("taken");
    desktop.windows.get_mut(&viewer).unwrap().rect = Rect::new(2400, 300, 800, 600);
    core.handle_window_event(&mut desktop, WindowEvent::Moved(viewer, 2400, 300))
        .expect("moved");
    // While it is held it is its workspace's still.
    assert_eq!(
        listed(&published(&core, &desktop), viewer),
        Some((OUTPUT, "main".to_string()))
    );
    desktop.output = BESIDE;
    core.handle_window_event(&mut desktop, WindowEvent::DragEnded(viewer))
        .expect("let go");
    core.flush_pending_layouts(&mut desktop).expect("layout");

    let after = published(&core, &desktop);
    assert_eq!(listed(&after, viewer), Some((BESIDE, "side".to_string())));
    assert_eq!(listed(&after, editor), Some((OUTPUT, "main".to_string())));
    assert_eq!(told_moved(&before, &after), [viewer.0.to_string()]);
    // And back, by a drag whose end was never told (the window manager
    // finds it let go of a moment later).
    core.handle_window_event(&mut desktop, WindowEvent::DragStarted(viewer))
        .expect("taken");
    desktop.windows.get_mut(&viewer).unwrap().rect = Rect::new(300, 300, 800, 600);
    desktop.output = OUTPUT;
    if let Some(drag) = core.executor.window_state.get_mut(viewer).drag.as_mut() {
        drag.last_move_time = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(2))
            .expect("the machine is up for longer than that");
    }
    core.executor
        .process_pending_drags(
            &mut desktop,
            &mut core.monitor_manager,
            &mut core.snap_zone_manager,
        )
        .expect("the drag is taken as ended");
    let back = published(&core, &desktop);
    assert_eq!(listed(&back, viewer), Some((OUTPUT, "main".to_string())));
    assert_eq!(told_moved(&after, &back), [viewer.0.to_string()]);
}

/// A tiled window dragged to the other output, and a window sent there
/// by a command: each is published for the output it went to alone.
#[test]
fn a_window_that_goes_to_the_other_output_leaves_the_first_ones_list() {
    use gevrek_model::command::{MonitorTarget, WindowSelector};
    let (mut core, mut desktop) = setup(&["main"]);
    second_display(&mut core, &mut desktop, &["side"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let terminal = open(&mut core, &mut desktop, "terminal");
    let before = published(&core, &desktop);

    core.handle_window_event(&mut desktop, WindowEvent::DragStarted(editor))
        .expect("taken");
    desktop.output = BESIDE;
    core.handle_window_event(&mut desktop, WindowEvent::DragEnded(editor))
        .expect("let go");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    let dragged = published(&core, &desktop);
    assert_eq!(listed(&dragged, editor), Some((BESIDE, "side".to_string())));
    assert_eq!(
        listed(&dragged, terminal),
        Some((OUTPUT, "main".to_string()))
    );
    assert_eq!(told_moved(&before, &dragged), [editor.0.to_string()]);

    desktop.output = OUTPUT;
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveWindowToMonitor {
            window: WindowSelector::Handle(terminal.0 as i64),
            target: MonitorTarget::Name("Second".to_string()),
        },
    );
    let sent = published(&core, &desktop);
    assert_eq!(listed(&sent, terminal), Some((BESIDE, "side".to_string())));
    assert_eq!(told_moved(&dragged, &sent), [terminal.0.to_string()]);
    assert!(sent[0]
        .workspaces
        .iter()
        .all(|workspace| workspace.detailed_windows.is_empty()));
}

/// A display goes: its windows are the remaining one's, each listed
/// once. It comes back: every window is listed once still, for an output
/// that is there.
#[test]
fn the_windows_of_a_display_that_goes_are_listed_once_for_one_that_is_there() {
    let (mut core, mut desktop) = setup(&["main"]);
    let screen = second_display(&mut core, &mut desktop, &["side"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = desktop.open_at("viewer", Rect::new(2200, 300, 800, 600));
    core.handle_window_event(&mut desktop, WindowEvent::Created(viewer))
        .expect("created");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    let both = published(&core, &desktop);
    assert_eq!(listed(&both, editor), Some((OUTPUT, "main".to_string())));
    assert_eq!(listed(&both, viewer), Some((BESIDE, "side".to_string())));

    let second = core
        .monitor_manager
        .get_monitor(BESIDE)
        .cloned()
        .expect("the second display");
    let change = |core: &mut WmCore, desktop: &mut FakeDesktop, there: bool| {
        let mut monitors: Vec<_> = core
            .monitor_manager
            .get_all_monitors()
            .into_iter()
            .filter(|monitor| monitor.handle != BESIDE)
            .collect();
        desktop.more_outputs.clear();
        if there {
            monitors.push(second.clone());
            desktop.more_outputs.push((BESIDE, screen));
        }
        let changes = core.monitor_manager.reconcile_with(monitors);
        core.executor
            .apply_monitor_changes(desktop, &core.monitor_manager, &changes);
        core.flush_pending_layouts(desktop).expect("layout");
    };

    change(&mut core, &mut desktop, false);
    let one = published(&core, &desktop);
    assert_eq!(one.len(), 1, "one display is published");
    for window in [editor, viewer] {
        assert_eq!(
            listed(&one, window).map(|(output, _)| output),
            Some(OUTPUT),
            "{window:?} is the remaining display's"
        );
    }

    change(&mut core, &mut desktop, true);
    let again = published(&core, &desktop);
    let there: Vec<MonitorHandle> = again.iter().map(|monitor| monitor.handle).collect();
    for window in [editor, viewer] {
        let (output, _) = listed(&again, window).expect("listed");
        assert!(
            there.contains(&output),
            "{window:?} is listed for {output:?}"
        );
    }
}

// ---- windows that belong to another window --------------------------------

/// A dialog of `parent` opens at `rect`, taking the keyboard as a new
/// window does, and the core is told.
fn open_dialog(
    core: &mut WmCore,
    desktop: &mut FakeDesktop,
    title: &str,
    parent: WindowHandle,
    rect: Rect,
) -> WindowHandle {
    let id = desktop.open_dialog(title, parent, rect);
    core.handle_window_event(desktop, WindowEvent::Created(id))
        .expect("created");
    core.flush_pending_layouts(desktop).expect("layout");
    id
}

/// The graph gevrek comes with, as far as new windows go: each is
/// arranged as what it is.
fn arranging_graph() -> NodeGraph {
    use gevrek_model::graph::{NodeType, WindowMode};
    let mut graph = NodeGraph::new();
    let created = graph.add_node(NodeType::OnWindowCreated, [0.0, 0.0]);
    let arrange = graph.add_node(
        NodeType::SetWindowArrangement {
            mode: WindowMode::Auto,
        },
        [0.0, 0.0],
    );
    graph
        .add_connection_by_name(created, "Event", arrange, "Trigger")
        .expect("connected");
    graph
        .add_connection_by_name(created, "Window", arrange, "Window")
        .expect("connected");
    graph
}

/// What was reported: a dialog a program opens while the person works on
/// another workspace came up there, over what they were doing, with the
/// keyboard. It opens on its window's workspace, out of sight with it,
/// and the keyboard stays where it was; it is there, with its window, when
/// that workspace is shown. Whether the window is taken by gevrek itself
/// or by the graph.
#[test]
fn a_dialog_of_a_window_that_is_put_away_opens_with_it() {
    for by_graph in [false, true] {
        let (mut core, mut desktop) = setup(&["main", "two"]);
        if by_graph {
            core.executor.set_graph(arranging_graph());
        }
        let editor = open(&mut core, &mut desktop, "editor");
        switch_to(&mut core, &mut desktop, "two");
        let notes = open(&mut core, &mut desktop, "notes");
        focus(&mut core, &mut desktop, notes);
        let before = published(&core, &desktop);

        let place = Rect::new(500, 300, 400, 200);
        let question = open_dialog(&mut core, &mut desktop, "Save changes?", editor, place);

        assert_eq!(
            core.monitor_manager.find_window(question),
            Some((OUTPUT, 0)),
            "on its window's workspace (by the graph: {by_graph})"
        );
        assert!(core.monitor_manager.is_window_floating(question));
        assert!(!desktop.window(question).visible, "out of sight");
        assert!(core.executor.window_state.get(question).hidden_by_wm);
        assert_eq!(desktop.focused, Some(notes), "the keyboard stays");
        assert_eq!(desktop.window(question).rect, place, "where it opened");
        assert_eq!(tiled(&core), [notes], "the workspace in sight is as it was");
        let after = published(&core, &desktop);
        assert_eq!(listed(&after, question), Some((OUTPUT, "main".to_string())));
        assert!(told_moved(&before, &after).is_empty());

        // Its window's workspace is shown: both are there.
        switch_to(&mut core, &mut desktop, "main");
        assert!(desktop.window(editor).visible && desktop.window(question).visible);
        assert!(!desktop.window(notes).visible);
        assert_eq!(desktop.window(question).rect, place);
        assert!(!core.executor.window_state.get(question).hidden_by_wm);
    }
}

/// A dialog of a window on the workspace that is shown opens as it always
/// did: in sight, floating, with the keyboard.
#[test]
fn a_dialog_of_a_window_in_sight_opens_in_sight_with_the_keyboard() {
    for by_graph in [false, true] {
        let (mut core, mut desktop) = setup(&["main", "two"]);
        if by_graph {
            core.executor.set_graph(arranging_graph());
        }
        let editor = open(&mut core, &mut desktop, "editor");
        focus(&mut core, &mut desktop, editor);
        let question = open_dialog(
            &mut core,
            &mut desktop,
            "Save changes?",
            editor,
            Rect::new(500, 300, 400, 200),
        );
        assert_eq!(
            core.monitor_manager.find_window(question),
            Some((OUTPUT, 0))
        );
        assert!(core.monitor_manager.is_window_floating(question));
        assert!(desktop.window(question).visible);
        assert_eq!(desktop.focused, Some(question), "it has the keyboard");
        assert_eq!(tiled(&core), [editor]);
    }
}

/// A rule of the person's that sends the window to a workspace is the
/// last word: the dialog goes where the rule says, not after its window.
#[test]
fn a_rule_that_names_a_workspace_wins_over_the_dialogs_window() {
    use gevrek_model::settings::{RuleEffect, RulePredicate, WindowRuleProfile};
    for dynamic in [false, true] {
        let rules = Arc::new(Mutex::new(RuleEngine::new()));
        rules
            .lock()
            .unwrap()
            .set_declarative_rules(vec![WindowRuleProfile {
                name: "questions".to_string(),
                dynamic,
                predicates: vec![RulePredicate {
                    kind: crate::rules::MatchKind::Title,
                    strategy: crate::rules::MatchStrategy::Contains,
                    value: "Save".to_string(),
                    ..Default::default()
                }],
                effects: vec![RuleEffect::MoveToWorkspace {
                    monitor_index: None,
                    workspace_index: 2,
                }],
                ..Default::default()
            }]);
        let executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
        let mut outputs = Outputs::with_workspace_count(0);
        outputs.add_test_monitor(OUTPUT, "Primary");
        for name in ["main", "two", "three"] {
            outputs
                .create_workspace(OUTPUT, name.to_string(), LayoutType::BSP)
                .expect("workspace created");
        }
        let mut core = WmCore::new(
            executor,
            outputs,
            FocusManager::new(),
            SnapZoneManager::new(Default::default()),
        );
        let mut desktop = FakeDesktop::new(OUTPUT);

        let editor = open(&mut core, &mut desktop, "editor");
        switch_to(&mut core, &mut desktop, "two");
        let question = open_dialog(
            &mut core,
            &mut desktop,
            "Save changes?",
            editor,
            Rect::new(500, 300, 400, 200),
        );
        assert_eq!(
            core.monitor_manager.find_window(question),
            Some((OUTPUT, 2)),
            "where the rule says (a rule looked at again later: {dynamic})"
        );
        assert!(!desktop.window(question).visible);
    }
}

/// Where the window a dialog belongs to is on no workspace (gevrek leaves
/// it alone, it is a scratchpad's, the person minimized it), the dialog
/// opens where the person is, as any window does.
#[test]
fn a_dialog_of_a_window_on_no_workspace_opens_where_the_person_is() {
    use gevrek_model::command::WindowSelector;
    // Left alone.
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::UnmanageWindow {
            selector: WindowSelector::Handle(editor.0 as i64),
        },
    );
    assert!(core.monitor_manager.find_window(editor).is_none());
    switch_to(&mut core, &mut desktop, "two");
    let place = Rect::new(500, 300, 400, 200);
    let question = open_dialog(&mut core, &mut desktop, "Save changes?", editor, place);
    assert_eq!(
        core.monitor_manager.find_window(question),
        Some((OUTPUT, 1))
    );
    assert!(desktop.window(question).visible);
    assert_eq!(desktop.focused, Some(question));

    // A scratchpad's.
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let terminal = desktop.open("terminal", "app.exe");
    core.executor.scratchpads.bind("terminal", terminal, false);
    let question = open_dialog(&mut core, &mut desktop, "Close?", terminal, place);
    assert_eq!(
        core.monitor_manager.find_window(question),
        Some((OUTPUT, 0))
    );
    assert!(desktop.window(question).visible);

    // Minimized by the person, floating on a workspace that is put away.
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let viewer = open(&mut core, &mut desktop, "viewer");
    focus(&mut core, &mut desktop, viewer);
    command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
    desktop.windows.get_mut(&viewer).unwrap().minimized = true;
    core.handle_window_event(&mut desktop, WindowEvent::Minimized(viewer))
        .expect("minimized");
    switch_to(&mut core, &mut desktop, "two");
    assert_eq!(core.monitor_manager.find_window(viewer), Some((OUTPUT, 0)));
    let question = open_dialog(&mut core, &mut desktop, "Close?", viewer, place);
    assert_eq!(
        core.monitor_manager.find_window(question),
        Some((OUTPUT, 1))
    );
    assert!(desktop.window(question).visible);

    // A dialog's own dialog goes after the window above them both.
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let unmanaged = desktop.open_dialog("Find", editor, place);
    switch_to(&mut core, &mut desktop, "two");
    let question = open_dialog(&mut core, &mut desktop, "Not found", unmanaged, place);
    assert_eq!(
        core.monitor_manager.find_window(question),
        Some((OUTPUT, 0))
    );
    assert!(!desktop.window(question).visible);
}

/// The window a dialog belongs to closes while both are put away: the
/// dialog is its workspace's still, and is there when that is shown.
#[test]
fn a_dialog_outlives_its_window_on_the_workspace_it_opened_on() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    switch_to(&mut core, &mut desktop, "two");
    let question = open_dialog(
        &mut core,
        &mut desktop,
        "Save changes?",
        editor,
        Rect::new(500, 300, 400, 200),
    );
    desktop.close(editor);
    core.handle_window_event(&mut desktop, WindowEvent::Destroyed(editor))
        .expect("destroyed");
    core.flush_pending_layouts(&mut desktop).expect("layout");
    assert_eq!(
        core.monitor_manager.find_window(question),
        Some((OUTPUT, 0))
    );
    assert!(!desktop.window(question).visible);
    assert_eq!(
        listed(&published(&core, &desktop), question),
        Some((OUTPUT, "main".to_string()))
    );

    switch_to(&mut core, &mut desktop, "main");
    assert!(desktop.window(question).visible);
    // And a window that opens for it now finds no window to go after.
    switch_to(&mut core, &mut desktop, "two");
    let orphan = open_dialog(
        &mut core,
        &mut desktop,
        "Really?",
        editor,
        Rect::new(500, 300, 400, 200),
    );
    assert_eq!(core.monitor_manager.find_window(orphan), Some((OUTPUT, 1)));
}

/// Under the Floating layout a window that belongs to another, or that
/// stands where its program put it, stays there; only one of which too
/// little would be on the display is brought back. A program's own
/// window still opens in the middle.
#[test]
fn the_floating_layout_leaves_a_window_with_a_place_of_its_own_where_it_is() {
    let (mut core, mut desktop) = setup(&["one"]);
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Floating,
        },
    );
    let middle = |desktop: &FakeDesktop, id: WindowHandle| {
        let rect = desktop.window(id).rect;
        // The fake output is 1920x1080, with 40 kept free at its foot.
        (rect.x + rect.width / 2 - 960).abs() <= 1 && (rect.y + rect.height / 2 - 520).abs() <= 1
    };
    let arrive = |core: &mut WmCore, desktop: &mut FakeDesktop, id: WindowHandle| {
        core.handle_window_event(desktop, WindowEvent::Created(id))
            .expect("created");
        core.flush_pending_layouts(desktop).expect("layout");
    };
    let editor = open(&mut core, &mut desktop, "editor");
    assert!(
        middle(&desktop, editor),
        "a program's window: in the middle"
    );

    // One that belongs to the editor (not a dialog by what the desktop
    // says: it would be tiled elsewhere, and comes as any window does).
    let place = Rect::new(1200, 150, 420, 300);
    let palette = desktop.open_at("palette", place);
    desktop.windows.get_mut(&palette).unwrap().parent = Some(editor);
    arrive(&mut core, &mut desktop, palette);
    assert!(core.monitor_manager.is_window_floating(palette));
    assert_eq!(desktop.window(palette).rect, place, "it keeps its place");

    // One that stands where its program put it.
    let own = Rect::new(40, 700, 500, 260);
    let player = desktop.open_at("player", own);
    desktop.windows.get_mut(&player).unwrap().placed_itself = true;
    arrive(&mut core, &mut desktop, player);
    assert_eq!(desktop.window(player).rect, own, "it keeps its place");

    // One of which only a sliver is on the display: brought back.
    let lost = desktop.open_at("tools", Rect::new(1900, 150, 420, 300));
    desktop.windows.get_mut(&lost).unwrap().parent = Some(editor);
    arrive(&mut core, &mut desktop, lost);
    assert!(middle(&desktop, lost), "{:?}", desktop.window(lost).rect);
    assert_eq!(
        (
            desktop.window(lost).rect.width,
            desktop.window(lost).rect.height
        ),
        (420, 300)
    );

    // And a window of its own, after all that: in the middle still.
    let viewer = open(&mut core, &mut desktop, "viewer");
    assert!(middle(&desktop, viewer));
    assert_eq!(desktop.window(palette).rect, place, "nobody else was moved");
}

/// A window that comes to a workspace under the Floating layout has a
/// border, as one that was tiled before the layout was set has: it was
/// floated before a border was ever drawn around it, and left without.
#[test]
fn a_window_that_comes_under_the_floating_layout_has_a_border() {
    let (mut core, mut desktop) = setup(&["one"]);
    core.monitor_manager
        .get_focused_workspace_mut(OUTPUT)
        .expect("workspace")
        .border_enabled = true;
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Floating,
        },
    );
    let editor = open(&mut core, &mut desktop, "editor");
    assert!(core.monitor_manager.is_window_floating(editor));
    assert!(
        desktop
            .decorated
            .iter()
            .any(|(window, _)| *window == editor),
        "no border was drawn around it: {:?}",
        desktop.decorated
    );
}

// ---- floating windows that are moved to another display --------------------

/// Two displays, an editor that is tiled and a viewer that floats, both on
/// the first.
fn a_viewer_that_floats() -> (WmCore, FakeDesktop, WindowHandle, WindowHandle) {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    second_display(&mut core, &mut desktop, &["side", "more"]);
    let editor = open(&mut core, &mut desktop, "editor");
    let viewer = open(&mut core, &mut desktop, "viewer");
    focus(&mut core, &mut desktop, viewer);
    command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
    assert!(core.monitor_manager.is_window_floating(viewer));
    (core, desktop, editor, viewer)
}

/// The program of a window (or the system, for a key) puts it somewhere.
fn moved_by_itself(core: &mut WmCore, desktop: &mut FakeDesktop, window: WindowHandle, to: Rect) {
    desktop.windows.get_mut(&window).unwrap().rect = to;
    core.handle_window_event(desktop, WindowEvent::Moved(window, to.x, to.y))
        .expect("moved");
    core.flush_pending_layouts(desktop).expect("layout");
}

/// The look that is taken every now and then, a moment after the last
/// move: the windows that joined another display.
fn at_rest(core: &mut WmCore, desktop: &mut FakeDesktop) -> Vec<WindowHandle> {
    let later = std::time::Instant::now() + std::time::Duration::from_secs(1);
    core.settle_moved_windows(desktop, later)
}

const ON_FIRST: Rect = Rect {
    x: 300,
    y: 300,
    width: 800,
    height: 600,
};
const ON_SECOND: Rect = Rect {
    x: 2400,
    y: 300,
    width: 800,
    height: 600,
};

/// What was reported. A floating window its program moved to the other
/// display (or the system did, for Win+Shift+arrow) stayed listed for the
/// display it had left. Once it has come to rest it is the other
/// display's, published for it alone and told as moved, once.
#[test]
fn a_floating_window_moved_to_the_other_display_by_its_program_joins_it() {
    let (mut core, mut desktop, editor, viewer) = a_viewer_that_floats();
    let before = published(&core, &desktop);
    let main = Some((OUTPUT, "main".to_string()));
    let side = Some((BESIDE, "side".to_string()));

    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    // Looked at while it may be on its way still: not yet.
    let now = std::time::Instant::now();
    assert!(core.settle_moved_windows(&mut desktop, now).is_empty());
    assert_eq!(listed(&published(&core, &desktop), viewer), main);

    assert_eq!(at_rest(&mut core, &mut desktop), [viewer]);
    let after = published(&core, &desktop);
    assert_eq!(listed(&after, viewer), side);
    assert_eq!(listed(&after, editor), main);
    assert_eq!(told_moved(&before, &after), [viewer.0.to_string()]);
    assert_eq!(
        desktop.window(viewer).rect,
        ON_SECOND,
        "it stays where it is"
    );
    assert!(desktop.window(viewer).visible);
    // (It has the keyboard: what is asked for next is asked of that
    // display.)
    assert_eq!(core.executor.focused_monitor(), Some(BESIDE));

    // Looked at again: nothing more happens, and nothing more is told.
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert!(told_moved(&after, &published(&core, &desktop)).is_empty());
    // It is hidden and shown with the workspace it joined.
    switch_to(&mut core, &mut desktop, "more");
    assert!(!desktop.window(viewer).visible);
    assert!(desktop.window(editor).visible);
    switch_to(&mut core, &mut desktop, "side");
    assert!(desktop.window(viewer).visible);

    // And back the same way.
    let there = published(&core, &desktop);
    moved_by_itself(&mut core, &mut desktop, viewer, ON_FIRST);
    assert_eq!(at_rest(&mut core, &mut desktop), [viewer]);
    let back = published(&core, &desktop);
    assert_eq!(listed(&back, viewer), main);
    assert_eq!(told_moved(&there, &back), [viewer.0.to_string()]);
}

/// A window that lies across the edge stays with the display it was on
/// until most of it has left; one that lies half and half stays with
/// whichever display has it, so it is not sent to and fro.
#[test]
fn a_window_across_the_edge_stays_until_most_of_it_has_left() {
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    let home = |core: &WmCore| core.monitor_manager.find_window(viewer).map(|(on, _)| on);
    let at = |x: i32| Rect::new(x, 300, 800, 600);

    // 620 of its 800 on the first display.
    moved_by_itself(&mut core, &mut desktop, viewer, at(1300));
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core), Some(OUTPUT));
    // Half and half.
    moved_by_itself(&mut core, &mut desktop, viewer, at(1520));
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core), Some(OUTPUT));
    // 399 on the first.
    moved_by_itself(&mut core, &mut desktop, viewer, at(1521));
    assert_eq!(at_rest(&mut core, &mut desktop), [viewer]);
    assert_eq!(home(&core), Some(BESIDE));
    // Half and half again: it is the second display's now, and stays.
    for _ in 0..3 {
        moved_by_itself(&mut core, &mut desktop, viewer, at(1520));
        assert!(at_rest(&mut core, &mut desktop).is_empty());
        assert_eq!(home(&core), Some(BESIDE));
    }
    // 401 on the first: back.
    moved_by_itself(&mut core, &mut desktop, viewer, at(1519));
    assert_eq!(at_rest(&mut core, &mut desktop), [viewer]);
    assert_eq!(home(&core), Some(OUTPUT));
}

/// What is not a window that came to rest on another display by itself:
/// one the desktop is moving (a workspace that slides), one that is
/// hidden, one that is held by the pointer, one gevrek put there, a tiled
/// one, one put out of the way, a maximized one, and one that was seen
/// before the displays changed.
#[test]
fn a_floating_window_is_not_taken_to_another_display_for_a_move_that_is_not_its_own() {
    let home = |core: &WmCore, window| core.monitor_manager.find_window(window).map(|(on, _)| on);

    // The desktop moves it itself: its workspace slides in.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    desktop.animating.insert(viewer);
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    // (And where the slide began after the move was seen: it waits, and
    // is judged by where the window is when the slide is over.)
    desktop.animating.clear();
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    desktop.animating.insert(viewer);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    desktop.windows.get_mut(&viewer).unwrap().rect = ON_FIRST;
    desktop.animating.clear();
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));

    // Hidden: its workspace is put away (what a hidden window says of its
    // place is where it was put away, or a place it passes through).
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    switch_to(&mut core, &mut desktop, "two");
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(core.monitor_manager.find_window(viewer), Some((OUTPUT, 0)));
    // Seen in sight, and hidden before it came to rest.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    switch_to(&mut core, &mut desktop, "two");
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(core.monitor_manager.find_window(viewer), Some((OUTPUT, 0)));

    // Held: a drag says where it went when it ends, by where the pointer
    // is. (The pointer is on the first display still.)
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    core.handle_window_event(&mut desktop, WindowEvent::DragStarted(viewer))
        .expect("taken");
    // (At the size it has: a drag that changes the size is another matter.)
    let held = desktop.window(viewer).rect;
    let across = Rect::new(2400, 300, held.width, held.height);
    moved_by_itself(&mut core, &mut desktop, viewer, across);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));
    core.handle_window_event(&mut desktop, WindowEvent::DragEnded(viewer))
        .expect("let go");
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));
    // Its place is told once more after the letting go, as some systems
    // do: the drag has said where it is (most of it lies on the second
    // display, and the pointer let go of it on the first).
    core.handle_window_event(&mut desktop, WindowEvent::Moved(viewer, across.x, across.y))
        .expect("moved");
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));
    // Moved by its program after that, it is judged as any other.
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    assert_eq!(at_rest(&mut core, &mut desktop), [viewer]);
    assert_eq!(home(&core, viewer), Some(BESIDE));
    // The button is down, with no drag told of: the same.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    desktop.button_down = true;
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    desktop.button_down = false;
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    // Seen with no button down, and taken hold of before it came to rest.
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    desktop.button_down = true;
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    desktop.button_down = false;
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));

    // gevrek put it there itself.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    core.executor.expect_own_move_for_tests(viewer);
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));

    // A tiled window is where its tile is.
    let (mut core, mut desktop, editor, _viewer) = a_viewer_that_floats();
    moved_by_itself(&mut core, &mut desktop, editor, ON_SECOND);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, editor), Some(OUTPUT));

    // Put out of the way, to the side of its display the other one is at.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    moved_by_itself(
        &mut core,
        &mut desktop,
        viewer,
        Rect::new(1000, 300, 800, 600),
    );
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::ToggleFloatingAside,
    );
    let aside = desktop.window(viewer).rect;
    assert!(aside.x > 1520, "most of it is beyond the edge: {aside:?}");
    core.handle_window_event(&mut desktop, WindowEvent::Moved(viewer, aside.x, aside.y))
        .expect("moved");
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));

    // Maximized on the other display by its program: left to it.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    desktop.windows.get_mut(&viewer).unwrap().maximized = true;
    moved_by_itself(
        &mut core,
        &mut desktop,
        viewer,
        Rect::new(1920, 0, 2560, 1440),
    );
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));

    // The displays change before it came to rest: nothing is carried over.
    let (mut core, mut desktop, _editor, viewer) = a_viewer_that_floats();
    moved_by_itself(&mut core, &mut desktop, viewer, ON_SECOND);
    let same = core.monitor_manager.get_all_monitors();
    let changes = core.monitor_manager.reconcile_with(same);
    core.executor
        .apply_monitor_changes(&mut desktop, &core.monitor_manager, &changes);
    assert!(at_rest(&mut core, &mut desktop).is_empty());
    assert_eq!(home(&core, viewer), Some(OUTPUT));
}

// ---- showing the desktop ----------------------------------------------------

/// The events of the desktop that were told: each kind with its fields.
type Heard = Arc<Mutex<Vec<(String, std::collections::BTreeMap<String, String>)>>>;

/// What showing the desktop is tried on: workspaces `main`, `two` and
/// `empty` on the first display and `side` and `more` on the second, with
/// on `main` two tiled windows (`editor` with the keyboard, and `browser`,
/// maximized) and a floating one, `notes`, and on `side` the tiled
/// `viewer`. The events the window manager tells are heard.
struct Desk {
    core: WmCore,
    desktop: FakeDesktop,
    heard: Heard,
    editor: WindowHandle,
    browser: WindowHandle,
    notes: WindowHandle,
    viewer: WindowHandle,
}

impl Desk {
    fn new() -> Self {
        use crate::executor::{NodeExecutionEvent, RuntimeEventKind};
        let heard = Arc::new(Mutex::new(Vec::new()));
        let told = heard.clone();
        let rules = Arc::new(Mutex::new(RuleEngine::new()));
        let executor = Executor::new_with_events(
            NodeGraph::new(),
            Arc::new(Mutex::new(Vec::new())),
            None,
            Some(Arc::new(move |event: NodeExecutionEvent| {
                if let NodeExecutionEvent::Runtime(event) = event {
                    if matches!(
                        event.kind,
                        RuntimeEventKind::DesktopShown | RuntimeEventKind::DesktopHidden
                    ) {
                        told.lock()
                            .unwrap()
                            .push((event.kind.as_str().to_string(), event.fields));
                    }
                }
            })),
            rules,
        );
        let mut outputs = Outputs::with_workspace_count(0);
        outputs.add_test_monitor(OUTPUT, "Primary");
        for name in ["main", "two", "empty"] {
            outputs
                .create_workspace(OUTPUT, name.to_string(), LayoutType::BSP)
                .expect("workspace created");
        }
        let mut core = WmCore::new(
            executor,
            outputs,
            FocusManager::new(),
            SnapZoneManager::new(Default::default()),
        );
        let mut desktop = FakeDesktop::new(OUTPUT);
        second_display(&mut core, &mut desktop, &["side", "more"]);
        let editor = open(&mut core, &mut desktop, "editor");
        let browser = open(&mut core, &mut desktop, "browser");
        let notes = desktop.open_at("notes", Rect::new(300, 200, 500, 400));
        desktop.windows.get_mut(&notes).unwrap().floats = true;
        core.handle_window_event(&mut desktop, WindowEvent::Created(notes))
            .expect("created");
        core.flush_pending_layouts(&mut desktop).expect("layout");
        let viewer = desktop.open_at("viewer", Rect::new(2400, 300, 800, 600));
        core.handle_window_event(&mut desktop, WindowEvent::Created(viewer))
            .expect("created");
        core.flush_pending_layouts(&mut desktop).expect("layout");
        assert!(core.monitor_manager.is_window_floating(notes));
        assert_eq!(
            core.monitor_manager
                .find_window(viewer)
                .map(|(output, _)| output),
            Some(BESIDE)
        );
        focus(&mut core, &mut desktop, browser);
        command(&mut core, &mut desktop, DispatchCommand::ToggleMaximized);
        assert!(desktop.window(browser).maximized);
        focus(&mut core, &mut desktop, editor);
        Self {
            core,
            desktop,
            heard,
            editor,
            browser,
            notes,
            viewer,
        }
    }

    fn all(&self) -> [WindowHandle; 4] {
        [self.editor, self.browser, self.notes, self.viewer]
    }

    fn in_sight(&self, window: WindowHandle) -> bool {
        let window = self.desktop.window(window);
        window.visible && !window.hidden_by_wm && !window.minimized
    }

    fn places(&self) -> Vec<Rect> {
        self.all()
            .map(|window| self.desktop.window(window).rect)
            .to_vec()
    }

    fn show(&mut self, on: Option<bool>) {
        command(
            &mut self.core,
            &mut self.desktop,
            DispatchCommand::ShowDesktop { on },
        );
    }

    fn heard(&self) -> Vec<(String, Option<String>)> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .map(|(kind, fields)| (kind.clone(), fields.get("reason").cloned()))
            .collect()
    }

    /// Nothing is marked as away for the desktop any more.
    fn nothing_away(&self) -> bool {
        self.core
            .executor
            .window_state
            .ids_where(|state| state.away_for_desktop)
            .is_empty()
    }
}

/// Both displays' windows go out of sight, as on a change of workspace
/// (not minimized), and the desktop has the keyboard; again, each is back
/// where and as it was (the maximized one maximized, the floating one
/// floating), and the window that had the keyboard has it again. The
/// display with the keyboard stays the one it was all the while.
#[test]
fn showing_the_desktop_puts_every_displays_windows_away_and_back_as_they_were() {
    let mut desk = Desk::new();
    let places = desk.places();
    assert_eq!(desk.core.executor.focused_monitor(), Some(OUTPUT));

    desk.show(None);
    assert!(desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(!desk.in_sight(window), "{window:?} is out of sight");
        assert!(
            desk.desktop.window(window).hidden_by_wm,
            "{window:?} is hidden, as on a switch"
        );
        assert!(
            !desk.desktop.window(window).minimized,
            "{window:?} is not minimized"
        );
    }
    assert_eq!(desk.desktop.focused, None, "the desktop has the keyboard");
    assert_eq!(desk.core.executor.focused_monitor(), Some(OUTPUT));
    // (Nothing moved them meanwhile, nor did a layout pass.)
    command(&mut desk.core, &mut desk.desktop, DispatchCommand::Retile);
    for window in desk.all() {
        assert!(
            !desk.in_sight(window),
            "{window:?} stays away through a layout pass"
        );
    }

    desk.show(None);
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?} is back");
    }
    assert_eq!(desk.places(), places, "every window is where it was");
    assert!(desk.desktop.window(desk.browser).maximized);
    assert!(desk.core.monitor_manager.is_window_floating(desk.notes));
    assert_eq!(desk.desktop.focused, Some(desk.editor));
    assert_eq!(desk.core.executor.focused_monitor(), Some(OUTPUT));
    assert!(desk.nothing_away());
    assert_eq!(
        desk.heard(),
        [
            ("desktop.shown".to_string(), None),
            ("desktop.hidden".to_string(), Some("command".to_string())),
        ]
    );
    assert_eq!(
        desk.heard.lock().unwrap()[0]
            .1
            .get("windows")
            .map(String::as_str),
        Some("4")
    );

    // With the keyboard on the other display, it is that display's again.
    focus(&mut desk.core, &mut desk.desktop, desk.viewer);
    desk.show(None);
    assert_eq!(desk.core.executor.focused_monitor(), Some(BESIDE));
    desk.show(None);
    assert_eq!(desk.desktop.focused, Some(desk.viewer));
    assert_eq!(desk.core.executor.focused_monitor(), Some(BESIDE));
}

/// `on` and `off` do only that: asked for what is the case already,
/// nothing happens (and nothing is told).
#[test]
fn showing_the_desktop_on_and_off_twice_does_it_once() {
    let mut desk = Desk::new();
    desk.show(Some(true));
    desk.show(Some(true));
    assert!(desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(!desk.in_sight(window));
    }
    desk.show(Some(false));
    desk.show(Some(false));
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window));
    }
    assert_eq!(desk.desktop.focused, Some(desk.editor));
    assert_eq!(desk.heard().len(), 2);

    // And the toggle twice over: away, back, away, back.
    for shown in [true, false, true, false] {
        desk.show(None);
        assert_eq!(desk.core.executor.desktop_shown(), shown);
        assert!(desk
            .all()
            .iter()
            .all(|window| desk.in_sight(*window) != shown));
    }
    assert_eq!(desk.desktop.focused, Some(desk.editor));
}

/// Another workspace shown on one display ends it: the other display's
/// windows come back, those of the workspace that was left stay away with
/// it, and the change goes on as always. Back on that workspace, its
/// windows are where they were.
#[test]
fn another_workspace_ends_the_shown_desktop() {
    let mut desk = Desk::new();
    let places = desk.places();
    desk.show(None);
    switch_to(&mut desk.core, &mut desk.desktop, "two");
    assert!(!desk.core.executor.desktop_shown());
    assert!(
        desk.in_sight(desk.viewer),
        "the other display's window is back"
    );
    for window in [desk.editor, desk.browser, desk.notes] {
        assert!(
            !desk.in_sight(window),
            "{window:?} is away with its workspace"
        );
    }
    assert!(desk.nothing_away());
    assert_eq!(
        desk.heard().last().cloned(),
        Some(("desktop.hidden".to_string(), Some("workspace".to_string())))
    );
    switch_to(&mut desk.core, &mut desk.desktop, "main");
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert_eq!(desk.places(), places);

    // Asked for the workspace it shows, a display shows it again.
    desk.show(None);
    switch_to(&mut desk.core, &mut desk.desktop, "main");
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert_eq!(desk.desktop.focused, Some(desk.editor));
}

/// A window given the keyboard (a press on its button in the panel,
/// Alt+Tab) ends it: every window is back, and that window keeps the
/// keyboard. So does a command that focuses a window.
#[test]
fn a_window_given_the_keyboard_ends_the_shown_desktop() {
    let mut desk = Desk::new();
    desk.show(None);
    focus(&mut desk.core, &mut desk.desktop, desk.browser);
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert_eq!(
        desk.desktop.focused,
        Some(desk.browser),
        "not the one that had it before"
    );
    assert_eq!(
        desk.heard().last().cloned(),
        Some(("desktop.hidden".to_string(), Some("focus".to_string())))
    );

    // A key that focuses a window.
    desk.show(None);
    command(
        &mut desk.core,
        &mut desk.desktop,
        DispatchCommand::FocusWindow {
            selector: gevrek_model::command::WindowSelector::Handle(desk.viewer.0 as _),
        },
    );
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert_eq!(desk.desktop.focused, Some(desk.viewer));

    // A window of no workspace that takes the keyboard (a launcher) does
    // not end it.
    desk.show(None);
    let launcher = desk.desktop.open("launcher", "launcher.exe");
    desk.core
        .executor
        .window_state
        .get_mut(launcher)
        .session_unmanaged = true;
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::Focused(launcher))
        .expect("focused");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert!(desk.core.executor.desktop_shown());
    assert!(desk.all().iter().all(|window| !desk.in_sight(*window)));
}

/// A new window on a shown workspace ends it, and the others come back
/// with it (it is laid out among them); it has the keyboard, as a new
/// window does.
#[test]
fn a_new_window_ends_the_shown_desktop() {
    let mut desk = Desk::new();
    desk.show(None);
    let mail = open(&mut desk.core, &mut desk.desktop, "mail");
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all().into_iter().chain([mail]) {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert_eq!(desk.desktop.focused, Some(mail));
    assert!(desk.nothing_away());
    assert_eq!(
        desk.heard().last().cloned(),
        Some(("desktop.hidden".to_string(), Some("window".to_string())))
    );
    let tiles: Vec<Rect> = tiled(&desk.core)
        .into_iter()
        .map(|window| desk.desktop.window(window).rect)
        .collect();
    assert_eq!(tiles.len(), 3, "{tiles:?}");
}

/// A window closed while the desktop is shown is forgotten: the others
/// come back, laid out without it, and the keyboard, which it had, goes
/// to what its workspace has to give it.
#[test]
fn a_window_closed_while_the_desktop_is_shown_is_forgotten() {
    let mut desk = Desk::new();
    let shared = desk.desktop.window(desk.browser).rect;
    desk.show(None);
    desk.desktop.close(desk.editor);
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::Destroyed(desk.editor))
        .expect("destroyed");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert!(
        desk.core.executor.desktop_shown(),
        "a window closing does not end it"
    );
    assert!(!desk.in_sight(desk.browser));

    desk.show(None);
    for window in [desk.browser, desk.notes, desk.viewer] {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert!(
        desk.desktop.window(desk.browser).rect.width > shared.width,
        "the browser has the room the editor left"
    );
    assert_eq!(desk.desktop.focused, Some(desk.browser));
    assert!(desk.nothing_away());
}

/// A window sent to another workspace while the desktop is shown stays
/// away with that workspace when the desktop ends; one sent to a shown
/// workspace (of the other display) stays away until it ends, and comes
/// back there.
#[test]
fn a_window_moved_while_the_desktop_is_shown_comes_back_where_it_went() {
    use gevrek_model::command::{MonitorTarget, WindowSelector};
    let mut desk = Desk::new();
    desk.show(None);
    command(
        &mut desk.core,
        &mut desk.desktop,
        DispatchCommand::MoveWindowToWorkspace {
            window: WindowSelector::Handle(desk.browser.0 as _),
            workspace: WorkspaceSelector::Name("two".to_string()),
        },
    );
    command(
        &mut desk.core,
        &mut desk.desktop,
        DispatchCommand::MoveWindowToMonitor {
            window: WindowSelector::Handle(desk.notes.0 as _),
            target: MonitorTarget::Name("Second".to_string()),
        },
    );
    assert!(desk.core.executor.desktop_shown());
    assert_eq!(
        desk.core
            .monitor_manager
            .find_window(desk.notes)
            .map(|(output, _)| output),
        Some(BESIDE)
    );
    for window in desk.all() {
        assert!(
            !desk.in_sight(window),
            "{window:?} is out of sight while the desktop is shown"
        );
    }

    desk.show(None);
    assert!(desk.in_sight(desk.editor));
    assert!(desk.in_sight(desk.viewer));
    assert!(desk.in_sight(desk.notes), "back on the display it went to");
    assert!(
        !desk.in_sight(desk.browser),
        "away with the workspace it went to"
    );
    assert!(desk.nothing_away());
    switch_to(&mut desk.core, &mut desk.desktop, "two");
    assert!(desk.in_sight(desk.browser));
}

/// Tabs: only the tab in front comes back, as on a change of workspace;
/// the one behind it stays behind. A tab chosen while the desktop is
/// shown (by a command that does not end it) stays away too.
#[test]
fn only_the_tab_in_front_comes_back() {
    let mut desk = Desk::new();
    command(&mut desk.core, &mut desk.desktop, DispatchCommand::StackAll);
    assert!(desk.in_sight(desk.editor));
    assert!(
        !desk.in_sight(desk.browser),
        "the browser is behind the editor"
    );
    desk.show(None);
    command(
        &mut desk.core,
        &mut desk.desktop,
        DispatchCommand::SetLayoutGap { gap: 12 },
    );
    assert!(desk.all().iter().all(|window| !desk.in_sight(*window)));
    desk.show(None);
    assert!(desk.in_sight(desk.editor));
    assert!(!desk.in_sight(desk.browser), "still behind");
    assert!(desk.in_sight(desk.notes) && desk.in_sight(desk.viewer));
}

/// A window that was minimized is left as it is; brought back from the
/// taskbar while the desktop is shown, it ends it.
#[test]
fn a_minimized_window_is_left_minimized_and_brought_back_ends_the_desktop() {
    let mut desk = Desk::new();
    desk.desktop.windows.get_mut(&desk.notes).unwrap().minimized = true;
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::Minimized(desk.notes))
        .expect("minimized");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    desk.show(None);
    assert!(
        !desk.desktop.window(desk.notes).hidden_by_wm,
        "it is not put away"
    );
    desk.desktop.windows.get_mut(&desk.notes).unwrap().minimized = false;
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::Restored(desk.notes))
        .expect("restored");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert!(desk.nothing_away());
}

/// A display that goes while the desktop is shown ends it: its windows
/// are on the other display's workspaces then, and every window is back.
#[test]
fn a_display_that_goes_ends_the_shown_desktop() {
    let mut desk = Desk::new();
    desk.show(None);
    let monitors: Vec<_> = desk
        .core
        .monitor_manager
        .get_all_monitors()
        .into_iter()
        .filter(|monitor| monitor.handle != BESIDE)
        .collect();
    desk.desktop.more_outputs.clear();
    let changes = desk.core.monitor_manager.reconcile_with(monitors);
    desk.core.executor.apply_monitor_changes(
        &mut desk.desktop,
        &desk.core.monitor_manager,
        &changes,
    );
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert!(!desk.core.executor.desktop_shown());
    for window in desk.all() {
        assert!(desk.in_sight(window), "{window:?}");
    }
    assert!(desk.nothing_away());
    assert_eq!(
        desk.heard().last().cloned(),
        Some(("desktop.hidden".to_string(), Some("displays".to_string())))
    );
}

/// Every hidden window shown (`restore-windows`, or gevrek stopping): the
/// desktop is not shown any more either.
#[test]
fn restoring_every_window_ends_the_shown_desktop() {
    let mut desk = Desk::new();
    desk.show(None);
    desk.core.executor.restore_hidden_windows(&mut desk.desktop);
    assert!(!desk.core.executor.desktop_shown());
    assert!(desk.all().iter().all(|window| desk.in_sight(*window)));
    assert!(desk.nothing_away());
}

/// The rule of a workspace with no window holds with the desktop shown:
/// the display that changes to one keeps the keyboard, and it is not the
/// other display's window (which comes back) that gets it, also where the
/// system gives it to a window shown as the others go.
#[test]
fn a_workspace_with_no_window_keeps_the_keyboard_after_the_desktop() {
    let mut desk = Desk::new();
    desk.desktop.hides_later = true;
    desk.show(None);
    desk.desktop.settle();
    assert_eq!(desk.desktop.focused, None);
    switch_to(&mut desk.core, &mut desk.desktop, "empty");
    desk.desktop.settle();
    assert!(
        desk.in_sight(desk.viewer),
        "the other display's window is back"
    );
    assert_eq!(desk.desktop.focused, None, "the keyboard is nobody's");
    assert_eq!(desk.core.executor.focused_monitor(), Some(OUTPUT));
    switch_to(&mut desk.core, &mut desk.desktop, "main");
    desk.desktop.settle();
    assert_eq!(desk.desktop.focused, Some(desk.editor));
    assert_eq!(desk.core.executor.focused_monitor(), Some(OUTPUT));
}

/// A command for the window with the keyboard, given while the desktop is
/// shown, finds none; one for the workspace (its window made to float, or
/// tiled again) leaves the desktop shown and the windows away.
#[test]
fn a_command_for_the_window_with_the_keyboard_finds_none_while_the_desktop_is_shown() {
    let mut desk = Desk::new();
    desk.show(None);
    assert!(desk
        .core
        .dispatch(
            &mut desk.desktop,
            DispatchCommand::SetPinned { enabled: true }
        )
        .is_err());
    for _ in 0..2 {
        let _ = desk
            .core
            .dispatch(&mut desk.desktop, DispatchCommand::ToggleFloating);
        desk.core
            .flush_pending_layouts(&mut desk.desktop)
            .expect("layout");
    }
    assert!(desk.core.executor.desktop_shown());
    assert!(desk.all().iter().all(|window| !desk.in_sight(*window)));
}

/// Minimized windows and the shown desktop agree: one minimized before it
/// stays minimized after it ends, and one minimized while the desktop is
/// shown is not brought into sight when it ends (it is minimized still,
/// and only its button is back).
#[test]
fn minimized_windows_stay_minimized_through_the_shown_desktop() {
    let mut desk = Desk::new();
    let minimize = |desk: &mut Desk, window: WindowHandle| {
        desk.desktop.windows.get_mut(&window).unwrap().minimized = true;
        desk.core
            .handle_window_event(&mut desk.desktop, WindowEvent::Minimized(window))
            .expect("minimized");
        desk.core
            .flush_pending_layouts(&mut desk.desktop)
            .expect("layout");
    };
    let (notes, browser) = (desk.notes, desk.browser);
    minimize(&mut desk, notes);
    desk.show(None);
    minimize(&mut desk, browser);
    assert!(
        desk.core.executor.desktop_shown(),
        "a window minimized does not end it"
    );
    desk.show(None);
    assert!(!desk.core.executor.desktop_shown());
    for window in [desk.notes, desk.browser] {
        assert!(
            desk.desktop.window(window).minimized,
            "{window:?} is minimized still"
        );
        assert!(!desk.in_sight(window), "{window:?} is not in sight");
    }
    assert!(desk.in_sight(desk.editor) && desk.in_sight(desk.viewer));
    assert!(desk.nothing_away());
}
/// A floating window put away for the desktop on the second display stays
/// out of sight with its workspace when another workspace is shown there
/// (here by a graph, which does not end the desktop first); dragged onto
/// the first display afterwards, it is on the workspace shown there, and
/// in sight. It used to join that workspace out of sight, for good (seed
/// 0xcdf1a3ae49d44796 of the long runs).
#[test]
fn a_window_away_with_its_workspace_and_dragged_onto_a_shown_one_is_in_sight() {
    let mut desk = Desk::new();
    let palette = desk
        .desktop
        .open_at("palette", Rect::new(2600, 400, 400, 300));
    desk.desktop.windows.get_mut(&palette).unwrap().floats = true;
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::Created(palette))
        .expect("created");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert!(desk.core.monitor_manager.is_window_floating(palette));
    desk.show(None);
    // (`more`, the second display's other workspace.)
    desk.core
        .executor
        .show_workspace(&mut desk.desktop, &mut desk.core.monitor_manager, BESIDE, 1)
        .expect("shown");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert!(!desk.core.executor.desktop_shown());
    assert!(!desk.in_sight(palette), "away with its workspace");
    assert!(
        desk.in_sight(desk.editor),
        "the first display's windows are back"
    );

    let dropped = Rect::new(600, 300, 400, 300);
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::DragStarted(palette))
        .expect("drag");
    desk.desktop.windows.get_mut(&palette).unwrap().rect = dropped;
    desk.core
        .handle_window_event(
            &mut desk.desktop,
            WindowEvent::Moved(palette, dropped.x, dropped.y),
        )
        .expect("moved");
    desk.core
        .handle_window_event(&mut desk.desktop, WindowEvent::DragEnded(palette))
        .expect("dropped");
    desk.core
        .flush_pending_layouts(&mut desk.desktop)
        .expect("layout");
    assert_eq!(
        desk.core
            .monitor_manager
            .find_window(palette)
            .map(|(output, _)| output),
        Some(OUTPUT)
    );
    assert!(
        desk.in_sight(palette),
        "on the workspace that is shown, in sight"
    );
    assert!(desk.nothing_away());
}
/// A window moved to a workspace "and follow", where it joins a window
/// there as a tab (the workspace appends new windows to the focused
/// tile), is the tab in front, in sight, with the other behind it. It
/// used to be made the focused tab after the workspace was shown, and
/// stayed out of sight (found by the long runs, seed 0xb2b290c37f612c33).
#[test]
fn a_window_followed_into_tabs_is_the_tab_in_front_and_in_sight() {
    let (mut core, mut desktop) = setup(&["main", "two"]);
    let editor = open(&mut core, &mut desktop, "editor");
    switch_to(&mut core, &mut desktop, "two");
    let terminal = open(&mut core, &mut desktop, "terminal");
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::ToggleWindowContainerBehaviour,
    );
    switch_to(&mut core, &mut desktop, "main");
    focus(&mut core, &mut desktop, editor);
    command(
        &mut core,
        &mut desktop,
        DispatchCommand::MoveToWorkspaceAndFollow {
            selector: WorkspaceSelector::Name("two".to_string()),
        },
    );
    let workspace = core.monitor_manager.get_focused_workspace(OUTPUT).unwrap();
    assert_eq!(workspace.containers.len(), 1, "they share a tile");
    assert_eq!(workspace.focused_window(), Some(editor));
    assert!(desktop.window(editor).visible && !desktop.window(editor).hidden_by_wm);
    assert!(
        !desktop.window(terminal).visible,
        "the terminal is behind it"
    );
    assert_eq!(desktop.focused, Some(editor));
}
/// A window kept on top is a floating one, and one of each workspace's:
/// a tiled window is refused, the window kept on top before is let go
/// when another is, and a window that is tiled again is let go too. The
/// state the panel reads says which it is.
#[test]
fn one_floating_window_of_a_workspace_is_kept_on_top() {
    use gevrek_model::command::WindowSelector;
    let (mut core, mut desktop) = setup(&["main", "other"]);
    let (notes, viewer, editor) = (
        open(&mut core, &mut desktop, "notes"),
        open(&mut core, &mut desktop, "viewer"),
        open(&mut core, &mut desktop, "editor"),
    );
    let handle = |window: WindowHandle| WindowSelector::Handle(window.0 as i64);
    for window in [notes, viewer] {
        command(
            &mut core,
            &mut desktop,
            DispatchCommand::SetWindowFloating {
                selector: handle(window),
                enabled: true,
            },
        );
    }
    let on_top = |desktop: &FakeDesktop| {
        [notes, viewer, editor].map(|window| desktop.window(window).always_on_top)
    };
    let pin = |core: &mut WmCore, desktop: &mut FakeDesktop, window: WindowHandle| {
        core.dispatch(
            desktop,
            DispatchCommand::SetWindowPinned {
                selector: handle(window),
                enabled: true,
            },
        )
    };

    // A tiled window is not kept on top.
    assert!(pin(&mut core, &mut desktop, editor).is_err());
    assert_eq!(on_top(&desktop), [false, false, false]);

    pin(&mut core, &mut desktop, notes).expect("notes on top");
    assert_eq!(on_top(&desktop), [true, false, false]);
    let told = |core: &WmCore, desktop: &FakeDesktop, window: WindowHandle| {
        published(core, desktop)
            .iter()
            .flat_map(|monitor| &monitor.workspaces)
            .flat_map(|workspace| &workspace.detailed_windows)
            .find(|detailed| detailed.handle == window)
            .map(|detailed| detailed.is_always_on_top)
    };
    assert_eq!(told(&core, &desktop, notes), Some(true));

    // Another of the workspace: the one before is let go.
    pin(&mut core, &mut desktop, viewer).expect("viewer on top");
    assert_eq!(on_top(&desktop), [false, true, false]);
    assert_eq!(told(&core, &desktop, notes), Some(false));

    // The toggle lets it go, and keeps it on top again.
    focus(&mut core, &mut desktop, viewer);
    command(&mut core, &mut desktop, DispatchCommand::TogglePinned);
    assert_eq!(on_top(&desktop), [false, false, false]);
    command(&mut core, &mut desktop, DispatchCommand::TogglePinned);
    assert_eq!(on_top(&desktop), [false, true, false]);

    // Tiled again, it is among the others.
    command(&mut core, &mut desktop, DispatchCommand::ToggleFloating);
    assert!(tiled(&core).contains(&viewer));
    assert_eq!(on_top(&desktop), [false, false, false]);
}
