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
use crate::testing::fake_desktop::FakeDesktop;
use crate::zones::ZoneManager;
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
        ZoneManager::new(Default::default()),
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
    let workspaces = core.monitor_manager.get_workspaces(OUTPUT).unwrap().elements();
    let (main, two) = (&workspaces[0], &workspaces[1]);
    assert_eq!(main.floating_windows.elements(), [on_main]);
    assert!(main.containers.is_empty());
    assert!(two.floating_windows.is_empty() && two.containers.is_empty());

    // And tiles again where it is.
    core.dispatch(&mut desktop, DispatchCommand::ToggleFloating)
        .expect("toggled back");
    let workspaces = core.monitor_manager.get_workspaces(OUTPUT).unwrap().elements();
    assert!(workspaces[0].floating_windows.is_empty());
    assert_eq!(workspaces[0].containers.len(), 1);
    assert!(workspaces[1].floating_windows.is_empty() && workspaces[1].containers.is_empty());
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
    assert_eq!(desktop.window(editor).rect, shown.0, "nothing lays it out meanwhile");

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
    apply(&mut core, &[first.clone(), profile("2"), profile("3"), profile("4")], true);
    assert_eq!(names(&core), ["1", "2", "3", "4"]);
    let window = open(&mut core, &mut desktop, "a window");
    assert_eq!(shown(&core).as_deref(), Some("1"));

    // 2 is taken out: empty and not shown, it goes.
    apply(&mut core, &[first.clone(), profile("3"), profile("4")], false);
    assert_eq!(names(&core), ["1", "3", "4"]);

    // Another order: theirs too, and the one shown stays shown.
    apply(&mut core, &[profile("4"), profile("3"), first.clone()], false);
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
        .map(|workspace| (workspace.padding.left, workspace.configured, workspace.persistent));
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
