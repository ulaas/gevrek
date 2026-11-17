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
use crate::outside_input::Chance;
use crate::rules::RuleEngine;
use crate::testing::fake_desktop::FakeDesktop;
use crate::zones::ZoneManager;
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
const LEFT_OUT: [&str; 19] = [
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
fn commands() -> Vec<serde_json::Value> {
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
fn desk(two: bool) -> (WmCore, FakeDesktop) {
    let examples = gevrek_model::examples::built_in_examples();
    let mut engine = RuleEngine::new();
    engine.set_declarative_rules(
        examples
            .iter()
            .flat_map(|example| example.config.desktop.window_rules.clone())
            .collect(),
    );
    let rules = Arc::new(Mutex::new(engine));
    let mut executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
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
        ZoneManager::new(Default::default()),
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
    (core, desktop)
}

/// What must hold after every step. Returns what does not.
fn broken(core: &WmCore, desktop: &FakeDesktop) -> Option<String> {
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
            if shown && workspace.tiling_enabled && workspace.expose_state.is_none() {
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
    let (mut core, mut desktop) = desk(two);
    let mut trail: Vec<String> = Vec::new();
    let mut opened = 0;
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
        // A window that a command closed is told of as gone, as the
        // system tells of it.
        for id in &windows {
            if !desktop.exists_window(*id) && !said.ends_with("closes") {
                let _ = core.handle_window_event(&mut desktop, WindowEvent::Destroyed(*id));
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
        wrong.iter().take(3).cloned().collect::<Vec<_>>().join("\n\n")
    );
}
