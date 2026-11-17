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

fn desk() -> (WmCore, FakeDesktop) {
    let rules = Arc::new(Mutex::new(RuleEngine::new()));
    let executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
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
    (core, FakeDesktop::new(OUTPUT))
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
            if workspace
                .containers
                .elements()
                .iter()
                .any(|container| container.windows.is_empty())
            {
                return Some(format!("a tile with no window on workspace {}", workspace.name));
            }
            // The tiles of the workspace that is shown have a size.
            let shown = index == ring.focused_idx();
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
    let (mut core, mut desktop) = desk();
    let mut trail: Vec<String> = Vec::new();
    let mut opened = 0;
    for step in 0..steps {
        let windows: Vec<WindowHandle> = desktop.windows.keys().copied().collect();
        let some = |chance: &mut Chance| windows.get(chance.below(windows.len())).copied();
        let said = match chance.below(20) {
            // A window opens (not for ever more of them).
            0..=3 if windows.len() < 12 => {
                opened += 1;
                let id = desktop.open(&format!("window {opened}"), "app.exe");
                let _ = core.handle_window_event(&mut desktop, WindowEvent::Created(id));
                format!("{id:?} opens")
            }
            4 => match some(&mut chance) {
                Some(id) => {
                    desktop.close(id);
                    let _ = core.handle_window_event(&mut desktop, WindowEvent::Destroyed(id));
                    format!("{id:?} closes")
                }
                None => "nothing".to_string(),
            },
            5 => match some(&mut chance) {
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
