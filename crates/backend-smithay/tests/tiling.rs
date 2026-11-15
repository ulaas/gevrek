//! What the Wayland WM does with the windows of real clients: the core
//! decides, the compositor configures, the clients answer.
#![cfg(target_os = "linux")]

mod common;

use common::{Harness, SCREEN};
use gevrek_engine::layouts::LayoutType;
use gevrek_engine::{Rect, WindowQueries};
use gevrek_model::command::{DispatchCommand, WorkspaceSelector};
use gevrek_model::config::Config;

fn inside(rect: Rect, screen: Rect) -> bool {
    rect.x >= screen.x
        && rect.y >= screen.y
        && rect.x + rect.width <= screen.x + screen.width
        && rect.y + rect.height <= screen.y + screen.height
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

#[test]
fn the_first_window_is_configured_to_fill_the_output() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (window, id) = harness.open(client, "editor", "org.test.editor");

    let rect = harness.rect(id);
    assert!(harness.shown(id));
    assert!(inside(rect, SCREEN), "{rect:?}");
    assert!(
        rect.width > SCREEN.width * 9 / 10 && rect.height > SCREEN.height * 9 / 10,
        "alone, it has nearly the whole output: {rect:?}"
    );
    // The client was told that size and drew it.
    let committed = harness.clients[client].app.windows[window]
        .committed
        .clone();
    assert_eq!(committed.first(), Some(&(640, 480)), "its own size first");
    assert_eq!(committed.last(), Some(&(rect.width, rect.height)));
    // The WM knows it by what the client said about itself.
    assert_eq!(harness.state.desktop.title(id), "editor");
    assert_eq!(harness.state.desktop.class(id), "org.test.editor");
    assert_eq!(harness.state.desktop.focused, Some(id));
    assert!(harness.clients[client].app.windows[window].activated);
}

#[test]
fn two_windows_are_tiled_side_by_side_without_overlap() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (first_window, first) = harness.open(client, "editor", "org.test.editor");
    let alone = harness.rect(first);
    let (second_window, second) = harness.open(client, "terminal", "org.test.terminal");

    let (a, b) = (harness.rect(first), harness.rect(second));
    assert!(inside(a, SCREEN) && inside(b, SCREEN), "{a:?} {b:?}");
    assert!(!overlap(a, b), "{a:?} {b:?}");
    assert!(a.width < alone.width, "the first window gave up space");
    assert_eq!(a.y, b.y);
    assert_eq!(a.height, b.height);
    assert!((a.width - b.width).abs() <= 2, "{a:?} {b:?}");

    // Each client drew exactly the size the WM gave it.
    let app = &harness.clients[client].app;
    assert_eq!(
        app.windows[first_window].committed.last(),
        Some(&(a.width, a.height))
    );
    assert_eq!(
        app.windows[second_window].committed.last(),
        Some(&(b.width, b.height))
    );
    // The new window has the focus, and only it is active.
    assert_eq!(harness.state.desktop.focused, Some(second));
    assert!(app.windows[second_window].activated && !app.windows[first_window].activated);
}

#[test]
fn a_window_closed_by_its_client_gives_the_space_back() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (_, first) = harness.open(client, "editor", "org.test.editor");
    let alone = harness.rect(first);
    let (second_window, second) = harness.open(client, "terminal", "org.test.terminal");
    assert!(harness.rect(first).width < alone.width);

    harness.clients[client].app.close(second_window);
    harness.pump();

    assert!(!harness.state.desktop.exists(second));
    assert_eq!(harness.rect(first), alone);
    assert_eq!(harness.state.core.monitor_manager.find_window(second), None);
    // The focus went back to the window that is left.
    assert_eq!(harness.state.desktop.focused, Some(first));
}

#[test]
fn the_close_command_asks_the_client_to_close() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (window, id) = harness.open(client, "editor", "org.test.editor");

    harness
        .state
        .dispatch(DispatchCommand::CloseActive)
        .expect("close");
    harness.pump();
    // (the window leaves with its exit transition first)
    harness.finish_transitions();

    assert!(harness.clients[client].app.windows[window].close_requested);
    // The window is still there until its client really closes it.
    assert!(harness.state.desktop.exists(id));
}

#[test]
fn switching_the_workspace_hides_its_windows_and_shows_the_others() {
    let mut harness = Harness::new(Config::new());
    let output = harness.output;
    harness
        .state
        .core
        .monitor_manager
        .create_workspace(output, "two".to_string(), LayoutType::BSP)
        .expect("second workspace");
    let first_name = harness
        .state
        .core
        .monitor_manager
        .get_focused_workspace(output)
        .expect("a workspace is shown")
        .name
        .clone();
    let client = harness.connect();
    let (_, on_first) = harness.open(client, "editor", "org.test.editor");

    let switch = |harness: &mut Harness, name: &str| {
        harness
            .state
            .dispatch(DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name(name.to_string()),
            })
            .expect("switch");
        harness.pump();
    };

    switch(&mut harness, "two");
    assert!(!harness.shown(on_first), "hidden with its workspace");
    assert_eq!(harness.state.desktop.focused, None);

    let (_, on_second) = harness.open(client, "terminal", "org.test.terminal");
    assert!(harness.shown(on_second));
    let rect = harness.rect(on_second);
    assert!(
        rect.width > SCREEN.width * 9 / 10,
        "alone on its workspace: {rect:?}"
    );

    switch(&mut harness, &first_name);
    assert!(harness.shown(on_first) && !harness.shown(on_second));
    assert_eq!(harness.state.desktop.focused, Some(on_first));
}

#[test]
fn windows_of_different_clients_are_tiled_together() {
    let mut harness = Harness::new(Config::new());
    let one = harness.connect();
    let two = harness.connect();
    let (_, first) = harness.open(one, "editor", "org.test.editor");
    let (_, second) = harness.open(two, "terminal", "org.test.terminal");
    let (_, third) = harness.open(one, "browser", "org.test.browser");

    let rects = [
        harness.rect(first),
        harness.rect(second),
        harness.rect(third),
    ];
    for (index, rect) in rects.iter().enumerate() {
        assert!(inside(*rect, SCREEN), "{rect:?}");
        for other in &rects[index + 1..] {
            assert!(!overlap(*rect, *other), "{rect:?} {other:?}");
        }
    }
    let area: i32 = rects.iter().map(|rect| rect.width * rect.height).sum();
    assert!(
        area > SCREEN.width * SCREEN.height * 8 / 10,
        "together they fill the output"
    );
}

#[test]
fn a_client_that_refuses_a_width_is_cut_to_its_tile() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (first_window, first) = harness.open(client, "editor", "org.test.editor");
    harness.clients[client].app.windows[first_window].min_width = 1400;
    let (_, second) = harness.open(client, "terminal", "org.test.terminal");

    // The WM asked for about half and the client drew 1400 wide. The
    // window's place stays its tile, beside the other window, and what the
    // client drew beyond it is known to be cut.
    let (tile, other) = (harness.rect(first), harness.rect(second));
    assert!(tile.width < 1400, "the place is the tile: {tile:?}");
    assert!(!overlap(tile, other), "{tile:?} {other:?}");
    assert!(harness.state.desktop.windows[&first].overflows);
    assert!(!harness.state.desktop.windows[&second].overflows);
}

#[test]
fn a_floating_window_has_a_border_and_glow_of_its_own() {
    let mut config = Config::new();
    config.desktop.appearance.border_width = 5;
    config.desktop.appearance.effects.glow.enabled = true;
    config.desktop.appearance.effects.glow.radius = 30;
    config.desktop.appearance.floating =
        gevrek_model::settings::FloatingAppearance::softer_than_tiled(&config.desktop.appearance);
    let mut harness = Harness::new(config);
    let client = harness.connect();
    let (_, first) = harness.open(client, "editor", "org.test.editor");
    let (_, second) = harness.open(client, "terminal", "org.test.terminal");
    let look = |harness: &Harness, id| {
        let window = &harness.state.desktop.windows[&id];
        (
            window.border.map(|border| border.width),
            window.glow.map(|glow| glow.radius),
        )
    };
    assert_eq!(look(&harness, first), (Some(5), None));
    assert_eq!(
        look(&harness, second),
        (Some(5), None),
        "tiled: the look of every window"
    );

    // The focused window floats: a thinner border, less of a glow.
    harness
        .state
        .run_command(DispatchCommand::ToggleFloating)
        .expect("floating");
    harness.pump();
    assert_eq!(look(&harness, second), (Some(4), Some(20)));
    assert_eq!(
        look(&harness, first),
        (Some(5), None),
        "the tiled one keeps its own"
    );

    // Tiled again: as before.
    harness
        .state
        .run_command(DispatchCommand::ToggleFloating)
        .expect("tiled again");
    harness.pump();
    assert_eq!(look(&harness, second), (Some(5), None));
}

#[test]
fn a_bar_reserves_its_space_and_gives_it_back_when_it_goes() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (_, window) = harness.open(client, "editor", "org.test.editor");
    let full = harness.rect(window);

    // A 30 px bar at the top (layer-shell, exclusive zone 30).
    let bar = {
        let client = &mut harness.clients[client];
        let handle = client.queue.handle();
        client.app.open_bar(&handle, 30)
    };
    harness.pump();
    assert_eq!(
        harness.clients[client].app.bars[bar].configured,
        Some((SCREEN.width as u32, 30)),
        "the bar was given the width of the output"
    );
    let below = harness.rect(window);
    assert_eq!(below.y, full.y + 30, "the window moved below the bar");
    assert_eq!(below.height, full.height - 30);
    assert_eq!((below.x, below.width), (full.x, full.width));
    // The engine was told the smaller work area.
    let monitor = &harness.state.core.monitor_manager.get_all_monitors()[0];
    assert_eq!(
        (monitor.work_y, monitor.work_height),
        (30, SCREEN.height - 30)
    );
    assert_eq!(monitor.height, SCREEN.height);

    harness.clients[client].app.close_bar(bar);
    harness.pump();
    assert_eq!(harness.rect(window), full, "the space came back");
}

#[test]
fn the_protocols_clients_look_for_are_announced() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let globals = &harness.clients[client].app.globals;
    for interface in [
        "wl_compositor",
        "wl_shm",
        "wl_seat",
        "wl_output",
        "wl_data_device_manager",
        "xdg_wm_base",
        "zxdg_decoration_manager_v1",
        "zxdg_output_manager_v1",
        "zwlr_layer_shell_v1",
        "xdg_activation_v1",
        "ext_foreign_toplevel_list_v1",
        "wp_viewporter",
        "wp_fractional_scale_manager_v1",
        "wp_cursor_shape_manager_v1",
        "zwp_idle_inhibit_manager_v1",
        "zwp_primary_selection_device_manager_v1",
    ] {
        assert!(
            globals.iter().any(|name| name == interface),
            "{interface} is missing from {globals:?}"
        );
    }
}

#[test]
fn a_layout_change_is_animated_where_it_is_drawn_and_the_client_resizes_once() {
    use std::time::{Duration, Instant};
    let mut harness = Harness::new(Config::new());
    assert!(
        harness.state.desktop.animations.profile().enabled,
        "the default configuration animates"
    );
    let client = harness.connect();
    let (first_window, first) = harness.open(client, "editor", "org.test.editor");
    // Let the entrance of the first window finish.
    harness
        .state
        .desktop
        .tick_animations(Instant::now() + Duration::from_secs(5));
    let alone = harness.rect(first);
    assert_eq!(harness.state.desktop.drawn_rect(first), Some(alone));
    let commits = harness.clients[client].app.windows[first_window]
        .committed
        .len();

    let (_, second) = harness.open(client, "terminal", "org.test.terminal");

    // The window is at its new place for the WM and for its client at once...
    let target = harness.rect(first);
    assert!(target.width < alone.width);
    // (its client drew again for the new size and for losing the focus, and
    // every time at the final size: no sizes in between were asked for)
    let drawn_since = &harness.clients[client].app.windows[first_window].committed[commits..];
    assert!(!drawn_since.is_empty());
    assert!(
        drawn_since
            .iter()
            .all(|size| *size == (target.width, target.height)),
        "{drawn_since:?}"
    );
    // ...and is drawn from where it was,
    assert_eq!(harness.state.desktop.drawn_rect(first), Some(alone));
    // through sizes in between,
    assert!(harness
        .state
        .desktop
        .tick_animations(Instant::now() + Duration::from_millis(60)));
    let midway = harness.state.desktop.drawn_rect(first).unwrap();
    assert!(
        midway.width < alone.width && midway.width > target.width,
        "{alone:?} -> {midway:?} -> {target:?}"
    );
    // to where it belongs.
    assert!(!harness
        .state
        .desktop
        .tick_animations(Instant::now() + Duration::from_secs(5)));
    assert_eq!(harness.state.desktop.drawn_rect(first), Some(target));
    assert_eq!(
        harness.state.desktop.drawn_rect(second),
        Some(harness.rect(second))
    );
    assert!(harness
        .state
        .desktop
        .windows
        .values()
        .all(|window| window.drawn.is_none()));
}

#[test]
fn stacked_windows_get_a_tab_strip_and_a_tab_click_shows_its_window() {
    let mut config = Config::new();
    config.desktop.appearance.stackbar.enabled = true;
    let bar_height = config.desktop.appearance.stackbar.height;
    let mut harness = Harness::new(config);
    let client = harness.connect();
    let (_, editor) = harness.open(client, "editor", "org.test.editor");
    let (_, terminal) = harness.open(client, "terminal", "org.test.terminal");
    assert!(
        harness.state.desktop.stackbars.is_empty(),
        "side by side: no tabs"
    );
    let side_by_side = harness.rect(editor);

    harness
        .state
        .run_command(DispatchCommand::StackAll)
        .expect("stack");
    harness.pump();

    // One strip with a tab per window, titled by the windows.
    assert_eq!(harness.state.desktop.stackbars.len(), 1);
    let (bar_rect, tabs, titles, focused_tab) = {
        let bar = &harness.state.desktop.stackbars[0];
        (
            bar.rect,
            bar.tabs.clone(),
            bar.titles.clone(),
            bar.focused_tab,
        )
    };
    // (the engine decides the order of the tabs)
    assert_eq!(tabs.len(), 2);
    assert!(tabs.contains(&editor) && tabs.contains(&terminal));
    for (tab, title) in tabs.iter().zip(&titles) {
        assert_eq!(title, if *tab == editor { "editor" } else { "terminal" });
    }
    assert_eq!(bar_rect.height, bar_height);
    // The stack has the whole width; the shown window sits under the strip.
    let shown = tabs[focused_tab];
    let hidden = if shown == editor { terminal } else { editor };
    assert!(harness.shown(shown) && !harness.shown(hidden));
    let rect = harness.rect(shown);
    assert!(rect.width > side_by_side.width);
    assert_eq!((bar_rect.x, bar_rect.width), (rect.x, rect.width));
    assert_eq!(
        rect.y,
        bar_rect.y + bar_rect.height,
        "{bar_rect:?} {rect:?}"
    );

    // A click on the other tab shows the other window.
    let other_tab = 1 - focused_tab;
    let x = bar_rect.x + bar_rect.width * (2 * other_tab as i32 + 1) / 4;
    let hit = harness
        .state
        .desktop
        .stackbar_tab_at(x, bar_rect.y + bar_rect.height / 2);
    assert_eq!(hit.map(|(_, tab)| tab), Some(other_tab));
    let (key, tab) = hit.unwrap();
    harness
        .state
        .core
        .executor
        .focus_stackbar_tab(
            &mut harness.state.desktop,
            &mut harness.state.core.monitor_manager,
            key,
            tab,
        )
        .expect("tab");
    harness.state.settle();
    harness.pump();
    assert!(harness.shown(hidden) && !harness.shown(shown));
    assert_eq!(harness.state.desktop.stackbars[0].focused_tab, other_tab);
    // Below the strip there is no tab.
    assert_eq!(
        harness
            .state
            .desktop
            .stackbar_tab_at(x, bar_rect.y + bar_rect.height + 5),
        None
    );

    // Unstacked, the strip is gone.
    harness
        .state
        .run_command(DispatchCommand::UnstackAll)
        .expect("unstack");
    harness.pump();
    assert!(harness.state.desktop.stackbars.is_empty());
    assert!(harness.shown(editor) && harness.shown(terminal));
}

#[test]
fn a_rule_on_the_application_id_applies_to_a_wayland_window() {
    let mut config = Config::new();
    config.desktop.window_rules.push(
        serde_json::from_value(serde_json::json!({
            "name": "by application id",
            "enabled": true,
            "dynamic": false,
            "mode": "All",
            "predicates": [{"kind": "AppId", "strategy": "Equals", "value": "org.test.terminal",
                            "negate": false, "case_sensitive": false}],
            "effects": ["Ignore"],
        }))
        .expect("rule"),
    );
    let mut harness = Harness::new(config);
    let client = harness.connect();
    let (_, editor) = harness.open(client, "editor", "org.test.editor");
    let alone = harness.rect(editor);
    let (_, terminal) = harness.open(client, "terminal", "org.test.terminal");

    assert_eq!(harness.state.desktop.app_id(terminal), "org.test.terminal");
    // Ignored by its id: it is not tiled, and the other keeps the output.
    assert_eq!(
        harness.state.core.monitor_manager.find_window(terminal),
        None
    );
    assert_eq!(harness.rect(editor), alone);
}

#[test]
fn a_closed_window_leaves_with_its_transition_and_borders_change_colour_gradually() {
    use gevrek_engine::Desktop;
    use std::time::{Duration, Instant};
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (first_window, first) = harness.open(client, "editor", "org.test.editor");
    let (second_window, second) = harness.open(client, "terminal", "org.test.terminal");
    let settle = |harness: &mut Harness| {
        harness
            .state
            .desktop
            .tick_animations(Instant::now() + Duration::from_secs(5));
        harness.pump();
    };
    settle(&mut harness);
    let look = |harness: &Harness, id| harness.state.desktop.windows[&id].border.expect("a border");
    assert!(look(&harness, second).focused && !look(&harness, first).focused);

    // The focus moves: both borders are on their way to the other look.
    harness.state.focus_window(first);
    harness.pump();
    assert!(look(&harness, first).focused && !look(&harness, second).focused);
    let starting = harness.state.desktop.windows[&first]
        .drawn_border
        .expect("in transition");
    assert_eq!(
        starting.0,
        look(&harness, first).inactive_color,
        "from the unfocused colour"
    );
    harness
        .state
        .desktop
        .tick_animations(Instant::now() + Duration::from_millis(40));
    let midway = harness.state.desktop.windows[&first]
        .drawn_border
        .expect("still changing");
    assert_ne!(midway.0, look(&harness, first).inactive_color);
    assert_ne!(midway.0, look(&harness, first).active_color);
    settle(&mut harness);
    assert!(harness.state.desktop.windows[&first].drawn_border.is_none());
    assert!(harness.state.desktop.windows[&second]
        .drawn_border
        .is_none());

    // Closing: the window fades first, and its client is asked to close
    // only when the fade is over.
    harness
        .state
        .desktop
        .apply(&gevrek_engine::Effect::Close(second))
        .expect("close");
    harness.pump();
    assert!(!harness.clients[client].app.windows[second_window].close_requested);
    assert!(harness.state.desktop.windows[&second]
        .drawn_opacity
        .is_some());
    settle(&mut harness);
    assert!(harness.clients[client].app.windows[second_window].close_requested);
    assert!(!harness.clients[client].app.windows[first_window].close_requested);
    // A client that does not close is shown again.
    assert!(harness.state.desktop.windows[&second]
        .drawn_opacity
        .is_none());
}
