//! The state on its way out of the window manager, timed: every time a
//! panel, the studio or the command line asks for it, what the runtime
//! published is copied, made into the protocol's message, written for
//! the line, read at the other end and made into JSON there.
//!
//!     cargo bench -p gevrek-server
//!
//! (`crates/engine/benches/support/harness.rs` says how a case is timed.)

use gevrek_engine::core::WmCore;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::executor::Executor;
use gevrek_engine::focus_manager::FocusManager;
use gevrek_engine::layouts::LayoutType;
use gevrek_engine::outputs::{MonitorHandle, Outputs};
use gevrek_engine::overlay_data::OverlayData;
use gevrek_engine::rules::RuleEngine;
use gevrek_engine::testing::fake_desktop::FakeDesktop;
use gevrek_engine::snap_zones::SnapZoneManager;
use gevrek_model::command::{DispatchCommand, WorkspaceSelector};
use gevrek_model::graph::NodeGraph;
use gevrek_protocol::proto::OverlayDataResponse;
use prost::Message;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

#[path = "../../engine/benches/support/harness.rs"]
mod harness;

const OUTPUT: MonitorHandle = MonitorHandle(1);

/// What a runtime publishes of a desktop with nine workspaces and ten
/// windows on each of the first three.
fn published() -> OverlayData {
    let rules = Arc::new(Mutex::new(RuleEngine::new()));
    let executor = Executor::new(NodeGraph::new(), Arc::new(Mutex::new(Vec::new())), rules);
    let mut outputs = Outputs::with_workspace_count(0);
    outputs.add_test_monitor(OUTPUT, "Primary");
    for name in ["1", "2", "3", "4", "5", "6", "7", "8", "9"] {
        outputs
            .create_workspace(OUTPUT, name.to_string(), LayoutType::Dwindle)
            .expect("a workspace");
    }
    let mut core = WmCore::new(
        executor,
        outputs,
        FocusManager::new(),
        SnapZoneManager::new(Default::default()),
    );
    let mut desktop = FakeDesktop::new(OUTPUT);
    for workspace in 0..3 {
        core.dispatch(
            &mut desktop,
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Index(workspace),
            },
        )
        .expect("switched");
        for index in 0..10 {
            let id = desktop.open(
                &format!("Document {index} of workspace {workspace} - An editor of some kind"),
                "editor.exe",
            );
            core.handle_window_event(&mut desktop, WindowEvent::Created(id))
                .expect("created");
            core.flush_pending_layouts(&mut desktop).expect("a layout");
        }
    }
    let config = gevrek_model::examples::starting_configuration().desktop;
    let (monitors, total_windows) = core.monitor_data(&desktop, &config);
    OverlayData {
        timestamp: std::time::SystemTime::now(),
        monitors,
        graph_node_count: 24,
        total_windows,
        event_loop_fps: 60.0,
        last_event: Some("window created".to_string()),
        active_animations: 0,
        active_binding_submap: "default".to_string(),
        tiling_suspended: false,
        gesture: Default::default(),
        rule_states: Vec::new(),
        scratchpad_states: core.scratchpad_data(),
        startup_states: Vec::new(),
        metrics: None,
        notices: Vec::new(),
        accent: Some((181, 0, 125)),
        keyboard_layout: None,
        frames: Vec::new(),
    }
}

fn main() {
    let mut bench = harness::Bench::from_args();
    let data = published();
    let config = gevrek_model::examples::starting_configuration();
    let appearance = config.desktop.appearance.clone();
    let shadow = config.desktop.platform.windows.shadow;

    // In the server, for each one who asks.
    bench.case("state_out/copied_from_the_runtime", || {
        black_box(black_box(&data).clone());
    });
    bench.case("state_out/the_look_copied", || {
        black_box(black_box(&config).desktop.appearance.clone());
    });
    bench.case("state_out/made_into_the_message", || {
        black_box(gevrek_server::overlay_response(black_box(&data), &appearance, shadow));
    });
    let response = gevrek_server::overlay_response(&data, &appearance, shadow);
    bench.case("state_out/written_for_the_line", || {
        black_box(black_box(&response).encode_to_vec());
    });

    // At the other end (the shell, the studio).
    let bytes = response.encode_to_vec();
    println!("({} bytes on the line)", bytes.len());
    bench.case("state_in/read_from_the_line", || {
        black_box(OverlayDataResponse::decode(black_box(bytes.as_slice())).expect("read"));
    });
    bench.case("state_in/made_into_json", || {
        black_box(serde_json::to_value(black_box(&response)).expect("JSON"));
    });
    bench.written();
}
