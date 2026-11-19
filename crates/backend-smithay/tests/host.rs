//! The host side of the Wayland WM: key bindings, commands, configuration
//! reload, and the channels of the IPC server. Real clients, real key
//! events through xkb, nothing drawn.
#![cfg(target_os = "linux")]

mod common;

use common::{Harness, SCREEN};
use gevrek_engine::layouts::LayoutType;
use gevrek_engine::WindowQueries;
use gevrek_host::command::{ManualCommand, RuntimeCommand, RuntimeCommandResult};
use gevrek_host::config_store::{config_channel, ConfigRequest};
use gevrek_model::command::{DispatchCommand, WorkspaceSelector};
use gevrek_model::config::Config;
use gevrek_model::settings::BindingProfile;
use gevrek_smithay::host::HostLinks;
use std::sync::{Arc, Mutex};

// evdev key codes (linux/input-event-codes.h).
const KEY_A: u32 = 30;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_F9: u32 = 67;
const KEY_F10: u32 = 68;

fn binding(key: &str, alt: bool, command: DispatchCommand) -> BindingProfile {
    BindingProfile {
        name: format!("test {key}"),
        key: key.to_string(),
        alt,
        win: false,
        command,
        ..BindingProfile::default()
    }
}

fn config_with(bindings: Vec<BindingProfile>) -> Config {
    let mut config = Config::new();
    config.desktop.bindings = bindings;
    config
}

#[test]
fn a_key_binding_runs_its_command_and_keeps_the_key_from_the_client() {
    let mut harness = Harness::new(config_with(vec![binding(
        "F9",
        true,
        DispatchCommand::CloseActive,
    )]));
    let client = harness.connect();
    let (window, _) = harness.open(client, "editor", "org.test.editor");

    // An ordinary key goes to the focused client.
    harness.chord(&[KEY_A]);
    assert_eq!(
        harness.clients[client].app.keys,
        [(KEY_A, true), (KEY_A, false)]
    );
    assert!(!harness.clients[client].app.windows[window].close_requested);

    // Alt+F9 is bound: the command runs, and the client sees Alt but
    // neither the press nor the release of F9.
    harness.clients[client].app.keys.clear();
    harness.chord(&[KEY_LEFTALT, KEY_F9]);
    assert!(harness.clients[client].app.windows[window].close_requested);
    assert_eq!(
        harness.clients[client].app.keys,
        [(KEY_LEFTALT, true), (KEY_LEFTALT, false)]
    );

    // F9 alone, or with another modifier, is not the binding.
    harness.clients[client].app.windows[window].close_requested = false;
    harness.clients[client].app.keys.clear();
    harness.chord(&[KEY_F9]);
    harness.chord(&[KEY_LEFTSHIFT, KEY_LEFTALT, KEY_F9]);
    assert!(!harness.clients[client].app.windows[window].close_requested);
    assert_eq!(harness.clients[client].app.keys.len(), 2 + 6);
}

#[test]
fn bindings_follow_the_active_submap_and_rest_while_paused() {
    let mut resize = binding("F10", true, DispatchCommand::CloseActive);
    resize.submap = "resize".to_string();
    let enter = binding(
        "F9",
        true,
        DispatchCommand::SetBindingSubmap {
            name: "resize".to_string(),
        },
    );
    let mut harness = Harness::new(config_with(vec![enter, resize]));
    let client = harness.connect();
    let (window, _) = harness.open(client, "editor", "org.test.editor");
    let closed = |harness: &Harness| harness.clients[client].app.windows[window].close_requested;

    // In the default submap Alt+F10 is not bound.
    harness.chord(&[KEY_LEFTALT, KEY_F10]);
    assert!(!closed(&harness));

    // Alt+F9 enters the submap; paused, its binding does not run.
    harness.chord(&[KEY_LEFTALT, KEY_F9]);
    assert_eq!(harness.state.active_submap, "resize");
    harness.state.paused = true;
    harness.chord(&[KEY_LEFTALT, KEY_F10]);
    assert!(!closed(&harness));

    harness.state.paused = false;
    harness.chord(&[KEY_LEFTALT, KEY_F10]);
    assert!(closed(&harness));

    harness
        .state
        .run_command(DispatchCommand::ResetBindingSubmap)
        .expect("reset");
    assert_eq!(harness.state.active_submap, "default");
}

#[test]
fn a_reload_applies_new_rules_and_bindings_and_keeps_the_windows() {
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (_, editor) = harness.open(client, "editor", "org.test.editor");
    let alone = harness.rect(editor);
    let (terminal_window, terminal) = harness.open(client, "terminal", "org.test.terminal");
    let shared = harness.rect(editor);
    assert!(shared.width < alone.width);

    // The same configuration again changes nothing.
    harness
        .state
        .reload_configuration(harness.state.config.clone());
    harness.pump();
    assert_eq!(harness.rect(editor), shared);

    // A rule that ignores the terminal, and a binding.
    let mut config = harness.state.config.clone();
    config.desktop.window_rules.push(
        serde_json::from_value(serde_json::json!({
            "name": "leave terminals alone",
            "enabled": true,
            "dynamic": false,
            "mode": "All",
            "predicates": [{"kind": "Class", "strategy": "Contains", "value": "terminal",
                            "negate": false, "case_sensitive": false}],
            "effects": ["Ignore"],
        }))
        .expect("rule"),
    );
    config
        .desktop
        .bindings
        .push(binding("F9", true, DispatchCommand::CloseActive));
    harness.state.reload_configuration(config);
    harness.pump();

    // The ignored window left the layout and is still there and shown; the
    // other one has the whole output again.
    assert_eq!(
        harness.state.core.monitor_manager.find_window(terminal),
        None
    );
    assert!(harness.state.desktop.exists(terminal) && harness.shown(terminal));
    assert_eq!(harness.rect(editor), alone);

    // The new binding is live.
    harness.state.focus_window(terminal);
    harness.chord(&[KEY_LEFTALT, KEY_F9]);
    assert!(harness.clients[client].app.windows[terminal_window].close_requested);
}

#[test]
fn commands_and_configurations_from_the_ipc_server_reach_the_wm() {
    let mut harness = Harness::new(Config::new());
    let output = harness.output;
    harness
        .state
        .core
        .monitor_manager
        .create_workspace(output, "two".to_string(), LayoutType::BSP)
        .expect("second workspace");
    let client = harness.connect();
    let (_, window) = harness.open(client, "editor", "org.test.editor");

    // The channels the gRPC server holds the other ends of.
    let (config_sender, config_receiver) = std::sync::mpsc::channel();
    let (command_sender, command_receiver) = std::sync::mpsc::channel();
    let (config_publisher, config_watch) = config_channel(harness.state.config.clone());
    let overlay_data = Arc::new(Mutex::new(None));
    let stop_flag = Arc::new(Mutex::new(false));
    harness.state.attach_host(HostLinks {
        config_receiver,
        command_receiver,
        config_publisher,
        config_path: None,
        overlay_data: Arc::clone(&overlay_data),
        pause_flag: Arc::new(Mutex::new(false)),
        stop_flag: Arc::clone(&stop_flag),
    });

    // The state snapshot is published for the server to read.
    {
        let data = overlay_data.lock().unwrap();
        let data = data.as_ref().expect("published at attach");
        assert_eq!(data.total_windows, 1);
        assert_eq!(data.monitors.len(), 1);
        assert_eq!(data.monitors[0].width, SCREEN.width);
        // An output with no display behind it is known by its name.
        assert_eq!(data.monitors[0].stable_id, "port:TEST-1");
    }

    // A command: answered, and carried out.
    let (response, mut answer) = tokio::sync::oneshot::channel();
    command_sender
        .send(ManualCommand {
            command: RuntimeCommand::Dispatch(DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("two".to_string()),
            }),
            response,
        })
        .unwrap();
    assert!(harness.state.host_tick());
    harness.pump();
    assert_eq!(
        answer.try_recv().unwrap(),
        Ok(RuntimeCommandResult::Complete)
    );
    assert!(!harness.shown(window), "its workspace is hidden now");

    // A command that fails says why.
    let (response, mut answer) = tokio::sync::oneshot::channel();
    command_sender
        .send(ManualCommand {
            command: RuntimeCommand::Dispatch(DispatchCommand::SetBindingSubmap {
                name: "nowhere".to_string(),
            }),
            response,
        })
        .unwrap();
    harness.state.host_tick();
    let error = answer.try_recv().unwrap().expect_err("no such submap");
    assert!(error.contains("nowhere"), "{error}");

    // A configuration from the current revision is applied and published
    // as a new revision; one from a stale revision is refused.
    let base = config_watch.borrow().revision;
    let mut edited = harness.state.config.clone();
    edited.desktop.focus.history_size = 7;
    config_sender
        .send(ConfigRequest::ReplaceFrom {
            config: Box::new(edited.clone()),
            base_revision: base,
        })
        .unwrap();
    harness.state.host_tick();
    assert_eq!(harness.state.config.desktop.focus.history_size, 7);
    let applied = config_watch.borrow().revision;
    assert!(applied > base);
    assert_eq!(config_watch.borrow().config.desktop.focus.history_size, 7);

    edited.desktop.focus.history_size = 9;
    config_sender
        .send(ConfigRequest::ReplaceFrom {
            config: Box::new(edited),
            base_revision: base,
        })
        .unwrap();
    harness.state.host_tick();
    assert_eq!(harness.state.config.desktop.focus.history_size, 7);
    assert_eq!(config_watch.borrow().revision, applied);

    // The stop flag ends the loop.
    *stop_flag.lock().unwrap() = true;
    assert!(!harness.state.host_tick());
}

#[test]
fn a_floating_window_is_moved_and_resized_with_the_pointer() {
    use smithay::backend::input::KeyState;
    const KEY_LEFTMETA: u32 = 125;
    const LEFT: u32 = 0x110;
    const RIGHT: u32 = 0x111;

    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (_, window) = harness.open(client, "note", "org.test.note");
    harness
        .state
        .run_command(DispatchCommand::ToggleFloating)
        .expect("float");
    harness.pump();
    harness
        .state
        .desktop
        .tick_animations(std::time::Instant::now() + std::time::Duration::from_secs(5));
    assert!(harness
        .state
        .core
        .monitor_manager
        .is_window_floating(window));
    let before = harness.rect(window);

    // Super held, left button down inside the window, pointer moves.
    harness
        .state
        .key_event((KEY_LEFTMETA + 8).into(), KeyState::Pressed, 0);
    let grab = (f64::from(before.x + 30), f64::from(before.y + 30));
    harness.state.pointer_moved(grab.into(), 0);
    harness.state.pointer_button(LEFT, true, 0);
    assert!(harness.state.drag.is_some());
    harness
        .state
        .pointer_moved((grab.0 + 100.0, grab.1 + 50.0).into(), 0);
    harness.pump();
    let moved = harness.rect(window);
    assert_eq!((moved.x, moved.y), (before.x + 100, before.y + 50));
    assert_eq!((moved.width, moved.height), (before.width, before.height));
    harness.state.pointer_button(LEFT, false, 0);
    harness.pump();
    assert!(harness.state.drag.is_none());
    assert_eq!(
        harness.rect(window),
        moved,
        "a floating window stays where it was dropped"
    );

    // The right button resizes from the corner nearest to the pointer:
    // near the bottom right, that corner follows and the top left stays.
    let corner = (
        f64::from(moved.x + moved.width - 20),
        f64::from(moved.y + moved.height - 20),
    );
    harness.state.pointer_moved(corner.into(), 0);
    harness.state.pointer_button(RIGHT, true, 0);
    harness
        .state
        .pointer_moved((corner.0 + 60.0, corner.1 + 40.0).into(), 0);
    harness.pump();
    harness.state.pointer_button(RIGHT, false, 0);
    harness.pump();
    let resized = harness.rect(window);
    assert_eq!((resized.x, resized.y), (moved.x, moved.y));
    assert_eq!(
        (resized.width, resized.height),
        (moved.width + 60, moved.height + 40)
    );

    // Near the top left, that corner follows and the bottom right stays.
    let corner = (f64::from(resized.x + 20), f64::from(resized.y + 20));
    harness.state.pointer_moved(corner.into(), 0);
    harness.state.pointer_button(RIGHT, true, 0);
    harness
        .state
        .pointer_moved((corner.0 - 30.0, corner.1 - 20.0).into(), 0);
    harness.pump();
    harness.state.pointer_button(RIGHT, false, 0);
    harness.pump();
    let grown = harness.rect(window);
    assert_eq!((grown.x, grown.y), (resized.x - 30, resized.y - 20));
    assert_eq!(
        (grown.x + grown.width, grown.y + grown.height),
        (resized.x + resized.width, resized.y + resized.height)
    );
    let resized = grown;
    harness
        .state
        .key_event((KEY_LEFTMETA + 8).into(), KeyState::Released, 0);

    // Without Super a press is an ordinary click: nothing is dragged.
    harness.state.pointer_button(LEFT, true, 0);
    assert!(harness.state.drag.is_none());
    harness.state.pointer_button(LEFT, false, 0);
    assert_eq!(harness.rect(window), resized);
}

#[test]
fn bindings_on_mouse_buttons_the_wheel_long_presses_and_repeats() {
    use gevrek_model::settings::{BindingDevice, BindingTrigger};
    use smithay::backend::input::KeyState;
    use std::time::{Duration, Instant};
    const MIDDLE: u32 = 0x112;

    let submap = |name: &str| DispatchCommand::SetBindingSubmap {
        name: name.to_string(),
    };
    // Each binding enters a submap of its own, so which one ran is visible.
    let mut bindings = Vec::new();
    for name in ["middle", "wheel", "long", "repeat"] {
        let mut marker = binding("F9", false, DispatchCommand::FocusNext);
        marker.submap = name.to_string();
        bindings.push(marker);
    }
    let mut middle = binding("MIDDLE", true, submap("middle"));
    middle.device = BindingDevice::Mouse;
    let mut wheel = binding("WHEEL_UP", true, submap("wheel"));
    wheel.device = BindingDevice::Mouse;
    let mut long = binding("F10", false, submap("long"));
    long.trigger = BindingTrigger::LongPress;
    long.long_press_ms = 300;
    let mut repeat = binding("F9", false, submap("repeat"));
    repeat.trigger = BindingTrigger::Repeat;
    bindings.extend([middle, wheel, long, repeat]);
    let mut harness = Harness::new(config_with(bindings));
    let client = harness.connect();
    harness.open(client, "editor", "org.test.editor");
    let reset = |harness: &mut Harness| {
        harness
            .state
            .run_command(DispatchCommand::ResetBindingSubmap)
            .unwrap();
        assert_eq!(harness.state.active_submap, "default");
    };

    // Alt + middle button, and only with Alt.
    harness.state.pointer_button(MIDDLE, true, 0);
    harness.state.pointer_button(MIDDLE, false, 0);
    assert_eq!(harness.state.active_submap, "default");
    harness
        .state
        .key_event((KEY_LEFTALT + 8).into(), KeyState::Pressed, 0);
    harness.state.pointer_button(MIDDLE, true, 0);
    harness.state.pointer_button(MIDDLE, false, 0);
    assert_eq!(harness.state.active_submap, "middle");
    reset(&mut harness);

    // Alt + wheel up is taken by its binding; wheel down is not bound.
    assert!(!harness.state.pointer_wheel(false));
    assert!(harness.state.pointer_wheel(true));
    assert_eq!(harness.state.active_submap, "wheel");
    harness
        .state
        .key_event((KEY_LEFTALT + 8).into(), KeyState::Released, 0);
    reset(&mut harness);

    // A long press runs once, and only after the key was down long enough.
    let start = Instant::now();
    harness
        .state
        .key_event((KEY_F10 + 8).into(), KeyState::Pressed, 0);
    harness.state.tick_held(start + Duration::from_millis(100));
    assert_eq!(harness.state.active_submap, "default");
    harness.state.tick_held(start + Duration::from_millis(400));
    assert_eq!(harness.state.active_submap, "long");
    reset(&mut harness);
    harness.state.tick_held(start + Duration::from_millis(900));
    assert_eq!(harness.state.active_submap, "default", "once per press");
    harness
        .state
        .key_event((KEY_F10 + 8).into(), KeyState::Released, 0);
    // A short press never runs it.
    harness
        .state
        .key_event((KEY_F10 + 8).into(), KeyState::Pressed, 0);
    harness
        .state
        .key_event((KEY_F10 + 8).into(), KeyState::Released, 0);
    harness
        .state
        .tick_held(Instant::now() + Duration::from_secs(2));
    assert_eq!(harness.state.active_submap, "default");

    // A repeat binding runs again and again while the key stays down.
    let start = Instant::now();
    harness
        .state
        .key_event((KEY_F9 + 8).into(), KeyState::Pressed, 0);
    harness.state.tick_held(start + Duration::from_millis(100));
    assert_eq!(
        harness.state.active_submap, "default",
        "not before the delay"
    );
    harness.state.tick_held(start + Duration::from_millis(500));
    assert_eq!(harness.state.active_submap, "repeat");
    harness
        .state
        .key_event((KEY_F9 + 8).into(), KeyState::Released, 0);
    assert!(harness.state.held.is_empty());
}

#[test]
fn a_program_on_the_whitelist_may_take_the_focus() {
    let mut harness = Harness::new(Config::new());
    assert!(harness.state.config.desktop.focus.prevent_focus_steal);
    let focus = &harness.state.core.focus_manager;
    assert!(!focus.can_steal_focus("anything"));
    let mut config = harness.state.config.clone();
    config
        .desktop
        .focus
        .focus_steal_whitelist
        .insert("launcher".to_string());
    harness.state.reload_configuration(config);
    assert!(harness.state.core.focus_manager.can_steal_focus("launcher"));
    assert!(!harness.state.core.focus_manager.can_steal_focus("anything"));
}

#[test]
fn a_touchpad_swipe_runs_the_command_of_its_profile() {
    use gevrek_model::settings::{GestureDirection, GestureProfile};
    let mut config = Config::new();
    config.desktop.gestures.enabled = true;
    config.desktop.gestures.profiles = vec![GestureProfile {
        extra: Default::default(),
        name: "three left".to_string(),
        enabled: true,
        fingers: 3,
        direction: GestureDirection::Left,
        distance_percent: 20.0,
        command: DispatchCommand::SetBindingSubmap {
            name: "swiped".to_string(),
        },
    }];
    let mut marker = binding("F9", false, DispatchCommand::FocusNext);
    marker.submap = "swiped".to_string();
    config.desktop.bindings = vec![marker];
    let mut harness = Harness::new(config);
    let swipe = |harness: &mut Harness, fingers: u32, steps: &[(f64, f64)], cancelled: bool| {
        harness.state.swipe_begin(fingers);
        for (dx, dy) in steps {
            harness.state.swipe_update(*dx, *dy);
        }
        harness.state.swipe_end(cancelled)
    };
    let reset = |harness: &mut Harness| {
        harness
            .state
            .run_command(DispatchCommand::ResetBindingSubmap)
            .unwrap();
    };
    // 20 percent of the smaller side of a 1920x1080 output is 216.

    // Too short, the wrong number of fingers, the wrong way, a diagonal, a
    // cancelled swipe: nothing runs.
    assert!(!swipe(
        &mut harness,
        3,
        &[(-100.0, 0.0), (-100.0, 4.0)],
        false
    ));
    assert!(!swipe(&mut harness, 4, &[(-300.0, 0.0)], false));
    assert!(!swipe(&mut harness, 3, &[(300.0, 0.0)], false));
    assert!(!swipe(&mut harness, 3, &[(-300.0, -280.0)], false));
    assert!(!swipe(&mut harness, 3, &[(-300.0, 0.0)], true));
    assert_eq!(harness.state.active_submap, "default");

    // Three fingers, left, far enough, in several steps.
    assert!(swipe(
        &mut harness,
        3,
        &[(-90.0, 3.0), (-90.0, -2.0), (-60.0, 0.0)],
        false
    ));
    assert_eq!(harness.state.active_submap, "swiped");
    let progress = harness.state.gesture_progress.clone().expect("reported");
    assert_eq!(
        (progress.phase.as_str(), progress.fingers),
        ("completed", 3)
    );
    assert_eq!(progress.direction.as_deref(), Some("left"));
    reset(&mut harness);

    // Paused, or with gestures turned off, it does not run.
    harness.state.paused = true;
    assert!(!swipe(&mut harness, 3, &[(-300.0, 0.0)], false));
    harness.state.paused = false;
    let mut off = harness.state.config.clone();
    off.desktop.gestures.enabled = false;
    harness.state.reload_configuration(off);
    assert!(!swipe(&mut harness, 3, &[(-300.0, 0.0)], false));
    assert_eq!(harness.state.active_submap, "default");
}

#[test]
fn ctrl_alt_f_keys_are_ordinary_chords_unless_gevrek_owns_a_virtual_terminal() {
    const KEY_LEFTCTRL: u32 = 29;
    const KEY_F11: u32 = 87;
    let mut chord = binding("F11", true, DispatchCommand::CloseActive);
    chord.ctrl = true;
    let mut harness = Harness::new(config_with(vec![chord]));
    let client = harness.connect();
    let (window, _) = harness.open(client, "editor", "org.test.editor");

    // On real hardware the chord asks for virtual terminal 11 and the
    // binding does not run.
    harness.state.vt_switching = true;
    harness.chord(&[KEY_LEFTCTRL, KEY_LEFTALT, KEY_F11]);
    assert_eq!(harness.state.vt_request.take(), Some(11));
    assert!(!harness.clients[client].app.windows[window].close_requested);

    // Nested in a window it is the binding's.
    harness.state.vt_switching = false;
    harness.chord(&[KEY_LEFTCTRL, KEY_LEFTALT, KEY_F11]);
    assert_eq!(harness.state.vt_request, None);
    assert!(harness.clients[client].app.windows[window].close_requested);
}

#[test]
fn the_overview_of_the_windows_has_titles_a_mark_and_keys() {
    // evdev: Escape, Enter, the right arrow.
    const KEY_ESC: u32 = 1;
    const KEY_ENTER: u32 = 28;
    const KEY_RIGHT: u32 = 106;
    let mut harness = Harness::new(Config::new());
    let client = harness.connect();
    let (_, editor) = harness.open(client, "editor", "org.test.editor");
    let (_, terminal) = harness.open(client, "terminal", "org.test.terminal");
    let focused = harness.state.desktop.focused.expect("a window has the keyboard");
    let other = if focused == editor { terminal } else { editor };
    let before = (harness.rect(editor), harness.rect(terminal));
    assert!(!harness.state.window_overview_is_up());

    // The overview: a picture of each, smaller, with its title, and the
    // window with the keyboard pointed at.
    harness.state.run_command(DispatchCommand::ToggleExpose).unwrap();
    harness.pump();
    harness.state.sync_window_overview();
    assert!(harness.state.window_overview_is_up());
    let shown = &harness.state.window_overview.shown;
    let mut titles: Vec<&str> = shown.iter().map(|shown| shown.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, ["editor", "terminal"]);
    assert!(shown.iter().all(|shown| shown.rect.width < before.0.width));
    assert_eq!(harness.state.window_overview.marked, Some(focused));
    // (The windows themselves are as large as they were.)
    assert_eq!((harness.rect(editor), harness.rect(terminal)), before);

    // The keys are the overview's: nothing of them reaches the program.
    harness.clients[client].app.keys.clear();
    // An arrow goes to the other picture (whichever side it is on, one
    // of the two arrows does; Tab goes to the next in any case).
    harness.chord(&[KEY_RIGHT]);
    if harness.state.window_overview.marked == Some(focused) {
        const KEY_TAB: u32 = 15;
        harness.chord(&[KEY_TAB]);
    }
    assert_eq!(harness.state.window_overview.marked, Some(other));
    // Enter goes to it: the overview is over, and it has the keyboard.
    harness.chord(&[KEY_ENTER]);
    harness.state.sync_window_overview();
    assert!(!harness.state.window_overview_is_up());
    assert!(harness.state.window_overview.shown.is_empty());
    assert_eq!(harness.state.desktop.focused, Some(other));
    assert!(harness.clients[client].app.keys.is_empty());

    // Up again, and Escape puts it away with nothing chosen.
    harness.state.run_command(DispatchCommand::ToggleExpose).unwrap();
    harness.pump();
    harness.state.sync_window_overview();
    assert!(harness.state.window_overview_is_up());
    harness.chord(&[KEY_ESC]);
    harness.state.sync_window_overview();
    assert!(!harness.state.window_overview_is_up());
    assert_eq!(harness.state.desktop.focused, Some(other));
    assert!(harness.clients[client].app.keys.is_empty());
    // Away, the keys are the program's again.
    harness.chord(&[KEY_A]);
    assert_eq!(harness.clients[client].app.keys.len(), 2);
}

#[test]
fn the_pointer_left_in_a_corner_runs_that_corners_command() {
    use gevrek_model::settings::{HotCorner, HotCornersProfile, ScreenCorner};
    use std::time::{Duration, Instant};
    // The corner enters a submap of its own, so that it is seen to run.
    let mut marker = binding("F9", false, DispatchCommand::FocusNext);
    marker.submap = "corner".to_string();
    let mut config = config_with(vec![marker]);
    config.desktop.hot_corners = HotCornersProfile {
        corners: vec![HotCorner {
            corner: ScreenCorner::TopRight,
            command: DispatchCommand::SetBindingSubmap {
                name: "corner".to_string(),
            },
            ..HotCorner::default()
        }],
        ..HotCornersProfile::default()
    };
    let mut harness = Harness::new(config);
    let start = Instant::now();
    // Elsewhere, and in another corner: nothing.
    harness.state.pointer_moved((400.0, 300.0).into(), 0);
    harness.state.pointer_moved((0.0, 0.0).into(), 0);
    harness.state.tick_hot_corners(start + Duration::from_secs(2));
    assert_eq!(harness.state.active_submap, "default");
    assert!(!harness.state.hot_corners.waits());
    // In its corner: not at once, and the host is asked to look again.
    harness.state.pointer_moved((1919.0, 0.0).into(), 0);
    assert_eq!(harness.state.active_submap, "default");
    assert!(harness.state.hot_corners.waits());
    // Left there for its moment: once.
    harness.state.tick_hot_corners(Instant::now() + Duration::from_millis(600));
    assert_eq!(harness.state.active_submap, "corner");
    assert!(!harness.state.hot_corners.waits());
}

#[test]
fn the_pointer_stays_on_the_outputs() {
    let mut harness = Harness::new(Config::new());
    // A place beyond every edge of the 1920x1080 output, as a host reports
    // it when the mouse is on the frame of a nested window.
    for (given, kept) in [
        ((-25.0, -40.0), (0, 0)),
        ((5000.0, 300.0), (1919, 300)),
        ((640.0, 9000.0), (640, 1079)),
        ((100.0, 200.0), (100, 200)),
    ] {
        let location = harness.state.clamp_to_outputs(given.into());
        harness.state.pointer_moved(location, 0);
        assert_eq!(harness.state.desktop.pointer, kept, "{given:?}");
    }
}

#[test]
fn an_activation_request_follows_the_focus_rules_and_the_steal_policy() {
    fn rule(name: &str, class: &str, effect: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "name": name, "enabled": true, "dynamic": false, "mode": "All",
            "predicates": [{"kind": "Class", "strategy": "Contains", "value": class,
                            "negate": false, "case_sensitive": false}],
            "effects": [effect],
        })
    }
    let mut config = Config::new();
    for rule in [
        rule(
            "never",
            "org.test.never",
            serde_json::json!({"NoFocus": true}),
        ),
        rule(
            "not at first",
            "org.test.shy",
            serde_json::json!({"NoInitialFocus": true}),
        ),
    ] {
        config
            .desktop
            .window_rules
            .push(serde_json::from_value(rule).expect("rule"));
    }
    let mut harness = Harness::new(config);
    let client = harness.connect();
    let focused = |harness: &Harness| harness.state.desktop.focused;
    let (_, plain) = harness.open(client, "plain", "org.test.plain");
    // Neither ruled window keeps the focus as it opens.
    let (_, never) = harness.open(client, "never", "org.test.never");
    assert_eq!(focused(&harness), Some(plain));
    let (_, shy) = harness.open(client, "shy", "org.test.shy");
    assert_eq!(focused(&harness), Some(plain));
    let (_, other) = harness.open(client, "other", "org.test.other");
    assert_eq!(focused(&harness), Some(other));
    assert!(harness.state.core.executor.no_focus_enabled(never));
    assert!(harness.state.core.executor.no_initial_focus_enabled(shy));
    assert!(!harness.state.desktop.had_focus.contains(&never));
    assert!(!harness.state.desktop.had_focus.contains(&shy));

    // The steal policy: a stale token, or one not from a user action, is
    // refused; a fresh one from a user action gives the focus.
    assert!(!harness.state.activation_requested(plain, false, true));
    assert!(!harness.state.activation_requested(plain, true, false));
    assert_eq!(focused(&harness), Some(other));
    assert!(harness.state.activation_requested(plain, true, true));
    assert_eq!(focused(&harness), Some(plain));

    // `NoFocus`: never, whatever the token.
    assert!(!harness.state.activation_requested(never, true, true));
    assert_eq!(focused(&harness), Some(plain));

    // `NoInitialFocus`: not before the window has had the focus once.
    assert!(!harness.state.activation_requested(shy, true, true));
    assert_eq!(focused(&harness), Some(plain));
    harness.state.focus_window(shy);
    harness.state.focus_window(plain);
    assert!(harness.state.activation_requested(shy, true, true));
    assert_eq!(focused(&harness), Some(shy));
}

#[test]
fn a_window_is_told_the_output_it_is_on_and_its_scale() {
    use gevrek_model::command::MonitorTarget;
    let mut config = Config::new();
    config
        .desktop
        .platform
        .wayland
        .output_scales
        .insert("TEST-2".to_string(), 2.0);
    let mut harness = Harness::new(config);
    // Each output has the scale the configuration gives its name.
    assert_eq!(harness.state.configured_scale("TEST-1"), 1.0);
    let scale = harness.state.configured_scale("TEST-2");
    assert_eq!(scale, 2.0);
    let first = harness.output;
    let second = harness.state.add_scaled_output(
        "TEST-2",
        gevrek_engine::Rect::new(SCREEN.width, 0, 1280, 720),
        scale,
    );
    harness.state.output_added(second);
    harness.pump();
    let client = harness.connect();
    let (_, window) = harness.open(client, "editor", "org.test.editor");
    assert_eq!(
        harness.state.surface_outputs.get(&window),
        Some(&(first, 1.0))
    );

    // Moved to the other output, it is told that one and its scale.
    harness
        .state
        .run_command(DispatchCommand::MoveWindowToMonitor {
            window: gevrek_model::command::WindowSelector::Active,
            target: MonitorTarget::Next,
        })
        .expect("move to the second monitor");
    harness.pump();
    harness.finish_transitions();
    assert!(
        harness.rect(window).x >= SCREEN.width,
        "{:?}",
        harness.rect(window)
    );
    assert_eq!(
        harness.state.surface_outputs.get(&window),
        Some(&(second, 2.0))
    );

    // The output goes away: the window is on the first one again.
    harness.state.remove_output(second);
    harness.pump();
    harness.finish_transitions();
    assert_eq!(
        harness.state.surface_outputs.get(&window),
        Some(&(first, 1.0))
    );
}

#[test]
fn the_view_of_a_strip_follows_three_fingers_and_stays_where_they_leave_it() {
    let mut config = Config::new();
    config.desktop.animations.enabled = true;
    let mut harness = Harness::new(config);
    harness
        .state
        .run_command(DispatchCommand::SetLayout {
            layout: gevrek_model::layout::LayoutType::Scrolling,
        })
        .expect("the scrolling layout");
    let client = harness.connect();
    let windows: Vec<_> = (0..4)
        .map(|index| {
            harness
                .open(client, &format!("column {index}"), "org.test.column")
                .1
        })
        .collect();
    harness.pump();
    let settle = |harness: &mut Harness| {
        harness
            .state
            .desktop
            .tick_animations(std::time::Instant::now() + std::time::Duration::from_secs(5));
    };
    settle(&mut harness);
    assert!(harness.state.strip_under_pointer(), "a strip is shown");
    assert!(
        harness.state.swipe_is_ours(3),
        "three fingers are the strip's"
    );
    assert!(!harness.state.swipe_is_ours(4), "four are not");
    // The keyboard is on the last column, which is in view: the first is not.
    let before: Vec<_> = windows.iter().map(|id| harness.rect(*id)).collect();
    assert!(
        before[0].x < 0,
        "the first column is off to the left: {before:?}"
    );

    // Fingers to the right bring what is on the left into view, by as
    // much as they move and at once (no transition is under way).
    harness.state.swipe_begin(3);
    harness.state.swipe_update(10.0, 1.0);
    harness.state.swipe_update(30.0, -1.0);
    let during: Vec<_> = windows.iter().map(|id| harness.rect(*id)).collect();
    for (was, is) in before.iter().zip(&during) {
        assert_eq!(is.x - was.x, 100, "40 units of the fingers are 100 pixels");
        assert_eq!((is.y, is.width, is.height), (was.y, was.width, was.height));
    }
    assert!(
        windows
            .iter()
            .all(|id| harness.state.desktop.windows[id].drawn.is_none()),
        "the columns are where they belong, not on their way"
    );
    assert!(harness.state.swipe_end(false), "the swipe was the strip's");
    settle(&mut harness);
    let after: Vec<_> = windows.iter().map(|id| harness.rect(*id)).collect();
    assert_eq!(after, during, "the view stays where the fingers left it");

    // A long way: the strip ends at its first column, and the column
    // that had the keyboard, out of view now, has handed it on.
    harness.state.swipe_begin(3);
    for _ in 0..40 {
        harness.state.swipe_update(50.0, 0.0);
    }
    harness.state.swipe_end(false);
    settle(&mut harness);
    let first = harness.rect(windows[0]);
    assert!(
        first.x >= 0,
        "the strip stops at its first column: {first:?}"
    );
    let focused = harness
        .state
        .desktop
        .focused
        .expect("a column has the keyboard");
    assert_ne!(
        focused, windows[3],
        "the column out of view gave the keyboard away"
    );
    let rect = harness.rect(focused);
    assert!(
        rect.x < 1280 && rect.x + rect.width > 0,
        "to one in view: {rect:?}"
    );

    // Up and down is no business of the strip's.
    let here: Vec<_> = windows.iter().map(|id| harness.rect(*id)).collect();
    harness.state.swipe_begin(3);
    harness.state.swipe_update(2.0, -60.0);
    harness.state.swipe_update(-30.0, -60.0);
    assert!(!harness.state.swipe_end(false));
    let there: Vec<_> = windows.iter().map(|id| harness.rect(*id)).collect();
    assert_eq!(here, there, "a swipe upwards leaves the view");
}

#[test]
fn a_swipe_pulls_the_workspace_beside_into_view_and_the_old_one_goes_out() {
    use gevrek_model::settings::{GestureDirection, GestureProfile};
    let mut config = Config::new();
    config.desktop.animations.enabled = true;
    config.desktop.gestures.enabled = true;
    config.desktop.gestures.profiles = [
        (GestureDirection::Left, WorkspaceSelector::Next),
        (GestureDirection::Right, WorkspaceSelector::Previous),
    ]
    .into_iter()
    .map(|(direction, selector)| GestureProfile {
        extra: Default::default(),
        name: format!("{direction:?}"),
        enabled: true,
        fingers: 3,
        direction,
        distance_percent: 20.0,
        command: DispatchCommand::SwitchWorkspaceSelector { selector },
    })
    .collect();
    let mut harness = Harness::new(config);
    let later = || std::time::Instant::now() + std::time::Duration::from_secs(5);
    let client = harness.connect();
    let (_, first) = harness.open(client, "first", "org.test.first");
    harness
        .state
        .run_command(DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        })
        .expect("the second workspace");
    harness.pump();
    // A change of workspace: the old one's window is hidden at once (it
    // has neither keyboard nor pointer) and still drawn on its way out.
    assert!(!harness.shown(first), "the first window is hidden");
    assert!(
        harness.state.desktop.leaving(first),
        "and drawn while it goes out"
    );
    let width = harness.rect(first).width;
    harness.state.desktop.tick_animations(later());
    assert!(!harness.state.desktop.leaving(first), "until it is out");
    assert!(harness.state.desktop.windows[&first].drawn.is_none());

    let (_, second) = harness.open(client, "second", "org.test.second");
    harness.pump();
    harness.state.desktop.tick_animations(later());
    let home = harness.rect(second);
    assert_eq!(home.width, width);

    // Fingers to the left, a little: the next workspace (the first: there
    // are two) comes in from the right as the shown one goes out, by as
    // much as the fingers went.
    harness.state.swipe_begin(3);
    harness.state.swipe_update(-20.0, 0.0);
    harness.state.swipe_update(-88.0, 2.0);
    let drawn = |harness: &Harness, id| harness.state.desktop.windows[&id].drawn;
    let out = drawn(&harness, second).expect("the shown window is pulled");
    assert!(
        out.x < home.x && out.y == home.y,
        "to the left: {out:?} from {home:?}"
    );
    let pulled = home.x - out.x;
    let coming = drawn(&harness, first).expect("the window beside is pulled in");
    assert!(harness.state.desktop.leaving(first), "hidden, and drawn");
    assert!(!harness.shown(first), "it is not to be used yet");
    assert!(
        coming.x > home.x && coming.x - harness.rect(first).x + pulled > 1000,
        "it comes from the right, a display apart: {coming:?}, pulled {pulled}"
    );
    // Lifted before the profile's distance: everything goes back.
    assert!(
        !harness.state.swipe_end(false),
        "not far enough for the command"
    );
    assert!(harness.shown(second) && !harness.shown(first));
    harness.state.desktop.tick_animations(later());
    assert_eq!(drawn(&harness, second), None, "the shown window is back");
    assert!(
        !harness.state.desktop.leaving(first),
        "the other is out of view"
    );
    assert_eq!(drawn(&harness, first), None);

    // Far enough: the change is made, from where the fingers left it.
    harness.state.swipe_begin(3);
    harness.state.swipe_update(-150.0, 0.0);
    harness.state.swipe_update(-150.0, 0.0);
    let before = drawn(&harness, first).expect("pulled in");
    assert!(harness.state.swipe_end(false), "the command ran");
    harness.pump();
    assert!(
        harness.shown(first) && !harness.shown(second),
        "the workspaces changed"
    );
    assert_eq!(
        drawn(&harness, first).map(|rect| rect.x),
        Some(before.x),
        "it goes on from where it was pulled to"
    );
    assert!(harness.state.desktop.leaving(second), "the other goes out");
    harness.state.desktop.tick_animations(later());
    assert_eq!(drawn(&harness, first), None);
    assert!(!harness.state.desktop.leaving(second));
    assert!(harness.state.desktop.follow_workspace.is_none());
}

#[test]
fn the_overview_of_every_workspace_shows_them_all_and_goes_to_the_one_chosen() {
    let mut config = Config::new();
    config.desktop.animations.enabled = true;
    let mut harness = Harness::new(config);
    let later = || std::time::Instant::now() + std::time::Duration::from_secs(5);
    let client = harness.connect();
    let (_, first) = harness.open(client, "first", "org.test.first");
    harness
        .state
        .run_command(DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(1),
        })
        .expect("the second workspace");
    harness.pump();
    let (_, second) = harness.open(client, "second", "org.test.second");
    harness.pump();
    harness.state.desktop.tick_animations(later());
    let home = harness.rect(second);
    assert!(harness.shown(second) && !harness.shown(first));

    harness
        .state
        .run_command(DispatchCommand::WorkspaceOverview)
        .expect("the overview comes up");
    harness.state.desktop.tick_animations(later());
    let overview = harness.state.workspace_overview.as_ref().expect("it is up");
    let cells: Vec<_> = overview
        .cells
        .iter()
        .map(|cell| (cell.index, cell.shown, cell.rect, cell.windows.clone()))
        .collect();
    assert!(
        cells.len() >= 2,
        "every workspace has its picture: {}",
        cells.len()
    );
    let of = |window| {
        cells
            .iter()
            .find(|(_, _, _, windows)| windows.iter().any(|(id, _)| *id == window))
            .expect("the window is in a workspace's picture")
            .clone()
    };
    let (first_cell, second_cell) = (of(first), of(second));
    assert!(second_cell.1 && !first_cell.1, "the one shown is marked");
    assert_ne!(first_cell.0, second_cell.0);
    // Each window's picture lies inside its workspace's, smaller than
    // the window; the window itself keeps its size, and the hidden one
    // is drawn without being shown.
    for (window, cell) in [(first, &first_cell), (second, &second_cell)] {
        let picture = harness.state.desktop.windows[&window]
            .overview
            .expect("a picture");
        assert!(picture.width < home.width && picture.width > 0);
        assert!(
            picture.x >= cell.2.x && picture.x + picture.width <= cell.2.x + cell.2.width + 1,
            "{picture:?} in {:?}",
            cell.2
        );
    }
    assert_eq!(
        harness.rect(second),
        home,
        "no window is given another size"
    );
    assert!(!harness.shown(first), "a hidden window stays hidden");
    assert!(
        harness.state.desktop.drawn_in_order().contains(&first),
        "and is drawn in its workspace's picture"
    );

    // A press on the hidden window's picture: its workspace is shown, the
    // window has the keyboard, and the overview is gone.
    let picture = harness.state.desktop.windows[&first].overview.unwrap();
    harness.state.desktop.pointer = (
        picture.x + picture.width / 2,
        picture.y + picture.height / 2,
    );
    assert!(harness.state.workspace_overview_press(
        picture.x + picture.width / 2,
        picture.y + picture.height / 2
    ));
    harness.pump();
    assert!(harness.state.workspace_overview.is_none());
    assert!(
        harness.shown(first) && !harness.shown(second),
        "the chosen workspace is shown"
    );
    assert_eq!(harness.state.desktop.focused, Some(first));
    assert!(harness.state.desktop.decorations_visible);
    harness.state.desktop.tick_animations(later());
    assert!(harness
        .state
        .desktop
        .windows
        .values()
        .all(|window| window.overview.is_none() && window.drawn.is_none() && !window.in_overview));
    assert!(!harness.state.desktop.drawn_in_order().contains(&second));

    // Up again and put away with nothing chosen: all is as it was. A
    // window that comes while it is up puts it away too.
    harness
        .state
        .run_command(DispatchCommand::WorkspaceOverview)
        .expect("up again");
    harness
        .state
        .run_command(DispatchCommand::WorkspaceOverview)
        .expect("and away");
    assert!(harness.state.workspace_overview.is_none());
    assert!(harness.shown(first) && !harness.shown(second));
    harness
        .state
        .run_command(DispatchCommand::WorkspaceOverview)
        .expect("up a third time");
    let (_, third) = harness.open(client, "third", "org.test.third");
    harness.pump();
    assert!(
        harness.state.workspace_overview.is_none(),
        "a new window puts the overview away"
    );
    assert!(harness.shown(third));
}

#[test]
fn a_binding_on_release_runs_whichever_key_of_the_chord_is_let_go_first() {
    use smithay::backend::input::KeyState;
    let mut on_release = binding("F9", true, DispatchCommand::CloseActive);
    on_release.trigger = gevrek_model::settings::BindingTrigger::Release;
    let mut harness = Harness::new(config_with(vec![on_release]));
    let client = harness.connect();
    let (window, _) = harness.open(client, "editor", "org.test.editor");
    let key = |harness: &mut Harness, code: u32, state: KeyState| {
        harness.state.key_event((code + 8).into(), state, 0);
        harness.pump();
    };

    // Alt+F9 held: nothing yet, the binding is on the release.
    key(&mut harness, KEY_LEFTALT, KeyState::Pressed);
    key(&mut harness, KEY_F9, KeyState::Pressed);
    assert!(!harness.clients[client].app.windows[window].close_requested);
    // Alt is let go first, then F9: it was Alt+F9 that was held, and it
    // is Alt+F9 that is let go.
    key(&mut harness, KEY_LEFTALT, KeyState::Released);
    assert!(!harness.clients[client].app.windows[window].close_requested);
    key(&mut harness, KEY_F9, KeyState::Released);
    harness.finish_transitions();
    assert!(harness.clients[client].app.windows[window].close_requested);

    // F9 first, then Alt: the same.
    harness.clients[client].app.windows[window].close_requested = false;
    harness.chord(&[KEY_LEFTALT, KEY_F9]);
    assert!(harness.clients[client].app.windows[window].close_requested);

    // F9 pressed alone and let go with Alt held meanwhile was never
    // Alt+F9: nothing runs.
    harness.clients[client].app.windows[window].close_requested = false;
    key(&mut harness, KEY_F9, KeyState::Pressed);
    key(&mut harness, KEY_LEFTALT, KeyState::Pressed);
    key(&mut harness, KEY_F9, KeyState::Released);
    key(&mut harness, KEY_LEFTALT, KeyState::Released);
    harness.finish_transitions();
    assert!(!harness.clients[client].app.windows[window].close_requested);
}

#[test]
fn a_release_is_matched_in_the_submap_its_press_was_made_in() {
    use gevrek_model::settings::BindingTrigger;
    use smithay::backend::input::KeyState;
    let submap = |name: &str| DispatchCommand::SetBindingSubmap {
        name: name.to_string(),
    };
    // Alt+F9 goes into "held" when it goes down and back out when it
    // comes up: both are bindings of the default set.
    let enter = binding("F9", true, submap("held"));
    let mut leave = binding("F9", true, DispatchCommand::ResetBindingSubmap);
    leave.trigger = BindingTrigger::Release;
    // In "held": a release of Alt+F9 that must not run for a press made
    // outside it, and F10, which goes out when it comes up.
    let mut inside = binding("F9", true, submap("wrong"));
    inside.trigger = BindingTrigger::Release;
    inside.submap = "held".to_string();
    let mut out = binding("F10", false, DispatchCommand::ResetBindingSubmap);
    out.trigger = BindingTrigger::Release;
    out.submap = "held".to_string();
    let mut wrong = binding("F10", false, DispatchCommand::FocusNext);
    wrong.submap = "wrong".to_string();
    let mut harness = Harness::new(config_with(vec![enter, leave, inside, out, wrong]));
    let key = |harness: &mut Harness, code: u32, state: KeyState| {
        harness.state.key_event((code + 8).into(), state, 0);
        harness.pump();
    };

    key(&mut harness, KEY_LEFTALT, KeyState::Pressed);
    key(&mut harness, KEY_F9, KeyState::Pressed);
    assert_eq!(harness.state.active_submap, "held");
    // The release is the default set's (where the press was made), not
    // the one of the set that is live now.
    key(&mut harness, KEY_F9, KeyState::Released);
    key(&mut harness, KEY_LEFTALT, KeyState::Released);
    assert_eq!(harness.state.active_submap, "default");

    // A key pressed in "held" is let go in it: F10 leaves on its
    // release.
    harness.state.run_command(submap("held")).expect("the submap");
    key(&mut harness, KEY_F10, KeyState::Pressed);
    assert_eq!(harness.state.active_submap, "held");
    key(&mut harness, KEY_F10, KeyState::Released);
    assert_eq!(harness.state.active_submap, "default");
}

#[test]
fn a_mouse_buttons_release_is_the_release_of_what_was_pressed() {
    use gevrek_model::settings::{BindingDevice, BindingTrigger};
    use smithay::backend::input::KeyState;
    const MIDDLE: u32 = 0x112;
    let submap = |name: &str| DispatchCommand::SetBindingSubmap {
        name: name.to_string(),
    };
    // Alt+middle goes into "held" when it goes down and into "out" when
    // it comes up.
    let mut enter = binding("MIDDLE", true, submap("held"));
    enter.device = BindingDevice::Mouse;
    let mut leave = binding("MIDDLE", true, submap("out"));
    leave.device = BindingDevice::Mouse;
    leave.trigger = BindingTrigger::Release;
    let mut bindings = vec![enter, leave];
    for name in ["held", "out"] {
        let mut marker = binding("F9", false, DispatchCommand::FocusNext);
        marker.submap = name.to_string();
        bindings.push(marker);
    }
    let mut harness = Harness::new(config_with(bindings));
    let key = |harness: &mut Harness, code: u32, state: KeyState| {
        harness.state.key_event((code + 8).into(), state, 0);
        harness.pump();
    };

    // Alt let go before the button, and the set of bindings another by
    // then: the button's release is still Alt+middle's, of the set it
    // was pressed in.
    key(&mut harness, KEY_LEFTALT, KeyState::Pressed);
    harness.state.pointer_button(MIDDLE, true, 0);
    assert_eq!(harness.state.active_submap, "held");
    key(&mut harness, KEY_LEFTALT, KeyState::Released);
    harness.state.pointer_button(MIDDLE, false, 0);
    assert_eq!(harness.state.active_submap, "out");

    // The button pressed alone and let go with Alt held was never
    // Alt+middle: nothing runs.
    harness
        .state
        .run_command(DispatchCommand::ResetBindingSubmap)
        .expect("the default set");
    harness.state.pointer_button(MIDDLE, true, 0);
    key(&mut harness, KEY_LEFTALT, KeyState::Pressed);
    harness.state.pointer_button(MIDDLE, false, 0);
    key(&mut harness, KEY_LEFTALT, KeyState::Released);
    assert_eq!(harness.state.active_submap, "default");
}
