//! What the engine does most, timed: a layout, a window opened, a change
//! of workspace, the state as it is published, the rules, the overview's
//! arrangement, a reload's plan, a configuration read and written.
//!
//!     cargo bench -p gevrek-engine --features test-support
//!
//! Each line is one case with the time one run of it takes: the median
//! of thirty samples and the quickest of them. A name after `--` runs
//! only the cases that contain it (`-- layout/`). With
//! `GEVREK_BENCH_OUT=<file>` the medians are written there too, a name
//! and its nanoseconds on each line, for two runs to be held against
//! each other.
//!
//! No crate is used for it: the numbers are for this tree against
//! itself on one machine, before a change and after it.

use gevrek_engine::container::Container;
use gevrek_engine::core::WmCore;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::executor::Executor;
use gevrek_engine::focus_manager::FocusManager;
use gevrek_engine::layouts::{apply_layout, LayoutType};
use gevrek_engine::outputs::{MonitorHandle, Outputs};
use gevrek_engine::reload::ConfigDiff;
use gevrek_engine::rules::{RuleEngine, WindowInfo};
use gevrek_engine::snap_zones::SnapZoneManager;
use gevrek_engine::testing::fake_desktop::FakeDesktop;
use gevrek_engine::{Padding, Rect, WindowId};
use gevrek_model::command::{DispatchCommand, WorkspaceSelector};
use gevrek_model::config::Config;
use gevrek_model::graph::NodeGraph;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

#[path = "support/harness.rs"]
mod harness;
use harness::Bench;

const OUTPUT: MonitorHandle = MonitorHandle(1);
/// A core with one 1920x1080 output and the workspaces `names`, and a
/// fake desktop for it (as the scenarios of the engine's tests have it).
fn desk(names: &[&str], layout: LayoutType) -> (WmCore, FakeDesktop) {
    let rules = Arc::new(Mutex::new(RuleEngine::new()));
    let executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
    let mut outputs = Outputs::with_workspace_count(0);
    outputs.add_test_monitor(OUTPUT, "Primary");
    for name in names {
        outputs
            .create_workspace(OUTPUT, name.to_string(), layout)
            .expect("a workspace");
    }
    let core = WmCore::new(
        executor,
        outputs,
        FocusManager::new(),
        SnapZoneManager::new(Default::default()),
    );
    (core, FakeDesktop::new(OUTPUT))
}

fn open(core: &mut WmCore, desktop: &mut FakeDesktop, count: usize) {
    for index in 0..count {
        let id = desktop.open(&format!("window {index}"), "app.exe");
        core.handle_window_event(desktop, WindowEvent::Created(id))
            .expect("created");
        core.flush_pending_layouts(desktop).expect("a layout");
    }
}

fn switch(core: &mut WmCore, desktop: &mut FakeDesktop, to: usize) {
    core.dispatch(
        desktop,
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(to),
        },
    )
    .expect("switched");
    core.flush_pending_layouts(desktop).expect("a layout");
    // (The fake desktop keeps every effect it was given: not for ever.)
    desktop.applied.clear();
}

fn window(index: usize) -> WindowInfo {
    WindowInfo {
        handle: WindowId(index as _),
        title: format!("Document {index} - An editor of some kind"),
        class: format!("WindowClass{}", index % 12),
        exe: format!("program{}.exe", index % 12),
        path: format!(
            "C:\\Program Files\\Program {}\\program{}.exe",
            index % 12,
            index % 12
        ),
        app_id: format!("program{}", index % 12),
        has_parent: false,
        is_visible: true,
        is_tool_window: false,
        is_noactivate: false,
        has_owner: false,
        is_cloaked: false,
        is_layered: false,
        tags: Default::default(),
        workspace: Some("1".to_string()),
        workspace_index: Some(0),
    }
}

/// Twenty rules as a person writes them: by class, by title, by program.
fn rules() -> RuleEngine {
    let rules: Vec<serde_json::Value> = (0..20)
        .map(|index| {
            let (kind, strategy, value) = match index % 4 {
                0 => ("Class", "Equals", format!("WindowClass{index}")),
                1 => ("Title", "Contains", format!("Document {index} ")),
                2 => ("Exe", "Equals", format!("program{index}.exe")),
                _ => ("Title", "Regex", format!("^Document {index}\\d* - ")),
            };
            serde_json::json!({
                "name": format!("Rule {index}"),
                "enabled": true,
                "predicates": [{"kind": kind, "strategy": strategy, "value": value,
                                "negate": false, "case_sensitive": false}],
                "effects": ["Float"],
            })
        })
        .collect();
    let mut engine = RuleEngine::new();
    engine.set_declarative_rules(
        serde_json::from_value(serde_json::Value::Array(rules))
            .expect("rules as the model reads them"),
    );
    engine
}

fn main() {
    let mut bench = Bench::from_args();

    // A layout of 1, 10 and 50 windows, in each layout.
    let area = Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1400,
    };
    let padding = Padding::default();
    for (name, layout) in [
        ("bsp", LayoutType::BSP),
        ("dwindle", LayoutType::Dwindle),
        ("spiral", LayoutType::Spiral),
        ("grid", LayoutType::Grid),
        ("columns", LayoutType::Columns),
        ("rows", LayoutType::Rows),
        ("master_stack", LayoutType::MasterStack),
        ("centered_master", LayoutType::CenteredMaster),
        ("monocle", LayoutType::Monocle),
    ] {
        for count in [1usize, 10, 50] {
            let mut containers: Vec<Container> = (0..count)
                .map(|index| {
                    let mut container = Container::new(index);
                    container.add_window(WindowId(index as _));
                    container
                })
                .collect();
            bench.case(&format!("layout/{name}/{count}"), || {
                apply_layout(layout, black_box(area), &mut containers, padding);
                black_box(&containers);
            });
        }
    }

    // The core on a desktop without a windowing system: everything
    // between an event and the windows placed, but for the system itself.
    for count in [10usize, 50] {
        bench.case(&format!("core/open_{count}_windows"), || {
            let (mut core, mut desktop) = desk(&["main"], LayoutType::Dwindle);
            open(&mut core, &mut desktop, count);
            black_box(desktop.placements);
        });
    }
    {
        let (mut core, mut desktop) = desk(&["main"], LayoutType::Dwindle);
        open(&mut core, &mut desktop, 49);
        bench.case("core/one_more_window_beside_49", || {
            let id = desktop.open("one more", "app.exe");
            core.handle_window_event(&mut desktop, WindowEvent::Created(id))
                .expect("created");
            core.flush_pending_layouts(&mut desktop).expect("a layout");
            desktop.close(id);
            core.handle_window_event(&mut desktop, WindowEvent::Destroyed(id))
                .expect("destroyed");
            core.flush_pending_layouts(&mut desktop).expect("a layout");
            desktop.applied.clear();
        });
    }
    for count in [5usize, 25] {
        let (mut core, mut desktop) = desk(&["main", "two"], LayoutType::Dwindle);
        open(&mut core, &mut desktop, count);
        switch(&mut core, &mut desktop, 1);
        open(&mut core, &mut desktop, count);
        let mut shown = 1;
        bench.case(&format!("core/switch_workspace_{count}_on_each"), || {
            shown = 1 - shown;
            switch(&mut core, &mut desktop, shown);
        });
    }

    // The state as it is published: every display with its workspaces
    // and their windows, put together after each pass in which something
    // happened, and held against the one before for what to tell.
    {
        let names = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
        let (mut core, mut desktop) = desk(&names, LayoutType::Dwindle);
        for workspace in 0..3 {
            switch(&mut core, &mut desktop, workspace);
            open(&mut core, &mut desktop, 10);
        }
        let config = gevrek_model::examples::starting_configuration().desktop;
        bench.case("state/9_workspaces_30_windows", || {
            black_box(core.monitor_data(&desktop, black_box(&config)));
        });
        let (before, _) = core.monitor_data(&desktop, &config);
        let after = before.clone();
        bench.case("state/what_changed_when_nothing_did", || {
            black_box(gevrek_engine::overlay_data::changes(
                black_box(&before),
                black_box(&after),
            ));
        });
        bench.case("state/copied", || {
            black_box(black_box(&before).clone());
        });
    }

    // The rules held against the windows.
    {
        let engine = rules();
        let windows: Vec<WindowInfo> = (0..50).map(window).collect();
        bench.case("rules/20_rules_against_50_windows", || {
            for window in &windows {
                black_box(engine.resolve(black_box(window)));
            }
        });
    }

    // The overview of the windows: where each picture goes.
    for count in [6usize, 30] {
        let windows: Vec<(WindowId, Rect)> = (0..count)
            .map(|index| {
                let rect = Rect {
                    x: (index as i32 % 5) * 400,
                    y: (index as i32 / 5) * 250,
                    width: 700 + (index as i32 % 3) * 300,
                    height: 500 + (index as i32 % 4) * 150,
                };
                (WindowId(index as _), rect)
            })
            .collect();
        bench.case(&format!("overview/arrange_{count}_windows"), || {
            black_box(gevrek_engine::expose::arrange(black_box(&windows), area));
        });
    }

    // A configuration (the one gevrek starts with: 116 bindings): read,
    // written, checked, and held against another for a reload.
    {
        let config = gevrek_model::examples::starting_configuration();
        let text = serde_json::to_string_pretty(&config).expect("written");
        bench.case("config/read", || {
            black_box(serde_json::from_str::<Config>(black_box(&text)).expect("read"));
        });
        bench.case("config/write", || {
            black_box(serde_json::to_string_pretty(black_box(&config)).expect("written"));
        });
        bench.case("config/validate", || {
            black_box(black_box(&config).validate());
        });
        let mut other = config.clone();
        other.desktop.appearance.border_width += 1;
        bench.case("reload/plan_nothing_changed", || {
            black_box(ConfigDiff::between(black_box(&config), black_box(&config)).steps());
        });
        bench.case("reload/plan_one_setting_changed", || {
            black_box(ConfigDiff::between(black_box(&config), black_box(&other)).steps());
        });
    }

    bench.written();
}
