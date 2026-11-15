//! Isolated gRPC contract tests.
//!
//! Each test server binds an ephemeral port and uses fake runtime channels, so
//! the suite never touches a live Gevrek process, window layout, or user config.
use gevrek_engine::layouts::{MasterPosition, NewWindowPlacement};
use gevrek_host::command::{
    BatchCommandResult, ManualCommand, RuntimeCommand, RuntimeCommandResult,
};
use gevrek_model::command::DispatchCommand;
use gevrek_model::config::Config;
use gevrek_model::settings::{BindingProfile, MonitorProfile, ReservedArea, WorkspaceProfile};
use gevrek_protocol::proto::execution_event::Event;
use gevrek_protocol::window_manager_server::WindowManagerServer;
use gevrek_protocol::WMClient;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

struct TestServer {
    address: String,
    shutdown: Option<oneshot::Sender<()>>,
    config_path: PathBuf,
    events: Arc<broadcast::Sender<gevrek_protocol::proto::ExecutionEvent>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = std::fs::remove_file(&self.config_path);
    }
}

async fn start_test_server() -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("test listener should have a local address");

    let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let config_path = std::env::temp_dir().join(format!(
        "gevrek-grpc-test-{}-{}.json",
        std::process::id(),
        unique
    ));

    // Stands in for the runtime, the owner of the running configuration:
    // every replacement is applied by publishing it.
    let (config_sender, config_receiver) =
        std::sync::mpsc::channel::<gevrek_host::config_store::ConfigRequest>();
    let (config_publisher, config_watch) = gevrek_host::config_store::config_channel(Config::new());
    std::thread::spawn(move || {
        while let Ok(request) = config_receiver.recv() {
            let config = request.resolve(&config_publisher.current());
            config_publisher.publish_request(&config);
        }
    });

    let (manual_sender, manual_receiver) = std::sync::mpsc::channel::<ManualCommand>();
    std::thread::spawn(move || {
        while let Ok(command) = manual_receiver.recv() {
            let result = match command.command {
                RuntimeCommand::Trigger(ref target) if target == "fail" => {
                    Err("requested failure".to_string())
                }
                RuntimeCommand::DispatchBatch {
                    commands,
                    stop_on_error,
                } => {
                    let mut stopped = false;
                    let results = commands
                        .iter()
                        .map(|command| {
                            if stopped {
                                return BatchCommandResult {
                                    success: false,
                                    skipped: true,
                                    error: Some("Skipped after failure".to_string()),
                                };
                            }
                            if matches!(command, DispatchCommand::CloseActive) {
                                stopped = stop_on_error;
                                BatchCommandResult {
                                    success: false,
                                    skipped: false,
                                    error: Some("simulated close failure".to_string()),
                                }
                            } else {
                                BatchCommandResult {
                                    success: true,
                                    skipped: false,
                                    error: None,
                                }
                            }
                        })
                        .collect();
                    Ok(RuntimeCommandResult::Batch(results))
                }
                _ => Ok(RuntimeCommandResult::Complete),
            };
            let _ = command.response.send(result);
        }
    });

    let stop_flag = Arc::new(Mutex::new(false));
    let (event_sender, _) = broadcast::channel(100);
    let events = Arc::new(event_sender);
    let server = gevrek_server::WMServer::new(
        gevrek_server::ConfigSource {
            config: config_watch,
            config_path: config_path.clone(),
            has_config_file: false,
            config_load_error: None,
        },
        gevrek_server::RuntimeLinks {
            runtime_config_sender: config_sender,
            manual_command_sender: manual_sender,
            execution_paused: Arc::new(Mutex::new(false)),
            stop_flag: Arc::clone(&stop_flag),
            overlay_data: Arc::new(Mutex::new(None)),
            event_broadcaster: Arc::clone(&events),
            runtime_healthy: Arc::new(AtomicBool::new(true)),
            runtime_error: Arc::new(Mutex::new(None)),
        },
    )
    .with_backend(gevrek_host::backend_info::BackendInfo::new(
        "test-system",
        "test-backend",
        &["only_this_backend"],
    ));

    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let incoming = TcpListenerStream::new(listener);
    tokio::spawn(async move {
        let shutdown = async move {
            tokio::select! {
                _ = shutdown_receiver => {}
                _ = async {
                    loop {
                        if *stop_flag.lock().unwrap_or_else(|p| p.into_inner()) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } => {}
            }
        };

        Server::builder()
            .add_service(WindowManagerServer::new(server))
            .serve_with_incoming_shutdown(incoming, shutdown)
            .await
            .expect("test server should run");
    });

    let endpoint = format!("http://{}", address);
    for _ in 0..50 {
        if WMClient::connect(endpoint.clone()).await.is_ok() {
            return TestServer {
                address: endpoint,
                shutdown: Some(shutdown_sender),
                config_path,
                events,
            };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("test server did not become reachable");
}

#[tokio::test]
async fn health_and_node_definitions_are_available() {
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();

    let health = client.health_check().await.unwrap();
    assert!(health.healthy);
    assert!(!health.version.is_empty());

    let definitions = client.get_node_definitions().await.unwrap();
    assert!(!definitions.definitions.is_empty());
    assert!(definitions
        .definitions
        .iter()
        .all(|definition| !definition.type_id.is_empty()
            && !definition.display_name.is_empty()
            && !definition.category.is_empty()));

    let capabilities = client.get_capabilities().await.unwrap();
    assert_eq!(capabilities.protocol_version, "2");
    assert_eq!(
        capabilities.config_schema_version,
        gevrek_model::settings::DESKTOP_CONFIG_SCHEMA_VERSION
    );
    assert!(capabilities
        .features
        .iter()
        .any(|name| name == "dispatch_batch"));
    assert!(capabilities
        .features
        .iter()
        .any(|name| name == "extension_events_v1"));
    // (Features only one backend has are covered by
    // `capabilities_name_the_backend_and_only_its_features`.)
    assert_eq!(capabilities.dispatchers.len(), 140);
    assert!(capabilities
        .dispatchers
        .iter()
        .any(|dispatcher| dispatcher.id == "show_shortcuts"));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_stackbar" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "force_focus_window" && dispatcher.argument_kind == "window_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "new_workspace" && dispatcher.argument_kind == "optional_workspace_name"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "close_workspace" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "ensure_workspaces"
            && dispatcher.argument_kind == "monitor_selector_and_integer"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "ensure_named_workspace"
            && dispatcher.argument_kind == "monitor_selector_and_name"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_window_container_behaviour" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_workspace_window_container_behaviour"
            && dispatcher.argument_kind == "workspace_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_transparency" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "set_transparency_alpha" && dispatcher.argument_kind == "integer"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "set_global_work_area_offset"
            && dispatcher.argument_kind == "work_area_offsets"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "set_workspace_work_area_offset"
            && dispatcher.argument_kind == "work_area_offsets"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "set_workspace_padding" && dispatcher.argument_kind == "pixels"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "adjust_container_padding" && dispatcher.argument_kind == "pixels"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "save_resize" && dispatcher.argument_kind == "path"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "load_resize" && dispatcher.argument_kind == "path"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "quick_save_resize" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "quick_load_resize" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "watch_configuration" && dispatcher.argument_kind == "boolean"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "focus_direction" && dispatcher.argument_kind == "direction"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "move_workspace_to_monitor"
            && dispatcher.argument_kind == "monitor_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "rename_workspace"
            && dispatcher.argument_kind == "workspace_selector_and_name"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "set_floating" && dispatcher.argument_kind == "boolean"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "clear_window_tags" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "focus_window" && dispatcher.argument_kind == "window_selector"
    }));
    assert!(capabilities
        .dispatchers
        .iter()
        .any(|dispatcher| { dispatcher.id == "focus_last" && dispatcher.argument_kind == "none" }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "move_window"
            && dispatcher.argument_kind == "window_selector_xy_and_coordinate_mode"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "set_window_floating"
            && dispatcher.argument_kind == "window_selector_and_boolean"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_window_fullscreen" && dispatcher.argument_kind == "window_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_selected_window_tag"
            && dispatcher.argument_kind == "window_selector_and_tag"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "move_selected_to_workspace_and_follow"
            && dispatcher.argument_kind == "window_selector_and_workspace_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_window_group" && dispatcher.argument_kind == "window_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "cycle_window_group"
            && dispatcher.argument_kind == "window_selector_and_boolean"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "focus_window_group_index"
            && dispatcher.argument_kind == "window_selector_and_integer"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "move_window_in_group"
            && dispatcher.argument_kind == "window_selector_and_boolean"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "resize_edge" && dispatcher.argument_kind == "direction_and_sizing"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "resize_axis" && dispatcher.argument_kind == "axis_and_sizing"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "flip_layout" && dispatcher.argument_kind == "axis"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_monocle" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "promote_window" && dispatcher.argument_kind == "direction"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "move_window_to_monitor"
            && dispatcher.argument_kind == "window_selector_and_monitor_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "move_window_to_monitor_and_follow"
            && dispatcher.argument_kind == "window_selector_and_monitor_selector"
    }));
    assert!(capabilities
        .dispatchers
        .iter()
        .any(|dispatcher| { dispatcher.id == "retile" && dispatcher.argument_kind == "none" }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_tiling" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_workspace_tiling"
            && dispatcher.argument_kind == "workspace_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_workspace_float_override"
            && dispatcher.argument_kind == "workspace_selector"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_title_bar" && dispatcher.argument_kind == "none"
    }));
    assert!(capabilities.dispatchers.iter().any(|dispatcher| {
        dispatcher.id == "toggle_window_title_bar" && dispatcher.argument_kind == "window_selector"
    }));
    assert!(capabilities
        .node_types
        .iter()
        .any(|node| node == "LaunchProcess"));
}

#[tokio::test]
async fn pushed_graph_becomes_active_and_is_persisted() {
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();
    let mut config = Config::new();
    config
        .node_graph
        .add_node(gevrek_model::graph::NodeType::OnStartup, [100.0, 100.0]);

    let response = client
        .push_graph(gevrek::conversions::node_graph_to_proto(&config.node_graph))
        .await
        .unwrap();
    assert!(response.success);
    assert_eq!(response.node_count, 1);

    let active = client.get_current_graph().await.unwrap();
    assert_eq!(active.node_count, 1);
    assert!(server.config_path.exists());
    let persisted = Config::load(&server.config_path).unwrap();
    assert_eq!(persisted.node_graph.nodes.len(), 1);
}

#[tokio::test]
async fn complete_configuration_round_trips_validates_and_persists() {
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();

    let initial = client.get_configuration().await.unwrap();
    let mut config: Config = serde_json::from_str(&initial.json).unwrap();
    config.desktop.focus.history_size = 42;
    config.desktop.workspaces.push(WorkspaceProfile {
        name: "Development".to_string(),
        master_position: MasterPosition::Right,
        new_window_placement: NewWindowPlacement::AfterFocused,
        ..WorkspaceProfile::default()
    });
    config.desktop.monitors.push(MonitorProfile {
        reserved: ReservedArea {
            top: 36,
            right: 240,
            ..ReservedArea::default()
        },
        ..MonitorProfile::default()
    });
    config.desktop.bindings.push(BindingProfile {
        name: "Focus next".to_string(),
        key: "J".to_string(),
        ..BindingProfile::default()
    });
    config.node_graph = gevrek_model::examples::built_in_examples()
        .remove(0)
        .config
        .node_graph;
    let submitted_graph = serde_json::to_value(&config.node_graph).unwrap();

    let response = client
        .update_configuration(gevrek_protocol::ConfigurationDocument {
            json: serde_json::to_string_pretty(&config).unwrap(),
            version: config.version.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(response.success);

    let fetched = client.get_configuration().await.unwrap();
    let fetched: Config = serde_json::from_str(&fetched.json).unwrap();
    assert_eq!(fetched.desktop.focus.history_size, 42);
    assert_eq!(fetched.desktop.workspaces[0].name, "Development");
    assert_eq!(
        fetched.desktop.workspaces[0].master_position,
        MasterPosition::Right
    );
    assert_eq!(fetched.desktop.monitors[0].reserved.right, 240);
    assert_eq!(fetched.desktop.bindings[0].key, "J");
    assert_eq!(
        serde_json::to_value(&fetched.node_graph).unwrap(),
        submitted_graph
    );
    assert_eq!(
        Config::load(&server.config_path)
            .unwrap()
            .desktop
            .focus
            .history_size,
        42
    );

    let mut invalid = fetched;
    invalid.desktop.focus.history_size = 0;
    let error = client
        .update_configuration(gevrek_protocol::ConfigurationDocument {
            json: serde_json::to_string(&invalid).unwrap(),
            version: invalid.version,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<tonic::Status>().map(|s| s.code()),
        Some(tonic::Code::InvalidArgument)
    );
}

/// Two clients read the same revision; the second one to apply must not
/// overwrite the first one's change without noticing.
#[tokio::test]
async fn a_graph_push_reports_its_revision_and_is_announced_with_the_editor_id() {
    use gevrek_protocol::proto::execution_event::Event;
    let server = start_test_server().await;
    let mut editor = WMClient::connect(server.address.clone()).await.unwrap();
    let mut events = editor.subscribe_to_events().await.unwrap();
    let base = editor.get_configuration().await.unwrap().revision;

    let mut graph = gevrek_model::graph::NodeGraph::new();
    graph.add_node(gevrek_model::graph::NodeType::OnStartup, [0.0, 0.0]);
    let mut proto = gevrek::conversions::node_graph_to_proto(&graph);
    proto.editor_id = "alice".to_string();
    let response = editor.push_graph(proto).await.unwrap();
    assert!(response.success);
    assert_eq!(response.revision, base + 1);
    assert_eq!(
        editor.get_configuration().await.unwrap().revision,
        response.revision
    );

    let announced = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events
                .message()
                .await
                .unwrap()
                .and_then(|event| event.event)
            {
                Some(Event::ConfigurationChanged(changed)) => break changed,
                _ => continue,
            }
        }
    })
    .await
    .expect("the push is announced");
    assert_eq!(
        (announced.revision, announced.origin_client_id.as_str()),
        (response.revision, "alice")
    );
}

#[tokio::test]
async fn a_graph_push_from_a_stale_revision_is_refused() {
    let server = start_test_server().await;
    let mut alice = WMClient::connect(server.address.clone()).await.unwrap();
    let mut bob = WMClient::connect(server.address.clone()).await.unwrap();
    let base = alice.get_configuration().await.unwrap().revision;

    let graph_with = |nodes: usize, base_revision: u64, editor: &str| {
        let mut graph = gevrek_model::graph::NodeGraph::new();
        for index in 0..nodes {
            graph.add_node(
                gevrek_model::graph::NodeType::OnStartup,
                [index as f32, 0.0],
            );
        }
        let mut proto = gevrek::conversions::node_graph_to_proto(&graph);
        proto.editor_id = editor.to_string();
        proto.base_revision = base_revision;
        proto
    };

    let first = alice
        .push_graph(graph_with(1, base, "alice"))
        .await
        .unwrap();
    assert!(first.success && !first.conflict);

    // Bob edited from `base`: refused, and Alice's graph still runs.
    let stale = bob.push_graph(graph_with(2, base, "bob")).await.unwrap();
    assert!(!stale.success && stale.conflict);
    assert_eq!(stale.revision, first.revision);
    let running: Config =
        serde_json::from_str(&bob.get_configuration().await.unwrap().json).unwrap();
    assert_eq!(running.node_graph.nodes.len(), 1);

    // From the current revision, or unconditionally, it goes through.
    let retry = bob
        .push_graph(graph_with(2, stale.revision, "bob"))
        .await
        .unwrap();
    assert!(retry.success);
    let forced = alice.push_graph(graph_with(3, 0, "script")).await.unwrap();
    assert!(forced.success && forced.revision > retry.revision);
}

#[tokio::test]
async fn a_working_graph_from_a_stale_revision_is_not_shared() {
    let server = start_test_server().await;
    let mut alice = WMClient::connect(server.address.clone()).await.unwrap();
    let mut bob = WMClient::connect(server.address.clone()).await.unwrap();
    let base = alice.get_configuration().await.unwrap().revision;

    let graph_with = |nodes: usize, base_revision: u64, editor: &str| {
        let mut graph = gevrek_model::graph::NodeGraph::new();
        for index in 0..nodes {
            graph.add_node(
                gevrek_model::graph::NodeType::OnStartup,
                [index as f32, 0.0],
            );
        }
        let mut proto = gevrek::conversions::node_graph_to_proto(&graph);
        proto.editor_id = editor.to_string();
        proto.base_revision = base_revision;
        proto
    };

    // A draft from the running revision is shared.
    assert!(
        bob.update_working_graph(graph_with(2, base, "bob"))
            .await
            .unwrap()
            .success
    );
    assert_eq!(alice.get_working_graph().await.unwrap().node_count, 2);

    // Alice applies a graph; Bob's next draft is still made from `base`.
    let pushed = alice
        .push_graph(graph_with(1, base, "alice"))
        .await
        .unwrap();
    assert!(pushed.success);
    let stale = bob
        .update_working_graph(graph_with(3, base, "bob"))
        .await
        .unwrap();
    assert!(!stale.success);
    assert!(stale
        .error
        .unwrap_or_default()
        .contains("changed since revision"));
    assert_eq!(alice.get_working_graph().await.unwrap().node_count, 1);

    // From the current revision, or unconditionally, it is shared again.
    let current = bob
        .update_working_graph(graph_with(3, pushed.revision, "bob"))
        .await
        .unwrap();
    assert!(current.success);
    let forced = bob
        .update_working_graph(graph_with(4, 0, "script"))
        .await
        .unwrap();
    assert!(forced.success);
    assert_eq!(alice.get_working_graph().await.unwrap().node_count, 4);
}

#[tokio::test]
async fn a_stale_configuration_update_is_refused_as_a_conflict() {
    let server = start_test_server().await;
    let mut alice = WMClient::connect(server.address.clone()).await.unwrap();
    let mut bob = WMClient::connect(server.address.clone()).await.unwrap();

    let document = alice.get_configuration().await.unwrap();
    let base = document.revision;
    assert_eq!(bob.get_configuration().await.unwrap().revision, base);

    let edited = |history_size: usize| {
        let mut config: Config = serde_json::from_str(&document.json).unwrap();
        config.desktop.focus.history_size = history_size;
        serde_json::to_string(&config).unwrap()
    };
    let update =
        |json: String, base_revision: u64, client: &str| gevrek_protocol::ConfigurationDocument {
            json,
            base_revision,
            client_id: client.to_string(),
            ..Default::default()
        };

    let first = alice
        .update_configuration(update(edited(11), base, "alice"))
        .await
        .unwrap();
    assert!(first.success && !first.conflict);
    assert!(
        first.revision > base,
        "an applied change gets a new revision"
    );

    // Bob still holds `base`: refused, and told the current revision.
    let stale = bob
        .update_configuration(update(edited(22), base, "bob"))
        .await
        .unwrap();
    assert!(
        !stale.success && stale.conflict,
        "base {base}, first {first:?}, stale {stale:?}"
    );
    assert_eq!(stale.revision, first.revision);
    assert!(stale.error.unwrap().contains("changed since revision"));
    let running: Config =
        serde_json::from_str(&bob.get_configuration().await.unwrap().json).unwrap();
    assert_eq!(
        running.desktop.focus.history_size, 11,
        "Alice's change is intact"
    );

    // After reloading, Bob applies on top of Alice's revision.
    let retry = bob
        .update_configuration(update(edited(22), stale.revision, "bob"))
        .await
        .unwrap();
    assert!(retry.success);
    assert!(retry.revision > first.revision);

    // base_revision 0 is the unconditional form (scripts, the CLI).
    let forced = alice
        .update_configuration(update(edited(33), 0, "alice"))
        .await
        .unwrap();
    assert!(forced.success);
}

/// The settings program's session model against a real server: two of them start from
/// the same revision; the slower one is refused, sees the other's change as a
/// conflict, and resolves it either way without losing anyone's work
/// silently.
#[tokio::test]
async fn two_editor_sessions_never_overwrite_each_other_silently() {
    use gevrek_studio::session::{Fetched, RemoteNotice, Session};

    async fn open(client: &mut WMClient, id: &str) -> Session {
        let document = client.get_configuration().await.unwrap();
        let config: Config = serde_json::from_str(&document.json).unwrap();
        Session::new(id, config, document.revision)
    }
    /// Apply the session's draft; on a refusal, fetch what the WM runs.
    async fn apply(client: &mut WMClient, session: &mut Session) -> bool {
        let request = session.begin_apply().expect("there is a draft to apply");
        let response = client
            .update_configuration(gevrek_protocol::ConfigurationDocument {
                json: serde_json::to_string(&request.config).unwrap(),
                base_revision: request.base_revision,
                client_id: request.client_id,
                ..Default::default()
            })
            .await
            .unwrap();
        if response.success {
            session.applied(response.revision);
            return true;
        }
        assert!(response.conflict, "{:?}", response.error);
        let document = client.get_configuration().await.unwrap();
        let running: Config = serde_json::from_str(&document.json).unwrap();
        assert_eq!(
            session.remote_fetched(running, document.revision),
            Fetched::Conflict
        );
        false
    }
    async fn running_history(client: &mut WMClient) -> usize {
        let document = client.get_configuration().await.unwrap();
        serde_json::from_str::<Config>(&document.json)
            .unwrap()
            .desktop
            .focus
            .history_size
    }

    let server = start_test_server().await;
    let mut alice_client = WMClient::connect(server.address.clone()).await.unwrap();
    let mut bob_client = WMClient::connect(server.address.clone()).await.unwrap();
    let mut alice = open(&mut alice_client, "alice").await;
    let mut bob = open(&mut bob_client, "bob").await;

    // Both edit; Alice applies first.
    alice.edit().desktop.focus.history_size = 11;
    bob.edit().desktop.focus.history_size = 22;
    assert!(apply(&mut alice_client, &mut alice).await);
    assert_eq!(
        alice.on_remote_revision(alice.base_revision(), "alice"),
        RemoteNotice::OwnEcho
    );
    assert_eq!(
        bob.on_remote_revision(alice.base_revision(), "alice"),
        RemoteNotice::Fetch
    );

    // Bob is refused and now holds a conflict; nothing was overwritten.
    assert!(!apply(&mut bob_client, &mut bob).await);
    assert!(bob.has_conflict());
    assert_eq!(running_history(&mut bob_client).await, 11);
    assert_eq!(
        bob.config().desktop.focus.history_size,
        22,
        "his draft is intact"
    );
    assert_eq!(bob.remote().unwrap().0.desktop.focus.history_size, 11);

    // He keeps his version: a deliberate overwrite on top of Alice's revision.
    bob.keep_mine();
    assert!(apply(&mut bob_client, &mut bob).await);
    assert_eq!(running_history(&mut bob_client).await, 22);

    // Alice has no unsaved edits, so Bob's change simply arrives.
    assert_eq!(
        alice.on_remote_revision(bob.base_revision(), "bob"),
        RemoteNotice::Fetch
    );
    let document = alice_client.get_configuration().await.unwrap();
    let running: Config = serde_json::from_str(&document.json).unwrap();
    assert_eq!(
        alice.remote_fetched(running, document.revision),
        Fetched::FastForwarded
    );
    assert_eq!(alice.config().desktop.focus.history_size, 22);

    // The other resolution: edit, get refused, take theirs.
    alice.edit().desktop.focus.history_size = 33;
    bob.edit().desktop.focus.history_size = 44;
    assert!(apply(&mut bob_client, &mut bob).await);
    assert!(!apply(&mut alice_client, &mut alice).await);
    alice.take_theirs();
    assert!(!alice.is_dirty());
    assert_eq!(alice.config().desktop.focus.history_size, 44);
    assert_eq!(running_history(&mut alice_client).await, 44);
}

/// Subscribers learn of every configuration change, with its revision and
/// who made it, so a client can ignore the echo of its own change.
#[tokio::test]
async fn configuration_changes_are_announced_with_revision_and_origin() {
    let server = start_test_server().await;
    let mut editor = WMClient::connect(server.address.clone()).await.unwrap();
    let mut observer = WMClient::connect(server.address.clone()).await.unwrap();
    let mut events = observer.subscribe_to_events().await.unwrap();

    let document = editor.get_configuration().await.unwrap();
    let response = editor
        .update_configuration(gevrek_protocol::ConfigurationDocument {
            json: document.json.clone(),
            base_revision: document.revision,
            client_id: "editor-1".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(response.success);

    let changed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.message().await.unwrap().expect("stream stays open");
            if let Some(gevrek_protocol::execution_event::Event::ConfigurationChanged(changed)) =
                event.event
            {
                break changed;
            }
        }
    })
    .await
    .expect("a ConfigurationChanged event arrives");
    assert_eq!(changed.revision, response.revision);
    assert_eq!(changed.origin_client_id, "editor-1");
}

#[tokio::test]
async fn pause_resume_and_state_are_consistent() {
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();

    assert!(client.pause_execution().await.unwrap().success);
    let paused_state = client.query_state().await.unwrap();
    assert!(paused_state.execution_paused);
    assert!(paused_state.animations_enabled);
    assert_eq!(paused_state.animation_duration_ms, 180);
    assert_eq!(paused_state.animation_curve, "EaseInOut");
    assert!(paused_state.animate_layout);
    assert!(paused_state.animate_workspace);
    assert!(paused_state.animate_borders);
    assert_eq!(paused_state.animation_channels.len(), 4);
    assert!(paused_state
        .animation_channels
        .iter()
        .any(|channel| channel.scope == "workspace"
            && channel.duration_ms == 240
            && channel.curve == "CubicOut"));
    assert!(paused_state
        .animation_channels
        .iter()
        .any(|channel| channel.scope == "window"
            && channel.duration_ms == 220
            && channel.curve == "BackOut"
            && channel.enter_style == "Scale"
            && channel.exit_style == "Fade"));
    assert_eq!(paused_state.active_binding_submap, "default");
    assert!(!paused_state.dim_inactive);
    assert_eq!(paused_state.inactive_window_opacity, 220);
    assert!(!paused_state.dim_fullscreen);
    assert_eq!(paused_state.shadow_policy, "Default");
    assert!(client.resume_execution().await.unwrap().success);
    assert!(!client.query_state().await.unwrap().execution_paused);
}

#[tokio::test]
async fn capabilities_name_the_backend_and_only_its_features() {
    // What a system can do is what its backend says, not what the server
    // was compiled for: the test server's backend is an invented one.
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();
    let capabilities = client.get_capabilities().await.unwrap();
    let has = |feature: &str| capabilities.features.iter().any(|name| name == feature);

    assert!(has("typed_dispatch") && has("configuration_revisions_v1"));
    assert!(has("focus_steal_prevention") && has("dispatch_json"));
    assert_eq!(
        (
            capabilities.platform.as_str(),
            capabilities.backend.as_str()
        ),
        ("test-system", "test-backend")
    );
    assert!(has("only_this_backend"));
    // Nothing of a real backend leaks in.
    for foreign in [
        gevrek_model::graph::FEATURE_WINDOWS_WINDOW_EFFECTS,
        "native_backdrop",
        "render_time_animation",
        "server_side_borders",
        "bar",
        "drawn_effects",
    ] {
        assert!(!has(foreign), "{foreign}");
    }
}

#[tokio::test]
async fn working_graph_updates_notify_other_clients() {
    let server = start_test_server().await;
    let mut subscriber = WMClient::connect(server.address.clone()).await.unwrap();
    let mut editor = WMClient::connect(server.address.clone()).await.unwrap();
    let mut events = subscriber.subscribe_to_events().await.unwrap();

    let mut graph =
        gevrek::conversions::node_graph_to_proto(&Config::with_default_graph().node_graph);
    graph.editor_id = "test-editor".to_string();
    assert!(editor.update_working_graph(graph).await.unwrap().success);

    let event = tokio::time::timeout(Duration::from_secs(1), events.message())
        .await
        .expect("working-graph event should arrive")
        .unwrap()
        .expect("event stream should remain open");
    assert!(matches!(
        event.event,
        Some(Event::WorkingGraphUpdated(update)) if update.editor_id == "test-editor"
    ));
}

#[tokio::test]
async fn structured_runtime_events_reach_extension_subscribers() {
    let server = start_test_server().await;
    let mut subscriber = WMClient::connect(server.address.clone()).await.unwrap();
    let mut events = subscriber.subscribe_to_events().await.unwrap();

    server
        .events
        .send(gevrek_protocol::proto::ExecutionEvent {
            event: Some(Event::RuntimeEvent(gevrek_protocol::proto::RuntimeEvent {
                kind: "window.focused".to_string(),
                timestamp_ms: 123,
                window_id: Some(456),
                fields: [("title".to_string(), "Editor".to_string())]
                    .into_iter()
                    .collect(),
            })),
        })
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(1), events.message())
        .await
        .expect("runtime event should arrive")
        .unwrap()
        .expect("event stream should remain open");
    assert!(matches!(
        event.event,
        Some(Event::RuntimeEvent(event))
            if event.kind == "window.focused"
                && event.window_id == Some(456)
                && event.fields.get("title").map(String::as_str) == Some("Editor")
    ));
}

#[tokio::test]
async fn manual_commands_report_success_failure_and_pause() {
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();

    assert!(
        client
            .trigger_manual_event("focus_next".to_string())
            .await
            .unwrap()
            .success
    );
    let failed = client
        .trigger_manual_event("fail".to_string())
        .await
        .unwrap();
    assert!(!failed.success);
    assert_eq!(failed.error.as_deref(), Some("requested failure"));

    client.pause_execution().await.unwrap();
    let paused = client
        .trigger_manual_event("focus_next".to_string())
        .await
        .unwrap();
    assert!(!paused.success);
    assert_eq!(paused.error.as_deref(), Some("Execution is paused"));
}

#[tokio::test]
async fn a_command_as_json_reaches_the_runtime_and_an_unknown_one_is_refused() {
    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();
    let response = client
        .dispatch_command(r#"{"command":"focus_next"}"#)
        .await
        .unwrap();
    assert!(response.success, "{:?}", response.error);

    let with_fields = client
        .dispatch_command(r#"{"command":"exec","program":"true","args":["--x"]}"#)
        .await
        .unwrap();
    assert!(with_fields.success, "{:?}", with_fields.error);

    let unknown = client
        .dispatch_command(r#"{"command":"teleport_window","to":"mars"}"#)
        .await;
    assert!(
        unknown.is_err(),
        "a command this WM does not know is refused"
    );
    let not_json = client.dispatch_command("focus_next").await;
    assert!(not_json.is_err());
}

#[tokio::test]
async fn typed_dispatch_validates_and_reaches_runtime() {
    use gevrek_protocol::proto::{Direction, DispatchRequest, DispatchType};

    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();
    let response = client
        .dispatch(DispatchRequest {
            command: DispatchType::FocusNext as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: String::new(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(response.success);

    let named_workspace = client
        .dispatch(DispatchRequest {
            command: DispatchType::SwitchWorkspace as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "name:code".to_string(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(named_workspace.success);

    let group_dispatch = client
        .dispatch(DispatchRequest {
            command: DispatchType::CycleGroupNext as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: String::new(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(group_dispatch.success);

    let layout_dispatch = client
        .dispatch(DispatchRequest {
            command: DispatchType::SetLayout as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "dwindle".to_string(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(layout_dispatch.success);

    let dwindle_dispatch = client
        .dispatch(DispatchRequest {
            command: DispatchType::SetDwindleDirection as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "vertical".to_string(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(dwindle_dispatch.success);

    let invalid_dwindle_direction = client
        .dispatch(DispatchRequest {
            command: DispatchType::SetDwindleDirection as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "diagonal".to_string(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(invalid_dwindle_direction.is_err());

    let invalid_ratio = client
        .dispatch(DispatchRequest {
            command: DispatchType::SetLayoutRatio as i32,
            direction: Direction::Unspecified as i32,
            x: 950,
            y: 0,
            argument: String::new(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(invalid_ratio.is_err());

    let invalid_workspace = client
        .dispatch(DispatchRequest {
            command: DispatchType::MoveToWorkspace as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "somewhere".to_string(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(invalid_workspace.is_err());

    let empty_tag = client
        .dispatch(DispatchRequest {
            command: DispatchType::AddWindowTag as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "   ".to_string(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(empty_tag.is_err());

    let submap = client
        .dispatch(DispatchRequest {
            command: DispatchType::SetBindingSubmap as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "resize".to_string(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(submap.success);

    let empty_submap = client
        .dispatch(DispatchRequest {
            command: DispatchType::SetBindingSubmap as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: " ".to_string(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(empty_submap.is_err());

    let restart_startup = client
        .dispatch(DispatchRequest {
            command: DispatchType::RestartStartup as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: "status-bar".to_string(),
            secondary_argument: String::new(),
        })
        .await
        .unwrap();
    assert!(restart_startup.success);

    let empty_startup = client
        .dispatch(DispatchRequest {
            command: DispatchType::LaunchStartup as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: " ".to_string(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(empty_startup.is_err());

    let invalid = client
        .dispatch(DispatchRequest {
            command: DispatchType::FocusDirection as i32,
            direction: Direction::Unspecified as i32,
            x: 0,
            y: 0,
            argument: String::new(),
            secondary_argument: String::new(),
        })
        .await;
    assert!(invalid.is_err());
}

#[tokio::test]
async fn dispatch_batches_validate_first_and_report_each_result() {
    use gevrek_protocol::proto::{Direction, DispatchRequest, DispatchType};

    let server = start_test_server().await;
    let mut client = WMClient::connect(server.address.clone()).await.unwrap();
    let request = |command| DispatchRequest {
        command: command as i32,
        direction: Direction::Unspecified as i32,
        x: 0,
        y: 0,
        argument: String::new(),
        secondary_argument: String::new(),
    };

    let response = client
        .dispatch_batch(
            vec![
                request(DispatchType::FocusNext),
                request(DispatchType::CloseActive),
                request(DispatchType::FocusPrevious),
            ],
            true,
        )
        .await
        .unwrap();
    assert!(!response.success);
    assert_eq!(response.results.len(), 3);
    assert!(response.results[0].success);
    assert!(!response.results[1].success);
    assert!(!response.results[1].skipped);
    assert!(response.results[2].skipped);

    let invalid = client
        .dispatch_batch(
            vec![
                request(DispatchType::FocusNext),
                request(DispatchType::FocusDirection),
            ],
            false,
        )
        .await;
    assert!(invalid.is_err());

    assert!(client.pause_execution().await.unwrap().success);
    let paused = client
        .dispatch_batch(vec![request(DispatchType::FocusNext)], false)
        .await
        .unwrap();
    assert!(!paused.success);
    assert!(paused.results[0].skipped);
    assert_eq!(
        paused.results[0].error.as_deref(),
        Some("Execution is paused")
    );
}

#[tokio::test]
async fn concurrent_clients_are_supported() {
    let server = start_test_server().await;
    let mut clients = Vec::new();
    for _ in 0..5 {
        let address = server.address.clone();
        clients.push(tokio::spawn(async move {
            let mut client = WMClient::connect(address).await.unwrap();
            assert!(client.health_check().await.unwrap().healthy);
        }));
    }
    for client in clients {
        client.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_address_fails_without_hanging() {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        WMClient::connect("http://127.0.0.1:99999".to_string()),
    )
    .await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_err());
}

#[tokio::test]
async fn shutdown_rpc_stops_the_server() {
    let server = start_test_server().await;
    let address = server.address.clone();
    let mut client = WMClient::connect(address.clone()).await.unwrap();
    assert!(client.shutdown(false, 0).await.unwrap().success);

    for _ in 0..50 {
        if WMClient::connect(address.clone()).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("server remained reachable after shutdown");
}
