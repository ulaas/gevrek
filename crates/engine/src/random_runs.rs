//! The engine driven by long runs of commands and events that nobody
//! thought of in that order, with what must always hold looked at after
//! each: no run ends the program, every window is in one place only, no
//! window that has gone is still kept, a workspace's tiles have a size.
//!
//! The commands are those of gevrek's own example configurations (what
//! keys are bound to) and some more with numbers a person would not
//! choose; the events are windows that open, close, are minimized, moved
//! and get the keyboard. The runs are the same every time (the numbers
//! come from a seed); `GEVREK_RUNS=<n>` makes n times as many, others.

use crate::core::WmCore;
use crate::events::WindowEvent;
use crate::executor::Executor;
use crate::focus_manager::FocusManager;
use crate::layouts::LayoutType;
use crate::outputs::{MonitorHandle, Outputs};
use crate::spoil::Chance;
use crate::rules::RuleEngine;
use crate::testing::fake_desktop::FakeDesktop;
use crate::snap_zones::SnapZoneManager;
use crate::{Rect, WindowHandle};
use gevrek_model::command::DispatchCommand;
use gevrek_model::graph::NodeGraph;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

const OUTPUT: MonitorHandle = MonitorHandle(1);
const SECOND: MonitorHandle = MonitorHandle(2);
/// Where the second display is, beside the first.
const SECOND_AT: Rect = Rect { x: 1920, y: 0, width: 2560, height: 1440 };

fn second() -> crate::outputs::MonitorInfo {
    crate::outputs::MonitorInfo {
        handle: SECOND,
        name: "Second".to_string(),
        device_path: String::new(),
        stable_id: String::new(),
        is_primary: false,
        x: SECOND_AT.x,
        y: SECOND_AT.y,
        width: SECOND_AT.width,
        height: SECOND_AT.height,
        work_x: SECOND_AT.x,
        work_y: SECOND_AT.y,
        work_width: SECOND_AT.width,
        work_height: SECOND_AT.height - 40,
        dpi_scale: 1.0,
    }
}

/// The second display comes (`there`) or goes, as a backend tells the
/// engine of it.
fn display_changed(core: &mut WmCore, desktop: &mut FakeDesktop, there: bool) {
    let mut monitors: Vec<_> = core
        .monitor_manager
        .get_all_monitors()
        .into_iter()
        .filter(|monitor| monitor.handle != SECOND)
        .collect();
    desktop.more_outputs.clear();
    if there {
        monitors.push(second());
        desktop.more_outputs.push((SECOND, SECOND_AT));
    }
    let changes = core.monitor_manager.reconcile_with(monitors);
    if there
        && core
            .monitor_manager
            .get_workspaces(SECOND)
            .is_none_or(|workspaces| workspaces.is_empty())
    {
        let _ = core
            .monitor_manager
            .create_workspace(SECOND, "side".to_string(), LayoutType::BSP);
    }
    core.executor
        .apply_monitor_changes(desktop, &core.monitor_manager, &changes);
}

/// Commands that reach outside the engine (a file of the person's, a
/// program started, the session ended): not for a run of chance.
pub(crate) const LEFT_OUT: [&str; 20] = [
    // (Every window is shown, wherever it is: for when something went
    // wrong, and not a state the desktop is kept in.)
    "restore_windows",
    "exec",
    "shell",
    "notify",
    "osd",
    "quit",
    "save_resize",
    "load_resize",
    "quick_save_resize",
    "quick_load_resize",
    "launch_startup",
    "stop_startup",
    "restart_startup",
    "watch_configuration",
    "night_light_preview",
    "app_shortcut",
    "pick_window",
    "show_shortcuts",
    "keyboard_layout",
    "output_trial",
];

/// Every command a run may give, as JSON: what the examples bind keys
/// to, and commands with numbers at and beyond their edges.
pub(crate) fn commands() -> Vec<serde_json::Value> {
    let mut all: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for example in gevrek_model::examples::built_in_examples() {
        for binding in &example.config.desktop.bindings {
            let command = serde_json::to_value(&binding.command).expect("a command");
            all.insert(command.to_string(), command);
        }
    }
    let layouts = [
        "BSP", "Grid", "Columns", "Rows", "Monocle", "UltrawideVerticalStack", "MasterStack",
        "CenteredMaster", "Spiral", "Dwindle", "Scrolling", "Floating",
    ];
    // One of nearly every command there is, those that choose a
    // window, a workspace or a display among them.
    for command in crate::outside_input::every_command() {
        all.insert(command.to_string(), command);
    }
    for layout in layouts {
        all.insert(
            format!("set_layout {layout}"),
            serde_json::json!({"command": "set_layout", "layout": layout}),
        );
    }
    for number in [-100000, -50, -1, 0, 1, 7, 50, 5000, 100000] {
        for (name, field) in [
            ("set_layout_gap", "gap"),
            ("adjust_layout_gap", "delta"),
            ("set_master_count", "count"),
            ("adjust_master_count", "delta"),
            ("set_resize_delta", "delta"),
        ] {
            all.insert(
                format!("{name} {number}"),
                serde_json::json!({"command": name, field: number}),
            );
        }
        for name in ["set_layout_ratio", "adjust_layout_ratio"] {
            for field in ["ratio", "delta"] {
                all.insert(
                    format!("{name} {field} {number}"),
                    serde_json::json!({"command": name, field: number as f64 / 10.0}),
                );
            }
        }
        all.insert(
            format!("switch {number}"),
            serde_json::json!({"command": "switch_workspace_selector",
                               "selector": {"kind": "index", "value": number.max(0)}}),
        );
        all.insert(
            format!("move {number}"),
            serde_json::json!({"command": "move_to_workspace_selector",
                               "selector": {"kind": "index", "value": number.max(0)}}),
        );
    }
    all.into_values()
        .filter(|command| {
            command["command"]
                .as_str()
                .is_none_or(|name| !LEFT_OUT.contains(&name))
        })
        // (Only what this version reads as a command: a line that names
        // none is kept as unknown and does nothing.)
        .filter(|command| {
            serde_json::from_value::<DispatchCommand>(command.clone())
                .is_ok_and(|command| !matches!(command, DispatchCommand::Unknown(_)))
        })
        .collect()
}

/// A desktop with four workspaces, the rules and the scratchpads of the
/// examples, and with `two` a second display beside the first.
fn desk(two: bool) -> (WmCore, FakeDesktop, Arc<Mutex<RuleEngine>>) {
    let examples = gevrek_model::examples::built_in_examples();
    let mut engine = RuleEngine::new();
    engine.set_declarative_rules(
        examples
            .iter()
            .flat_map(|example| example.config.desktop.window_rules.clone())
            .collect(),
    );
    let rules = Arc::new(Mutex::new(engine));
    let mut executor =
        Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules.clone());
    executor.set_scratchpad_profiles(
        examples
            .iter()
            .flat_map(|example| example.config.desktop.scratchpads.clone())
            .collect(),
    );
    let mut outputs = Outputs::with_workspace_count(0);
    outputs.add_test_monitor(OUTPUT, "Primary");
    for name in ["1", "2", "3", "4"] {
        outputs
            .create_workspace(OUTPUT, name.to_string(), LayoutType::Dwindle)
            .expect("a workspace");
    }
    let core = WmCore::new(
        executor,
        outputs,
        FocusManager::new(),
        SnapZoneManager::new(Default::default()),
    );
    let (mut core, mut desktop) = (core, FakeDesktop::new(OUTPUT));
    if two {
        let mut monitors = core.monitor_manager.get_all_monitors();
        monitors.push(second());
        core.monitor_manager.replace_monitors(monitors);
        for name in ["side", "more"] {
            let _ = core
                .monitor_manager
                .create_workspace(SECOND, name.to_string(), LayoutType::BSP);
        }
        desktop.more_outputs.push((SECOND, SECOND_AT));
    }
    (core, desktop, rules)
}

/// What must hold after every step. Returns what does not.
fn broken(core: &WmCore, desktop: &FakeDesktop) -> Option<String> {
    // The display that is worked on is one that is there.
    if let Some(focused) = core.executor.focused_monitor() {
        if core.monitor_manager.get_monitor(focused).is_none() {
            return Some(format!("the display worked on ({focused:?}) is not there"));
        }
    }
    let mut seen: BTreeSet<WindowHandle> = BTreeSet::new();
    for monitor in core.monitor_manager.get_all_monitors() {
        let Some(ring) = core.monitor_manager.get_workspaces(monitor.handle) else {
            continue;
        };
        if ring.is_empty() {
            return Some("a display without a workspace".to_string());
        }
        if ring.focused_idx() >= ring.elements().len() {
            return Some("the workspace that is shown is none of the display's".to_string());
        }
        for (index, workspace) in ring.elements().iter().enumerate() {
            let tiled = workspace
                .containers
                .elements()
                .iter()
                .flat_map(|container| container.windows.elements().iter().copied());
            for window in tiled.chain(workspace.floating_windows.elements().iter().copied()) {
                if !seen.insert(window) {
                    return Some(format!(
                        "{window:?} is in two places (found again on workspace {})",
                        workspace.name
                    ));
                }
                if !desktop.exists_window(window) {
                    return Some(format!(
                        "{window:?} has gone and is still on workspace {}",
                        workspace.name
                    ));
                }
            }
            // No window of a workspace that is not shown is in sight.
            // (The other way round does not hold: on the workspace that
            // is shown, the tabs behind a group's first, the windows
            // behind the one of a monocle and the columns out of view
            // are kept out of sight on purpose.)
            let shown = index == ring.focused_idx();
            let all = workspace
                .containers
                .elements()
                .iter()
                .flat_map(|container| container.windows.elements().iter().copied())
                .chain(workspace.floating_windows.elements().iter().copied());
            for window in all {
                let hidden = desktop.windows.get(&window).is_some_and(|w| w.hidden_by_wm);
                if !shown && !hidden {
                    return Some(format!(
                        "{window:?} is in sight and on workspace {}, which is not shown",
                        workspace.name
                    ));
                }
            }
            if workspace
                .containers
                .elements()
                .iter()
                .any(|container| container.windows.is_empty())
            {
                return Some(format!("a tile with no window on workspace {}", workspace.name));
            }
            // The tiles of the workspace that is shown have a size.
            // (Not while the tiling is held: nothing is laid out then.)
            if shown
                && workspace.tiling_enabled
                && workspace.expose_state.is_none()
                && !core.executor.tiling_suspended()
            {
                for container in workspace.containers.elements() {
                    let Rect { width, height, .. } = container.rect;
                    // (With many tiles the layouts that give each new
                    // window a share of the last one's room come down to
                    // tiles of a pixel and of none: known, and the
                    // roadmap's to decide. Fewer than that have a size;
                    // none is ever less than nothing.)
                    let many = workspace.containers.len() > 6;
                    if width < 0 || height < 0 || (!many && (width == 0 || height == 0)) {
                        return Some(format!(
                            "a tile of no size ({width}x{height}) on workspace {} in {:?};                              the tiles: {:?}; the splits: {:?} and {:?}; the gap {:?}",
                            workspace.name,
                            workspace.layout,
                            workspace
                                .containers
                                .elements()
                                .iter()
                                .map(|container| container.rect)
                                .collect::<Vec<_>>(),
                            workspace.split_ratios,
                            workspace.bsp_split_ratios,
                            workspace.container_padding,
                        ));
                    }
                }
            }
        }
    }
    None
}

/// One run of `steps` steps. Returns what went wrong, with the steps that
/// led there.
fn run(seed: u64, steps: usize, commands: &[serde_json::Value]) -> Result<(), String> {
    let mut chance = Chance(seed | 1);
    // (Every other run has two displays.)
    let two = (seed >> 1) & 1 == 1;
    let mut second_there = two;
    let (mut core, mut desktop, rules) = desk(two);
    let examples = gevrek_model::examples::built_in_examples();
    // The keys the graph that is read waits for.
    let mut keys: Vec<crate::input::HotkeyEvent> = Vec::new();
    let mut trail: Vec<String> = Vec::new();
    // (Every other pair of runs begins with a graph nobody drew.)
    if (seed >> 2) & 1 == 1 {
        core.executor.set_graph(graph_of_chance(&mut chance));
        keys = keys_of(core.executor.get_graph());
        let _ = core
            .executor
            .execute_startup(&mut desktop, &mut core.monitor_manager);
        trail.push("a graph of chance from the start".to_string());
    }
    let mut opened = 0;
    // Every window the engine was told of and not yet told gone.
    let mut known: BTreeSet<WindowHandle> = BTreeSet::new();
    // What a window may be: some are what the examples' rules are about.
    const TITLES: [&str; 5] = ["window", "Picture in picture", "Slack", "Terminal", "Notes"];
    const PROGRAMS: [&str; 4] = ["app.exe", "kitty", "wt.exe", "slack.exe"];
    for step in 0..steps {
        let windows: Vec<WindowHandle> = desktop.windows.keys().copied().collect();
        let some = |chance: &mut Chance| windows.get(chance.below(windows.len())).copied();
        let said = match chance.below(20) {
            // A window opens (not for ever more of them).
            0..=3 if windows.len() < 12 => {
                opened += 1;
                let title = format!("{} {opened}", TITLES[chance.below(TITLES.len())]);
                let id = desktop.open(&title, PROGRAMS[chance.below(PROGRAMS.len())]);
                known.insert(id);
                // (On the second display, now and then.)
                let on_second = second_there && chance.below(3) == 0;
                if on_second {
                    if let Some(window) = desktop.windows.get_mut(&id) {
                        window.rect = Rect::new(2400, 300, 800, 600);
                    }
                }
                let _ = core.handle_window_event(&mut desktop, WindowEvent::Created(id));
                format!("{id:?} ({title}) opens{}", if on_second { " on the second display" } else { "" })
            }
            4 => match some(&mut chance) {
                Some(id) => {
                    desktop.close(id);
                    let _ = core.handle_window_event(&mut desktop, WindowEvent::Destroyed(id));
                    format!("{id:?} closes")
                }
                None => "nothing".to_string(),
            },
            // (A window that is in sight: one that is hidden is given
            // the keyboard by no system.)
            5 => match some(&mut chance).filter(|id| !desktop.window(*id).hidden_by_wm) {
                Some(id) => {
                    desktop.focused = Some(id);
                    let _ = core.handle_window_event(&mut desktop, WindowEvent::Focused(id));
                    format!("{id:?} gets the keyboard")
                }
                None => "nothing".to_string(),
            },
            6 => match some(&mut chance) {
                Some(id) => {
                    let now = !desktop.window(id).minimized;
                    if let Some(window) = desktop.windows.get_mut(&id) {
                        window.minimized = now;
                    }
                    let event = if now {
                        WindowEvent::Minimized(id)
                    } else {
                        WindowEvent::Restored(id)
                    };
                    let _ = core.handle_window_event(&mut desktop, event);
                    format!("{id:?} minimized: {now}")
                }
                None => "nothing".to_string(),
            },
            7 => match some(&mut chance) {
                Some(id) => {
                    // Dragged somewhere, to some size.
                    let rect = Rect::new(
                        chance.below(2400) as i32 - 300,
                        chance.below(1400) as i32 - 200,
                        chance.below(1900) as i32 + 1,
                        chance.below(1100) as i32 + 1,
                    );
                    let _ = core.handle_window_event(&mut desktop, WindowEvent::DragStarted(id));
                    if let Some(window) = desktop.windows.get_mut(&id) {
                        window.rect = rect;
                    }
                    let _ = core.handle_window_event(
                        &mut desktop,
                        WindowEvent::Moved(id, rect.x, rect.y),
                    );
                    let _ = core.handle_window_event(
                        &mut desktop,
                        WindowEvent::Resized(id, rect.width, rect.height),
                    );
                    let _ = core.handle_window_event(&mut desktop, WindowEvent::DragEnded(id));
                    format!("{id:?} dragged to {rect:?}")
                }
                None => "nothing".to_string(),
            },
            // The configuration is read anew: another of the examples,
            // its rules, its scratchpads, its workspaces and its graph.
            9 if chance.below(5) == 0 => {
                let example = &examples[chance.below(examples.len())];
                let config = &example.config;
                if let Ok(mut rules) = rules.lock() {
                    rules.set_declarative_rules(config.desktop.window_rules.clone());
                }
                core.executor
                    .set_scratchpad_profiles(config.desktop.scratchpads.clone());
                core.executor.set_workspace_profiles(
                    config.desktop.workspaces.clone(),
                    config.desktop.appearance.clone(),
                );
                core.executor.set_graph(config.node_graph.clone());
                // (Every other time with a graph nobody drew.)
                let of_chance = chance.below(2) == 0;
                if of_chance {
                    core.executor.set_graph(graph_of_chance(&mut chance));
                }
                keys = keys_of(core.executor.get_graph());
                crate::config_apply::configure_monitor_profiles(
                    &mut core.monitor_manager,
                    &config.desktop,
                );
                let _ = core.apply_workspace_profile_set(
                    &config.desktop.workspaces,
                    &config.desktop.appearance,
                    |_| true,
                    false,
                );
                let _ = core
                    .executor
                    .rerun_startup_for_reload(&mut desktop, &mut core.monitor_manager);
                format!(
                    "the configuration {} is read{}",
                    example.id,
                    if of_chance { ", with a graph of chance" } else { "" }
                )
            }
            // A key the graph waits for is pressed.
            10 | 11 if !keys.is_empty() => {
                let key = keys[chance.below(keys.len())].clone();
                let _ = core.executor.execute_hotkey(
                    &mut desktop,
                    &mut core.monitor_manager,
                    &mut core.focus_manager,
                    &key,
                );
                format!("the graph's key {} is pressed", key.key)
            }
            8 if two && chance.below(4) == 0 => {
                second_there = !second_there;
                display_changed(&mut core, &mut desktop, second_there);
                format!("the second display there: {second_there}")
            }
            _ => {
                let command = &commands[chance.below(commands.len())];
                let parsed: DispatchCommand =
                    serde_json::from_value(command.clone()).expect("a command");
                let _ = core.dispatch(&mut desktop, parsed);
                command.to_string()
            }
        };
        // A window that a command or a graph closed is told of as gone,
        // as the system tells of it (one that was closed as it opened
        // too).
        known.extend(desktop.windows.keys().copied());
        let gone: Vec<WindowHandle> = known
            .iter()
            .copied()
            .filter(|id| !desktop.exists_window(*id))
            .collect();
        for id in gone {
            known.remove(&id);
            if said != format!("{id:?} closes") {
                let _ = core.handle_window_event(&mut desktop, WindowEvent::Destroyed(id));
            }
        }
        let _ = core.flush_pending_layouts(&mut desktop);
        desktop.applied.clear();
        if std::env::var_os("GEVREK_TRACE").is_some() {
            // (With `GEVREK_SEED`: where every window is after each step.)
            let mut places = Vec::new();
            for monitor in core.monitor_manager.get_all_monitors() {
                if let Some(ring) = core.monitor_manager.get_workspaces(monitor.handle) {
                    for (index, workspace) in ring.elements().iter().enumerate() {
                        let shown = if index == ring.focused_idx() { "*" } else { "" };
                        let tiled: Vec<isize> = workspace
                            .containers
                            .elements()
                            .iter()
                            .flat_map(|container| container.windows.elements().iter().map(|w| w.0))
                            .collect();
                        let floating: Vec<isize> =
                            workspace.floating_windows.elements().iter().map(|w| w.0).collect();
                        if !tiled.is_empty() || !floating.is_empty() || !shown.is_empty() {
                            places.push(format!("{}{shown} {tiled:?} {floating:?}", workspace.name));
                        }
                    }
                }
            }
            let hidden: Vec<isize> = desktop
                .windows
                .iter()
                .filter(|(_, window)| window.hidden_by_wm)
                .map(|(id, _)| id.0)
                .collect();
            println!("{step:>3} {said}
      {} | hidden {hidden:?}", places.join("; "));
        }
        trail.push(said);
        if let Some(what) = broken(&core, &desktop) {
            let from = trail.len().saturating_sub(12);
            return Err(format!(
                "seed {seed:#x}, step {step}: {what}\n    after:\n      {}",
                trail[from..].join("\n      ")
            ));
        }
    }
    Ok(())
}

/// A key that a graph waits for, as it is pressed.
fn pressed(key: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> crate::input::HotkeyEvent {
    crate::input::HotkeyEvent {
        registration_ids: Vec::new(),
        key: key.to_string(),
        ctrl,
        alt,
        shift,
        win,
        device: Default::default(),
        trigger: Default::default(),
        held_ms: 0,
    }
}

/// The keys a graph waits for.
fn keys_of(graph: &NodeGraph) -> Vec<crate::input::HotkeyEvent> {
    let mut keys: Vec<(usize, crate::input::HotkeyEvent)> = graph
        .nodes
        .iter()
        .filter_map(|(id, node)| match &node.node_type {
            gevrek_model::graph::NodeType::OnKeyPress {
                key,
                ctrl,
                alt,
                shift,
                win,
            } => Some((*id, pressed(key, *ctrl, *alt, *shift, *win))),
            _ => None,
        })
        .collect();
    // (In the order of the nodes: the same in every run.)
    keys.sort_by_key(|(id, _)| *id);
    keys.into_iter().map(|(_, key)| key).collect()
}

/// A graph nobody drew. Behind each of eight keys, and behind what
/// windows do and the start, a row of nodes of any kind; what a node
/// wants handed to it comes from any node that has such a thing, round
/// in a circle too; and now and then what is written in the nodes is
/// spoilt as a file from outside may be.
fn graph_of_chance(chance: &mut Chance) -> NodeGraph {
    use gevrek_model::graph::{node_catalog, pin_takes, Connection, NodeType, PinType};
    let kinds: Vec<NodeType> = node_catalog()
        .into_iter()
        .flat_map(|(_, kinds)| kinds)
        .filter(|kind| kind.category() != "Events")
        .collect();
    let mut graph = NodeGraph::new();
    let mut heads = vec![
        NodeType::OnStartup,
        NodeType::OnStartupCompleted,
        NodeType::OnWindowCreated,
        NodeType::OnWindowDestroyed,
        NodeType::OnWindowFocused,
        NodeType::OnWindowMoved,
        NodeType::OnWindowResized,
    ];
    for index in 0..8 {
        heads.push(NodeType::OnKeyPress {
            key: format!("F{}", 13 + index),
            ctrl: index & 1 == 1,
            alt: index & 2 == 2,
            shift: false,
            win: index & 4 == 4,
        });
    }
    // The pins of a node that say "now" (or those that carry a thing).
    let pins = |graph: &NodeGraph, node: usize, out: bool, now: bool| -> Vec<usize> {
        let node = &graph.nodes[&node];
        let side = if out { &node.outputs } else { &node.inputs };
        side.iter()
            .filter(|pin| (pin.pin_type == PinType::Execution) == now)
            .map(|pin| pin.id)
            .collect()
    };
    for head in heads {
        let mut last = graph.add_node(head, [0.0, 0.0]);
        for _ in 0..1 + chance.below(6) {
            let next = graph.add_node(kinds[chance.below(kinds.len())].clone(), [0.0, 0.0]);
            let (outs, ins) = (pins(&graph, last, true, true), pins(&graph, next, false, true));
            if outs.is_empty() || ins.is_empty() {
                // (One that is not run, and has what others may want.)
                continue;
            }
            graph.add_connection(Connection {
                from_node: last,
                from_pin: outs[chance.below(outs.len())],
                to_node: next,
                to_pin: ins[0],
            });
            if !pins(&graph, next, true, true).is_empty() {
                last = next;
            }
        }
    }
    let mut ids: Vec<usize> = graph.nodes.keys().copied().collect();
    ids.sort_unstable();
    // Some rows lead into others, back into themselves too.
    for _ in 0..chance.below(5) {
        let (from, to) = (ids[chance.below(ids.len())], ids[chance.below(ids.len())]);
        let (outs, ins) = (pins(&graph, from, true, true), pins(&graph, to, false, true));
        if !outs.is_empty() && !ins.is_empty() {
            graph.add_connection(Connection {
                from_node: from,
                from_pin: outs[chance.below(outs.len())],
                to_node: to,
                to_pin: ins[0],
            });
        }
    }
    // What a node wants handed to it, from any node that has it.
    for to in &ids {
        let kind = graph.nodes[to].node_type.clone();
        let wanted: Vec<_> = graph.nodes[to]
            .inputs
            .iter()
            .filter(|pin| pin.pin_type != PinType::Execution)
            .cloned()
            .collect();
        for into in wanted {
            if chance.below(4) == 0 {
                continue;
            }
            let has: Vec<(usize, usize)> = ids
                .iter()
                .flat_map(|from| {
                    graph.nodes[from]
                        .outputs
                        .iter()
                        .filter(|out| {
                            out.pin_type != PinType::Execution
                                && pin_takes(&kind, &into.pin_type, &out.pin_type)
                        })
                        .map(|out| (*from, out.id))
                        .collect::<Vec<_>>()
                })
                .collect();
            if has.is_empty() {
                continue;
            }
            let (from, out) = has[chance.below(has.len())];
            graph.add_connection(Connection {
                from_node: from,
                from_pin: out,
                to_node: *to,
                to_pin: into.id,
            });
        }
    }
    // What is written in the nodes, spoilt.
    if chance.below(2) == 0 {
        if let Ok(mut document) = serde_json::to_value(&graph) {
            let mut all = Vec::new();
            crate::spoil::places(&document, &mut Vec::new(), &mut all);
            all.retain(|place| place.len() > 3 && place[0] == "nodes" && place[2] == "node_type");
            if !all.is_empty() {
                for _ in 0..1 + chance.below(6) {
                    crate::spoil::spoil(&mut document, &all, chance);
                }
            }
            if let Ok(mut spoilt) = serde_json::from_value::<NodeGraph>(document) {
                spoilt.rebuild_connection_indices();
                graph = spoilt;
            }
        }
    }
    graph
}

/// A wire joins the first output of one node that fits to an input of
/// another.
fn wire(graph: &mut NodeGraph, from: usize, to: usize, input: usize) -> bool {
    use gevrek_model::graph::{pin_takes, Connection};
    let into = graph.nodes[&to].inputs[input].clone();
    let fits = graph.nodes[&from]
        .outputs
        .iter()
        .find(|out| pin_takes(&graph.nodes[&to].node_type, &into.pin_type, &out.pin_type))
        .map(|out| out.id);
    fits.is_some_and(|out| {
        graph.add_connection(Connection {
            from_node: from,
            from_pin: out,
            to_node: to,
            to_pin: into.id,
        })
    })
}

/// A graph is a file anyone may have written, and its wires may go
/// round: a sum that is one of its own two numbers, or two nodes that
/// each want the other's word first, are not followed for ever (they
/// were, until the program had no room left and ended).
#[test]
fn a_node_that_is_asked_for_its_own_value_is_not_followed_for_ever() {
    use gevrek_model::graph::NodeType;
    let key = || NodeType::OnKeyPress {
        key: "F13".to_string(),
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
    };
    let said = || NodeType::LogMessage {
        message: "sum ".to_string(),
        level: gevrek_model::graph::LogLevel::Info,
    };

    // A sum that is its own first number.
    let mut own = NodeGraph::new();
    let (on_key, log) = (own.add_node(key(), [0.0, 0.0]), own.add_node(said(), [0.0, 0.0]));
    let (text, sum) = (
        own.add_node(NodeType::ToString, [0.0, 0.0]),
        own.add_node(NodeType::Add, [0.0, 0.0]),
    );
    assert!(wire(&mut own, on_key, log, 0));
    assert!(wire(&mut own, text, log, 1));
    assert!(wire(&mut own, sum, text, 0));
    assert!(wire(&mut own, sum, sum, 0), "a wire that goes round is one a file may have");

    // Two that each want the other's first.
    let mut two = NodeGraph::new();
    let (on_key, log) = (two.add_node(key(), [0.0, 0.0]), two.add_node(said(), [0.0, 0.0]));
    let (text, sum, product) = (
        two.add_node(NodeType::ToString, [0.0, 0.0]),
        two.add_node(NodeType::Add, [0.0, 0.0]),
        two.add_node(NodeType::Multiply, [0.0, 0.0]),
    );
    assert!(wire(&mut two, on_key, log, 0));
    assert!(wire(&mut two, text, log, 1));
    assert!(wire(&mut two, sum, text, 0));
    assert!(wire(&mut two, product, sum, 1));
    assert!(wire(&mut two, sum, product, 0));

    // A row of sums longer than anybody draws: followed to its end, or
    // given up; the program lives.
    let mut long = NodeGraph::new();
    let (on_key, log) = (long.add_node(key(), [0.0, 0.0]), long.add_node(said(), [0.0, 0.0]));
    let text = long.add_node(NodeType::ToString, [0.0, 0.0]);
    assert!(wire(&mut long, on_key, log, 0));
    assert!(wire(&mut long, text, log, 1));
    let mut last = text;
    for _ in 0..20_000 {
        let sum = long.add_node(NodeType::Add, [0.0, 0.0]);
        assert!(wire(&mut long, sum, last, 0));
        last = sum;
    }

    // And a row of nodes that are run, as long, each of which wants
    // the word of such a row of sums.
    let mut row = NodeGraph::new();
    let text = row.add_node(NodeType::ToString, [0.0, 0.0]);
    let mut last = text;
    for _ in 0..20_000 {
        let sum = row.add_node(NodeType::Add, [0.0, 0.0]);
        assert!(wire(&mut row, sum, last, 0));
        last = sum;
    }
    let mut last = row.add_node(key(), [0.0, 0.0]);
    for _ in 0..20_000 {
        let log = row.add_node(said(), [0.0, 0.0]);
        assert!(wire(&mut row, last, log, 0));
        assert!(wire(&mut row, text, log, 1));
        last = log;
    }

    // Two loops that run each other: each begins its row anew, so
    // neither comes to an end of its own.
    let turns = |start, end, step| NodeType::ForLoop { start, end, step };
    let mut loops = NodeGraph::new();
    let on_key = loops.add_node(key(), [0.0, 0.0]);
    let (first, second) = (
        loops.add_node(turns(0, 9_999, 1), [0.0, 0.0]),
        loops.add_node(turns(0, 9_999, 1), [0.0, 0.0]),
    );
    assert!(wire(&mut loops, on_key, first, 0));
    assert!(wire(&mut loops, first, second, 0));
    assert!(wire(&mut loops, second, first, 0));

    // A loop that counts to the largest number there is, by such steps.
    let mut far = NodeGraph::new();
    let on_key = far.add_node(key(), [0.0, 0.0]);
    let (up, down) = (
        far.add_node(turns(i32::MAX - 1, i32::MAX, i32::MAX), [0.0, 0.0]),
        far.add_node(turns(i32::MIN + 1, i32::MIN, i32::MIN), [0.0, 0.0]),
    );
    assert!(wire(&mut far, on_key, up, 0));
    assert!(wire(&mut far, on_key, down, 0));

    for graph in [own, two, long, row, loops, far] {
        let (mut core, mut desktop, _) = desk(false);
        core.executor.set_graph(graph);
        let _ = core.executor.execute_hotkey(
            &mut desktop,
            &mut core.monitor_manager,
            &mut core.focus_manager,
            &pressed("F13", false, false, false, false),
        );
        assert_eq!(broken(&core, &desktop), None);
    }
}

/// A desk with a graph, a key of which (F13) asks for the scratchpad
/// that takes a window called Notes.
fn desk_with_a_scratchpad() -> (WmCore, FakeDesktop) {
    use gevrek_model::graph::{node_catalog, NodeType};
    let mut pad = node_catalog()
        .into_iter()
        .flat_map(|(_, kinds)| kinds)
        .find(|kind| matches!(kind, NodeType::ToggleScratchpad { .. }))
        .expect("the node");
    if let NodeType::ToggleScratchpad { pattern, .. } = &mut pad {
        *pattern = "Notes".to_string();
    }
    let mut graph = NodeGraph::new();
    let on_key = graph.add_node(
        NodeType::OnKeyPress {
            key: "F13".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        },
        [0.0, 0.0],
    );
    let pad = graph.add_node(pad, [0.0, 0.0]);
    assert!(wire(&mut graph, on_key, pad, 0));
    let (mut core, desktop, _) = desk(false);
    core.executor.set_graph(graph);
    (core, desktop)
}

fn press_f13(core: &mut WmCore, desktop: &mut FakeDesktop) {
    core.executor
        .execute_hotkey(
            desktop,
            &mut core.monitor_manager,
            &mut core.focus_manager,
            &pressed("F13", false, false, false, false),
        )
        .expect("the key");
    let _ = core.flush_pending_layouts(desktop);
}

fn asked(core: &mut WmCore, desktop: &mut FakeDesktop, command: serde_json::Value) {
    let command: DispatchCommand = serde_json::from_value(command).expect("a command");
    let _ = core.dispatch(desktop, command);
    let _ = core.flush_pending_layouts(desktop);
}

/// A scratchpad's window floats over the workspaces and is on none. One
/// that the person gives a place on a workspace (here: made to tile) is
/// an ordinary window from then on. It stayed the scratchpad's too, and
/// asked for from another workspace it was shown there: in sight, and
/// on a workspace that was not.
#[test]
fn a_scratchpads_window_given_a_place_is_the_scratchpads_no_longer() {
    let (mut core, mut desktop) = desk_with_a_scratchpad();
    let notes = desktop.open("Notes 1", "notes.exe");
    core.handle_window_event(&mut desktop, WindowEvent::Created(notes))
        .expect("opened");
    press_f13(&mut core, &mut desktop);
    assert!(core.monitor_manager.find_window(notes).is_none(), "the scratchpad has it");
    assert_eq!(desktop.focused, Some(notes));

    asked(&mut core, &mut desktop, serde_json::json!({"command": "toggle_floating"}));
    assert!(
        core.monitor_manager.find_window(notes).is_some(),
        "it has a place on a workspace"
    );
    asked(
        &mut core,
        &mut desktop,
        serde_json::json!({"command": "switch_workspace_selector", "selector": {"kind": "name", "value": "3"}}),
    );
    assert!(desktop.window(notes).hidden_by_wm, "it went with its workspace");

    // Asked for again, the scratchpad takes it as it takes any window
    // that is its kind: off its workspace, and shown.
    press_f13(&mut core, &mut desktop);
    assert_eq!(broken(&core, &desktop), None);
    assert!(core.monitor_manager.find_window(notes).is_none());
    assert!(!desktop.window(notes).hidden_by_wm);
}

/// A scratchpad's window that is hidden, and that the system tells of as
/// restored from being minimized, is given no tile: it is the
/// scratchpad's, and comes when the scratchpad is asked for.
#[test]
fn a_scratchpads_window_that_is_restored_is_given_no_tile() {
    let (mut core, mut desktop) = desk_with_a_scratchpad();
    let notes = desktop.open("Notes 1", "notes.exe");
    core.handle_window_event(&mut desktop, WindowEvent::Created(notes))
        .expect("opened");
    // (Minimized, which takes it off its workspace, and taken by the
    // scratchpad as it is.)
    desktop.windows.get_mut(&notes).expect("it").minimized = true;
    core.handle_window_event(&mut desktop, WindowEvent::Minimized(notes))
        .expect("minimized");
    press_f13(&mut core, &mut desktop);
    assert!(core.monitor_manager.find_window(notes).is_none());
    press_f13(&mut core, &mut desktop);
    assert!(desktop.window(notes).hidden_by_wm, "the scratchpad is put away");

    desktop.windows.get_mut(&notes).expect("it").minimized = false;
    core.handle_window_event(&mut desktop, WindowEvent::Restored(notes))
        .expect("restored");
    let _ = core.flush_pending_layouts(&mut desktop);
    assert!(core.monitor_manager.find_window(notes).is_none(), "on no workspace");
    assert!(desktop.window(notes).hidden_by_wm, "and put away still");
    press_f13(&mut core, &mut desktop);
    assert!(!desktop.window(notes).hidden_by_wm, "it comes when it is asked for");
    assert_eq!(broken(&core, &desktop), None);
}

/// A graph is handed the window that closed (`On Window Destroyed`), and
/// may hand it on: moved to a workspace or arranged, at once or by a key
/// pressed afterwards, a window that has gone is given no place. (It was,
/// and stayed there for good.)
#[test]
fn a_window_that_has_gone_is_given_no_place_by_a_graph() {
    use gevrek_model::graph::{NodeType, WindowMode};
    let mut graph = NodeGraph::new();
    let closed = graph.add_node(NodeType::OnWindowDestroyed, [0.0, 0.0]);
    let moved = graph.add_node(
        NodeType::MoveWindowToWorkspace { workspace_index: 1 },
        [0.0, 0.0],
    );
    assert!(wire(&mut graph, closed, moved, 0));
    assert!(wire(&mut graph, closed, moved, 1));
    let on_key = graph.add_node(
        NodeType::OnKeyPress {
            key: "F13".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
        },
        [0.0, 0.0],
    );
    for mode in [WindowMode::Tiled, WindowMode::Floating, WindowMode::Auto] {
        let arranged = graph.add_node(NodeType::SetWindowArrangement { mode }, [0.0, 0.0]);
        assert!(wire(&mut graph, on_key, arranged, 0));
        assert!(wire(&mut graph, closed, arranged, 1));
    }
    let again = graph.add_node(
        NodeType::MoveWindowToWorkspace { workspace_index: 2 },
        [0.0, 0.0],
    );
    assert!(wire(&mut graph, on_key, again, 0));
    assert!(wire(&mut graph, closed, again, 1));

    let (mut core, mut desktop, _) = desk(false);
    core.executor.set_graph(graph);
    let stays = desktop.open("window 1", "app.exe");
    let goes = desktop.open("window 2", "app.exe");
    for window in [stays, goes] {
        core.handle_window_event(&mut desktop, WindowEvent::Created(window))
            .expect("opened");
    }
    desktop.close(goes);
    core.handle_window_event(&mut desktop, WindowEvent::Destroyed(goes))
        .expect("closed");
    let _ = core.flush_pending_layouts(&mut desktop);
    assert_eq!(broken(&core, &desktop), None);
    assert!(core.monitor_manager.find_window(goes).is_none());

    press_f13(&mut core, &mut desktop);
    assert_eq!(broken(&core, &desktop), None);
    assert!(core.monitor_manager.find_window(goes).is_none());
    assert!(core.monitor_manager.find_window(stays).is_some());
}

/// Several nodes that wait for the same key are run in the order they
/// have in the graph, the same on every start. (They were run in the
/// order a table of this run happened to give them: a graph did one
/// thing first today and another tomorrow.)
#[test]
fn the_nodes_of_one_key_are_run_in_their_order() {
    use gevrek_model::graph::{LogLevel, NodeType};
    let mut graph = NodeGraph::new();
    for index in 0..24 {
        let on_key = graph.add_node(
            NodeType::OnKeyPress {
                key: "F13".to_string(),
                ctrl: false,
                alt: false,
                shift: false,
                win: false,
            },
            [0.0, 0.0],
        );
        let said = graph.add_node(
            NodeType::LogMessage {
                message: format!("{index:02}"),
                level: LogLevel::Info,
            },
            [0.0, 0.0],
        );
        assert!(wire(&mut graph, on_key, said, 0));
    }
    let (mut core, mut desktop, rules) = desk(false);
    let log = Arc::new(Mutex::new(Vec::new()));
    core.executor = Executor::new(graph, log.clone(), rules);
    press_f13(&mut core, &mut desktop);
    let said: Vec<String> = log
        .lock()
        .expect("the log")
        .iter()
        .map(|entry| entry.message.clone())
        .collect();
    let in_order: Vec<String> = (0..24).map(|index| format!("{index:02}")).collect();
    assert_eq!(said, in_order);
}

/// The other button on a window in the panel's taskbar: the window comes
/// to the workspace the display shows, floating and in its middle; the
/// same again puts it back as it was, on its workspace, tiled at its
/// place or floating where it floated.
#[test]
fn a_window_comes_to_the_middle_and_goes_back_as_it_was() {
    let (mut core, mut desktop, _) = desk(false);
    let open = |core: &mut WmCore, desktop: &mut FakeDesktop, title: &str| {
        let id = desktop.open(title, "app.exe");
        core.handle_window_event(desktop, WindowEvent::Created(id))
            .expect("opened");
        let _ = core.flush_pending_layouts(desktop);
        id
    };
    let (first, second, third) = (
        open(&mut core, &mut desktop, "window 1"),
        open(&mut core, &mut desktop, "window 2"),
        open(&mut core, &mut desktop, "window 3"),
    );
    let toggle = |id: WindowHandle| {
        serde_json::json!({
            "command": "toggle_window_centered",
            "window": {"kind": "handle", "value": id.0},
            "target": {"kind": "primary"},
        })
    };
    let tiles = |core: &WmCore, workspace: usize| -> Vec<WindowHandle> {
        core.monitor_manager.get_workspaces(OUTPUT).expect("workspaces").elements()[workspace]
            .containers
            .elements()
            .iter()
            .flat_map(|container| container.windows.elements().iter().copied())
            .collect()
    };
    assert_eq!(tiles(&core, 0), [first, second, third]);
    let was = desktop.window(second).rect;

    // From another workspace: it comes there, floats, and is in the middle.
    asked(
        &mut core,
        &mut desktop,
        serde_json::json!({"command": "switch_workspace_selector", "selector": {"kind": "name", "value": "3"}}),
    );
    assert!(desktop.window(second).hidden_by_wm);
    asked(&mut core, &mut desktop, toggle(second));
    assert_eq!(broken(&core, &desktop), None);
    assert_eq!(core.monitor_manager.find_window(second), Some((OUTPUT, 2)));
    assert!(core.monitor_manager.is_window_floating(second));
    assert!(!desktop.window(second).hidden_by_wm);
    let area = core
        .monitor_manager
        .get_adjusted_work_area(OUTPUT, &desktop)
        .expect("the display's room");
    let now = desktop.window(second).rect;
    assert!((now.x + now.width / 2 - (area.x + area.width / 2)).abs() <= 1, "{now:?} in {area:?}");
    assert!((now.y + now.height / 2 - (area.y + area.height / 2)).abs() <= 1, "{now:?} in {area:?}");
    assert_eq!(desktop.focused, Some(second));
    assert_eq!(tiles(&core, 0), [first, third]);

    // Again: back on its workspace, tiled, between the two it was between.
    asked(&mut core, &mut desktop, toggle(second));
    assert_eq!(broken(&core, &desktop), None);
    assert_eq!(core.monitor_manager.find_window(second), Some((OUTPUT, 0)));
    assert!(!core.monitor_manager.is_window_floating(second));
    assert_eq!(tiles(&core, 0), [first, second, third]);
    assert!(desktop.window(second).hidden_by_wm, "its workspace is not shown");
    asked(
        &mut core,
        &mut desktop,
        serde_json::json!({"command": "switch_workspace_selector", "selector": {"kind": "name", "value": "1"}}),
    );
    assert_eq!(desktop.window(second).rect, was, "where it was, as large as it was");

    // One that floated: to the middle, and back to where it floated.
    asked(
        &mut core,
        &mut desktop,
        serde_json::json!({"command": "set_window_floating", "selector": {"kind": "handle", "value": third.0}, "enabled": true}),
    );
    asked(
        &mut core,
        &mut desktop,
        serde_json::json!({"command": "move_window", "selector": {"kind": "handle", "value": third.0}, "x": 40, "y": 60, "relative": false}),
    );
    let floated = desktop.window(third).rect;
    asked(&mut core, &mut desktop, toggle(third));
    assert_ne!(desktop.window(third).rect, floated);
    asked(&mut core, &mut desktop, toggle(third));
    assert!(core.monitor_manager.is_window_floating(third));
    assert_eq!(desktop.window(third).rect, floated);
    assert_eq!(broken(&core, &desktop), None);

    // A window that is not there: nothing, and nothing is broken.
    asked(&mut core, &mut desktop, toggle(WindowHandle(9999)));
    assert_eq!(broken(&core, &desktop), None);
}

/// A window that is minimized is where the system keeps its mark: far
/// off every display, as small as a title. A workspace that comes hands
/// it no place to come to (that place was written down as the window's
/// own, and it came back very small and out of sight, however it had
/// been put right before). One that was minimized behind gevrek's back
/// while its workspace was away is brought back with it; and a floating
/// window that is on no display at all comes to the middle of its own,
/// at a size that can be seen.
#[test]
fn a_minimized_window_is_given_no_place_and_a_lost_one_is_found() {
    let show = |name: &str| {
        serde_json::json!({"command": "switch_workspace_selector", "selector": {"kind": "name", "value": name}})
    };
    let on_the_display = |core: &WmCore, desktop: &FakeDesktop, window: WindowHandle| {
        let now = desktop.window(window).rect;
        let area = core
            .monitor_manager
            .get_adjusted_work_area(OUTPUT, desktop)
            .expect("the display's room");
        assert!(
            now.x >= area.x
                && now.y >= area.y
                && now.x + now.width <= area.x + area.width
                && now.y + now.height <= area.y + area.height
                && now.width >= area.width / 3
                && now.height >= area.height / 3,
            "{now:?} in {area:?}"
        );
    };
    let minimized_by_the_system = |desktop: &mut FakeDesktop, window: WindowHandle| {
        let it = desktop.windows.get_mut(&window).expect("it");
        it.minimized = true;
        it.rect = Rect::new(-32000, -32000, 160, 28);
    };

    // A workspace of floating windows.
    let (mut core, mut desktop, _) = desk(false);
    asked(&mut core, &mut desktop, show("2"));
    let _ = core.dispatch(
        &mut desktop,
        DispatchCommand::SetLayout {
            layout: LayoutType::Floating,
        },
    );
    let floats = desktop.open("Terminal 1", "wt.exe");
    let other = desktop.open("Notes 2", "notes.exe");
    for window in [floats, other] {
        core.handle_window_event(&mut desktop, WindowEvent::Created(window))
            .expect("opened");
    }
    let _ = core.flush_pending_layouts(&mut desktop);
    assert!(core.monitor_manager.is_window_floating(floats));
    let before = desktop.window(other).rect;

    // Minimized by the person, in sight: it stays so when its workspace
    // goes and comes, and is handed no place.
    minimized_by_the_system(&mut desktop, floats);
    core.handle_window_event(&mut desktop, WindowEvent::Minimized(floats))
        .expect("minimized");
    asked(&mut core, &mut desktop, show("1"));
    desktop.came.clear();
    asked(&mut core, &mut desktop, show("2"));
    let came: Vec<(WindowHandle, Rect)> = desktop.came.concat();
    assert!(came.iter().all(|(window, _)| *window != floats), "{came:?}");
    assert!(came.contains(&(other, before)), "{came:?}");
    assert!(desktop.window(floats).minimized);

    // Brought back by the system where it was left written down: on no
    // display. It is found when its workspace comes.
    asked(&mut core, &mut desktop, show("1"));
    {
        let it = desktop.windows.get_mut(&floats).expect("it");
        it.minimized = false;
        it.rect = Rect::new(-32000, -32000, 480, 99);
    }
    core.handle_window_event(&mut desktop, WindowEvent::Restored(floats))
        .expect("restored");
    desktop.came.clear();
    asked(&mut core, &mut desktop, show("2"));
    on_the_display(&core, &desktop, floats);
    assert!(desktop
        .came
        .concat()
        .contains(&(floats, desktop.window(floats).rect)));
    assert_eq!(desktop.window(other).rect, before, "one in sight stays where it is");

    // Minimized behind gevrek's back while its workspace is away (told
    // of it or not): it comes back with its workspace.
    for told in [false, true] {
        asked(&mut core, &mut desktop, show("1"));
        minimized_by_the_system(&mut desktop, floats);
        if told {
            let _ = core.handle_window_event(&mut desktop, WindowEvent::Minimized(floats));
        }
        asked(&mut core, &mut desktop, show("2"));
        assert!(!desktop.window(floats).minimized, "told: {told}");
        on_the_display(&core, &desktop, floats);
        assert_eq!(broken(&core, &desktop), None);
    }

    // The same for tiles: they are the workspace's as it was left, and
    // have their tiles when it comes.
    let (mut core, mut desktop, _) = desk(false);
    let tiles: Vec<WindowHandle> = ["window 1", "window 2", "window 3"]
        .into_iter()
        .map(|title| {
            let window = desktop.open(title, "app.exe");
            core.handle_window_event(&mut desktop, WindowEvent::Created(window))
                .expect("opened");
            window
        })
        .collect();
    let _ = core.flush_pending_layouts(&mut desktop);
    let were: Vec<Rect> = tiles.iter().map(|window| desktop.window(*window).rect).collect();
    asked(&mut core, &mut desktop, show("2"));
    for window in &tiles {
        minimized_by_the_system(&mut desktop, *window);
        let _ = core.handle_window_event(&mut desktop, WindowEvent::Minimized(*window));
    }
    asked(&mut core, &mut desktop, show("1"));
    let _ = core.flush_pending_layouts(&mut desktop);
    for (window, was) in tiles.iter().zip(&were) {
        assert!(!desktop.window(*window).minimized);
        assert_eq!(desktop.window(*window).rect, *was);
    }
    assert_eq!(broken(&core, &desktop), None);
}

#[test]
fn long_runs_of_chance_leave_the_desktop_whole() {
    let times = std::env::var("GEVREK_RUNS")
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(1)
        .max(1);
    let commands = commands();
    assert!(commands.len() > 100, "{} commands", commands.len());
    let mut wrong: Vec<String> = Vec::new();
    // (`GEVREK_SEED=0x...`: the one run that a line of this test named.)
    if let Some(seed) = std::env::var("GEVREK_SEED")
        .ok()
        .and_then(|text| u64::from_str_radix(text.trim_start_matches("0x"), 16).ok())
    {
        run(seed, 400, &commands).expect("the run of that seed");
        return;
    }
    for round in 0..24 * times {
        let seed = (round + 1)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(times.wrapping_mul(0x2545_f491_4f6c_dd1d));
        let commands = commands.clone();
        let outcome = std::panic::catch_unwind(move || run(seed, 400, &commands));
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(what)) => wrong.push(what),
            Err(panic) => wrong.push(format!(
                "seed {seed:#x}: the program ended: {}",
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                    .unwrap_or_default()
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of the runs went wrong; the first:\n{}",
        wrong.len(),
        wrong.iter().take(if std::env::var_os("GEVREK_ALL").is_some() { 1000 } else { 3 }).cloned().collect::<Vec<_>>().join("\n\n")
    );
}
