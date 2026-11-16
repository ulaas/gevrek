//! The gRPC server of the window manager: what the CLI, the editor and the
//! overlay talk to. It does not know which windowing system the WM runs
//! on: it reads the published configuration and state, and hands commands
//! and configuration requests to the WM loop through channels.

// Every call answers with `tonic::Status` when it fails. It is a large
// value, and it is the type the protocol library asks for.
#![allow(clippy::result_large_err)]

use anyhow::Result;
pub mod node_registry;
use gevrek_host::command::{ManualCommand, RuntimeCommand};
use gevrek_host::config_store::{wait_for_request, ConfigRequest, ConfigWatch};
use gevrek_model::command::{
    Direction as CoreDirection, DispatchCommand, MonitorTarget, WindowSelector, WorkspaceSelector,
};
use gevrek_model::{config::Config, settings::DESKTOP_CONFIG_SCHEMA_VERSION};
use gevrek_protocol::window_manager_server::{WindowManager as WMService, WindowManagerServer};
use gevrek_protocol::*;
use node_registry::NodeRegistry;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tonic::{transport::Server, Request, Response, Status};

fn gesture_state_to_proto(
    state: &gevrek_engine::overlay_data::GestureStateData,
) -> gevrek_protocol::GestureState {
    gevrek_protocol::GestureState {
        available: state.available,
        backend: state.backend.clone(),
        device_count: state.device_count,
        configured_count: state.configured_count,
        active: state
            .active
            .as_ref()
            .map(|progress| gevrek_protocol::GestureProgress {
                phase: progress.phase.clone(),
                fingers: u32::from(progress.fingers),
                direction: progress.direction.clone(),
                progress: progress.progress,
                delta_x: progress.delta_x,
                delta_y: progress.delta_y,
            }),
        last_error: state.last_error.clone(),
    }
}

fn workspace_overlay_to_proto(
    workspace: &gevrek_engine::overlay_data::WorkspaceData,
) -> WorkspaceOverlayData {
    WorkspaceOverlayData {
        id: workspace.id as u32,
        name: workspace.name.clone(),
        layout_type: workspace.layout_type.clone(),
        total_windows: workspace.total_windows as u32,
        tiled_windows: workspace.tiled_windows as u32,
        floating_windows: workspace.floating_windows as u32,
        container_count: workspace.container_count as u32,
        windows: workspace
            .windows
            .iter()
            .map(|window| WindowInfo {
                window_id: window.handle.0 as u64,
                title: window.title.clone(),
                class: String::new(),
                exe: String::new(),
                position: Some(Position {
                    x: window.x as f32,
                    y: window.y as f32,
                }),
                size: Some(Size {
                    width: window.width,
                    height: window.height,
                }),
                is_focused: false,
                is_minimized: window.is_minimized,
                is_maximized: window.is_maximized,
                tags: Vec::new(),
                matched_rules: Vec::new(),
                effective_rule_effects: Vec::new(),
                effective_opacity: 255,
                is_dimmed: false,
                shadow_policy: "Default".to_string(),
                app_id: String::new(),
            })
            .collect(),
        detailed_windows: workspace
            .detailed_windows
            .iter()
            .map(|window| DetailedWindowInfo {
                window_id: window.handle.0 as u64,
                title: window.title.clone(),
                class: window.class.clone(),
                exe: window.exe.clone(),
                position: Some(Position {
                    x: window.x as f32,
                    y: window.y as f32,
                }),
                size: Some(Size {
                    width: window.width,
                    height: window.height,
                }),
                is_focused: window.is_focused,
                is_minimized: window.is_minimized,
                is_maximized: window.is_maximized,
                is_managed: window.is_managed,
                is_tiled: window.is_tiled,
                is_floating: window.is_floating,
                has_border: window.has_border,
                is_border_window: window.is_border_window,
                workspace_id: window.workspace_id as u32,
                tags: window.tags.clone(),
                matched_rules: window.matched_rules.clone(),
                effective_rule_effects: window.effective_rule_effects.clone(),
                effective_opacity: window.effective_opacity as u32,
                is_dimmed: window.is_dimmed,
                shadow_policy: window.shadow_policy.clone(),
                app_id: window.app_id.clone(),
                icon: window.icon.clone(),
            })
            .collect(),
        master_position: workspace.master_position.clone(),
        new_window_placement: workspace.new_window_placement.clone(),
        tiling_enabled: workspace.tiling_enabled,
        float_override: workspace.float_override,
        window_container_behaviour: workspace.window_container_behaviour.clone(),
    }
}

type StateSnapshot = (
    Vec<gevrek_protocol::proto::WorkspaceState>,
    Vec<WindowInfo>,
    Vec<gevrek_protocol::proto::MonitorInfo>,
    Option<String>,
);

fn overlay_state_to_proto(data: &gevrek_engine::overlay_data::OverlayData) -> StateSnapshot {
    use gevrek_protocol::proto::{MonitorInfo, WorkspaceState};

    let mut windows = Vec::new();
    let mut monitors = Vec::new();
    let mut workspaces = Vec::new();
    let mut active_workspace_id = None;
    let mut primary_workspace_id = None;

    for monitor in &data.monitors {
        if monitor.is_primary {
            primary_workspace_id = Some(monitor.workspace.id.to_string());
        }
        if monitor.is_focused {
            active_workspace_id = Some(monitor.workspace.id.to_string());
        }
        monitors.push(MonitorInfo {
            name: monitor.name.clone(),
            output_id: monitor.handle.0 as u64,
            connector: monitor.connector.clone(),
            position: Some(Position {
                x: monitor.x as f32,
                y: monitor.y as f32,
            }),
            size: Some(Size {
                width: monitor.width,
                height: monitor.height,
            }),
            is_primary: monitor.is_primary,
            work_position: Some(Position {
                x: monitor.work_x as f32,
                y: monitor.work_y as f32,
            }),
            work_size: Some(Size {
                width: monitor.work_width,
                height: monitor.work_height,
            }),
            reserved_left: monitor.reserved_left,
            reserved_top: monitor.reserved_top,
            reserved_right: monitor.reserved_right,
            reserved_bottom: monitor.reserved_bottom,
            is_focused: monitor.is_focused,
        });

        for workspace in &monitor.workspaces {
            workspaces.push(WorkspaceState {
                id: workspace.id.to_string(),
                name: workspace.name.clone(),
                monitor_id: monitor.handle.0.to_string(),
                window_ids: workspace
                    .detailed_windows
                    .iter()
                    .map(|window| window.handle.0.to_string())
                    .collect(),
                layout_type: workspace.layout_type.clone(),
                dwindle_direction: workspace.dwindle_direction.clone(),
                dwindle_preserve_split: workspace.dwindle_preserve_split,
                pseudotiled_windows: workspace.pseudotiled_windows as u32,
                master_position: workspace.master_position.clone(),
                new_window_placement: workspace.new_window_placement.clone(),
                tiling_enabled: workspace.tiling_enabled,
                float_override: workspace.float_override,
                window_container_behaviour: workspace.window_container_behaviour.clone(),
            });
            windows.extend(workspace.detailed_windows.iter().map(|window| WindowInfo {
                window_id: window.handle.0 as u64,
                title: window.title.clone(),
                class: window.class.clone(),
                exe: window.exe.clone(),
                position: Some(Position {
                    x: window.x as f32,
                    y: window.y as f32,
                }),
                size: Some(Size {
                    width: window.width,
                    height: window.height,
                }),
                is_focused: window.is_focused,
                is_minimized: window.is_minimized,
                is_maximized: window.is_maximized,
                tags: window.tags.clone(),
                matched_rules: window.matched_rules.clone(),
                effective_rule_effects: window.effective_rule_effects.clone(),
                effective_opacity: window.effective_opacity as u32,
                is_dimmed: window.is_dimmed,
                shadow_policy: window.shadow_policy.clone(),
                app_id: window.app_id.clone(),
            }));
        }
    }

    (
        workspaces,
        windows,
        monitors,
        active_workspace_id.or(primary_workspace_id),
    )
}

/// The gRPC server implementation for the Window Manager
pub struct WMServer {
    /// The running configuration, as published by its owner, the runtime.
    config: ConfigWatch,
    /// Serializes configuration writes: read latest, save, hand to the
    /// runtime, wait until it is applied.
    config_write: Arc<tokio::sync::Mutex<()>>,
    /// The highest revision announced to subscribers (`ConfigurationChanged`).
    announced_revision: Arc<AtomicU64>,
    /// How many `ConfigRequest`s this server has sent to the runtime.
    config_requests_sent: AtomicU64,
    config_path: PathBuf,
    has_config_file: bool, // Only save to disk if there's an actual config file
    /// Set when the config file existed but failed to parse at startup. The
    /// server then runs read-only on defaults and refuses to write the file,
    /// so the user's broken config is never replaced behind their back.
    config_load_error: Mutex<Option<String>>,
    runtime_config_sender: std::sync::mpsc::Sender<ConfigRequest>,
    manual_command_sender: std::sync::mpsc::Sender<ManualCommand>,
    execution_paused: Arc<Mutex<bool>>,
    stop_flag: Arc<Mutex<bool>>,
    event_broadcaster: Arc<broadcast::Sender<ExecutionEvent>>,
    runtime_healthy: Arc<AtomicBool>,
    runtime_error: Arc<Mutex<Option<String>>>,
    start_time: SystemTime,
    node_registry: NodeRegistry,
    working_graph: Arc<Mutex<gevrek_model::graph::NodeGraph>>,
    overlay_data: Arc<Mutex<Option<gevrek_engine::overlay_data::OverlayData>>>,
    /// What the backend this server runs in says it is and can do.
    backend: gevrek_host::backend_info::BackendInfo,
    /// Whether made-up input is accepted (a WM started for testing).
    input_injection: bool,
}

/// While the session is locked nobody sees what a command does, and the
/// person at the keyboard may not be the user: only what the keys of a
/// locked keyboard may do is carried out (sound, brightness, media, the
/// displays), and locking itself.
fn refuse_while_locked(command: &gevrek_model::command::DispatchCommand) -> Result<(), Status> {
    use gevrek_model::command::{DispatchCommand, ShellAction};
    if !gevrek_host::lock::is_locked() {
        return Ok(());
    }
    let allowed = matches!(
        command,
        DispatchCommand::Shell {
            action: ShellAction::Lock
                | ShellAction::VolumeUp
                | ShellAction::VolumeDown
                | ShellAction::VolumeMute
                | ShellAction::MicMute
                | ShellAction::BrightnessUp
                | ShellAction::BrightnessDown
                | ShellAction::MediaPlayPause
                | ShellAction::MediaNext
                | ShellAction::MediaPrevious
                | ShellAction::MediaStop
                | ShellAction::DisplaysOff
        }
    );
    if allowed {
        Ok(())
    } else {
        Err(locked_status())
    }
}

fn locked_status() -> Status {
    Status::failed_precondition("The session is locked")
}

impl WMServer {
    /// Say which backend this server runs in. Each window manager calls it
    /// once, right after `new`.
    pub fn with_backend(mut self, backend: gevrek_host::backend_info::BackendInfo) -> Self {
        self.backend = backend;
        self
    }

    /// Accept made-up input (`InjectInput`). For a window manager started
    /// for automated checks only.
    pub fn with_input_injection(mut self, allowed: bool) -> Self {
        self.input_injection = allowed;
        self
    }
}

impl WMServer {
    /// Hand a command to the runtime and wait for what it says.
    async fn run_command(
        &self,
        command: gevrek_model::command::DispatchCommand,
    ) -> Result<Response<StatusResponse>, Status> {
        refuse_while_locked(&command)?;
        let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
        self.manual_command_sender
            .send(ManualCommand {
                command: RuntimeCommand::Dispatch(command),
                response: response_sender,
            })
            .map_err(|_| Status::unavailable("Window manager runtime is not running"))?;
        // A loop that waits for work is told there is some.
        gevrek_host::wake::now();

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), response_receiver)
            .await
            .map_err(|_| Status::deadline_exceeded("Dispatcher timed out"))?
            .map_err(|_| Status::unavailable("Window manager runtime stopped before replying"))?;

        Ok(Response::new(match result {
            Ok(_) => StatusResponse {
                success: true,
                error: None,
            },
            Err(error) => StatusResponse {
                success: false,
                error: Some(error),
            },
        }))
    }
}

/// The configuration the server answers with: the running one as it is
/// watched, the file it is from, and what went wrong reading it.
pub struct ConfigSource {
    pub config: ConfigWatch,
    pub config_path: PathBuf,
    pub has_config_file: bool,
    pub config_load_error: Option<String>,
}

/// The server's lines to the window manager's own loop: where requests
/// and commands go, the flags they share, the state that is told to
/// clients, the events that are passed on, and how the loop fares.
pub struct RuntimeLinks {
    pub runtime_config_sender: std::sync::mpsc::Sender<ConfigRequest>,
    pub manual_command_sender: std::sync::mpsc::Sender<ManualCommand>,
    pub execution_paused: Arc<Mutex<bool>>,
    pub stop_flag: Arc<Mutex<bool>>,
    pub overlay_data: Arc<Mutex<Option<gevrek_engine::overlay_data::OverlayData>>>,
    pub event_broadcaster: Arc<broadcast::Sender<ExecutionEvent>>,
    pub runtime_healthy: Arc<AtomicBool>,
    pub runtime_error: Arc<Mutex<Option<String>>>,
}

impl WMServer {
    pub fn new(source: ConfigSource, links: RuntimeLinks) -> Self {
        let ConfigSource {
            config,
            config_path,
            has_config_file,
            config_load_error,
        } = source;
        let RuntimeLinks {
            runtime_config_sender,
            manual_command_sender,
            execution_paused,
            stop_flag,
            overlay_data,
            event_broadcaster,
            runtime_healthy,
            runtime_error,
        } = links;
        // Initialize working graph with the current config graph
        let working_graph = config.borrow().config.node_graph.clone();

        let config_write = Arc::new(tokio::sync::Mutex::new(()));
        let announced_revision = Arc::new(AtomicU64::new(config.borrow().revision));
        // Changes this server does not make itself (the file watcher, runtime
        // toggles) are announced too. Taking the write lock first lets a
        // handler that is applying a change announce it with its client id.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let mut watch = config.clone();
            let (write, announced, events) = (
                Arc::clone(&config_write),
                Arc::clone(&announced_revision),
                Arc::clone(&event_broadcaster),
            );
            runtime.spawn(async move {
                while watch.changed().await.is_ok() {
                    let _writer = write.lock().await;
                    let revision = watch.borrow().revision;
                    announce_configuration(&events, &announced, revision, "");
                }
            });
        }

        Self {
            config,
            config_write,
            announced_revision,
            config_requests_sent: AtomicU64::new(0),
            config_path,
            has_config_file,
            config_load_error: Mutex::new(config_load_error),
            runtime_config_sender,
            manual_command_sender,
            execution_paused,
            stop_flag,
            event_broadcaster,
            runtime_healthy,
            runtime_error,
            backend: gevrek_host::backend_info::BackendInfo::none(),
            input_injection: false,
            start_time: SystemTime::now(),
            node_registry: NodeRegistry::new(),
            working_graph: Arc::new(Mutex::new(working_graph)),
            overlay_data,
        }
    }

    fn get_uptime(&self) -> i64 {
        self.start_time.elapsed().unwrap_or_default().as_secs() as i64
    }

    /// The configuration the runtime is running right now.
    fn running_config(&self) -> Arc<Config> {
        Arc::clone(&self.config.borrow().config)
    }

    #[cfg(test)]
    fn running_config_json(&self) -> String {
        serde_json::to_string(&*self.running_config()).unwrap()
    }

    /// Persist `file` and ask the runtime to apply `request`. Returns the
    /// revision the runtime published for it, so a following
    /// `GetConfiguration` reads it back. Callers hold `config_write`.
    async fn apply_configuration(
        &self,
        file: &Config,
        request: ConfigRequest,
    ) -> Result<u64, Status> {
        self.ensure_config_writable()?;
        file.save(&self.config_path)
            .map_err(|error| Status::internal(format!("Failed to save configuration: {error}")))?;
        self.runtime_config_sender
            .send(request)
            .map_err(|_| Status::unavailable("Window manager runtime is not running"))?;
        // A loop that waits for work is told there is some.
        gevrek_host::wake::now();
        // Wait for this request, not for any publish: a runtime toggle or a
        // request left queued by an earlier timeout publishes too.
        let request_number = self.config_requests_sent.fetch_add(1, Ordering::SeqCst) + 1;
        wait_for_request(&self.config, request_number, CONFIG_APPLY_TIMEOUT)
            .await
            .ok_or_else(|| {
                Status::deadline_exceeded(
                    "The configuration was saved, but the window manager has not applied it yet",
                )
            })
    }

    /// The startup parse error, cleared once the file parses again (or has
    /// been moved away), so fixing the file by hand needs no restart.
    fn current_config_load_error(&self) -> Option<String> {
        let mut error = self
            .config_load_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if error.is_some()
            && (!self.config_path.exists() || Config::load(&self.config_path).is_ok())
        {
            log::info!("Config file is readable again; leaving read-only mode");
            *error = None;
        }
        error.clone()
    }

    /// Refuses to write the config file while it is in a state we failed to
    /// parse: overwriting it would destroy the user's configuration.
    fn ensure_config_writable(&self) -> Result<(), Status> {
        match self.current_config_load_error() {
            Some(error) => Err(Status::failed_precondition(format!(
                "{error}. Fix or move the file, then apply again."
            ))),
            None => Ok(()),
        }
    }
}

/// Tell subscribers that the running configuration is now at `revision`.
/// Each revision is announced once: by the handler that made the change
/// (with its client id), or else by the watcher task (empty origin).
fn announce_configuration(
    events: &broadcast::Sender<ExecutionEvent>,
    announced: &AtomicU64,
    revision: u64,
    origin_client_id: &str,
) {
    if announced.fetch_max(revision, Ordering::SeqCst) >= revision {
        return;
    }
    let _ = events.send(ExecutionEvent {
        event: Some(execution_event::Event::ConfigurationChanged(
            ConfigurationChanged {
                revision,
                origin_client_id: origin_client_id.to_string(),
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
            },
        )),
    });
}

/// Forwards broadcast events to one subscriber's stream. A subscriber that
/// falls behind gets an `EventsDropped` marker and keeps streaming; ending the
/// stream on lag used to look like a lost connection to the editor.
async fn forward_events(
    mut receiver: broadcast::Receiver<ExecutionEvent>,
    sender: tokio::sync::mpsc::Sender<Result<ExecutionEvent, Status>>,
) {
    loop {
        let event = match receiver.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(count)) => {
                log::warn!("Event subscriber lagged; {count} events dropped");
                ExecutionEvent {
                    event: Some(execution_event::Event::EventsDropped(
                        gevrek_protocol::proto::EventsDropped { count },
                    )),
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        if sender.send(Ok(event)).await.is_err() {
            break;
        }
    }
}

/// How long a configuration write waits for the runtime to apply it. A hot
/// reload re-runs startup automation, so this is generous.
const CONFIG_APPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A runtime loop that has not started a pass for this long is reported as
/// not responding. Longer than a single pass (or the startup before the first
/// one) should take.
const RUNTIME_STALL_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(15);

fn config_load_error_message(path: &Path, error: &anyhow::Error) -> String {
    format!(
        "Config file {} could not be parsed ({error:#}); the window manager is          running read-only with defaults and will not overwrite it",
        path.display()
    )
}

fn dispatch_request_to_core(request: DispatchRequest) -> Result<DispatchCommand, Status> {
    let command = DispatchType::try_from(request.command)
        .map_err(|_| Status::invalid_argument("Unknown dispatcher"))?;
    let direction = || match Direction::try_from(request.direction)
        .map_err(|_| Status::invalid_argument("Unknown direction"))?
    {
        Direction::Left => Ok(CoreDirection::Left),
        Direction::Right => Ok(CoreDirection::Right),
        Direction::Up => Ok(CoreDirection::Up),
        Direction::Down => Ok(CoreDirection::Down),
        Direction::Unspecified => Err(Status::invalid_argument(
            "This dispatcher requires a direction",
        )),
    };
    let window_selector = || {
        request
            .argument
            .parse::<WindowSelector>()
            .map_err(Status::invalid_argument)
    };
    let relative = || match request
        .secondary_argument
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "relative" | "true" | "1" => Ok(true),
        "absolute" | "false" | "0" | "" => Ok(false),
        value => Err(Status::invalid_argument(format!(
            "Unknown coordinate mode '{value}'; use absolute or relative"
        ))),
    };

    Ok(match command {
        DispatchType::FocusNext => DispatchCommand::FocusNext,
        DispatchType::FocusPrevious => DispatchCommand::FocusPrevious,
        DispatchType::FocusLast => DispatchCommand::FocusLast,
        DispatchType::FocusWindow => DispatchCommand::FocusWindow {
            selector: request
                .argument
                .parse::<WindowSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::ForceFocusWindow => DispatchCommand::ForceFocusWindow {
            selector: request
                .argument
                .parse::<WindowSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::FocusDirection => DispatchCommand::FocusDirection {
            direction: direction()?,
        },
        DispatchType::SwapNext => DispatchCommand::SwapNext,
        DispatchType::SwapPrevious => DispatchCommand::SwapPrevious,
        DispatchType::SwapDirection => DispatchCommand::SwapDirection {
            direction: direction()?,
        },
        DispatchType::MoveActive => DispatchCommand::MoveActive {
            delta_x: request.x,
            delta_y: request.y,
        },
        DispatchType::MoveWindow => DispatchCommand::MoveWindow {
            selector: window_selector()?,
            x: request.x,
            y: request.y,
            relative: relative()?,
        },
        DispatchType::ResizeActive => DispatchCommand::ResizeActive {
            delta_width: request.x,
            delta_height: request.y,
        },
        DispatchType::ResizeWindow => DispatchCommand::ResizeWindow {
            selector: window_selector()?,
            width: request.x,
            height: request.y,
            relative: relative()?,
        },
        DispatchType::CloseActive => DispatchCommand::CloseActive,
        DispatchType::CloseWindow => DispatchCommand::CloseWindow {
            selector: window_selector()?,
        },
        DispatchType::MinimizeActive => DispatchCommand::MinimizeActive,
        DispatchType::ToggleMaximized => DispatchCommand::ToggleMaximized,
        DispatchType::ToggleFullscreen => DispatchCommand::ToggleFullscreen,
        DispatchType::ToggleWindowFullscreen => DispatchCommand::ToggleWindowFullscreen {
            selector: window_selector()?,
        },
        DispatchType::TogglePinned => DispatchCommand::TogglePinned,
        DispatchType::ToggleWindowPinned => DispatchCommand::ToggleWindowPinned {
            selector: window_selector()?,
        },
        DispatchType::ToggleFloating => DispatchCommand::ToggleFloating,
        DispatchType::ToggleWindowFloating => DispatchCommand::ToggleWindowFloating {
            selector: window_selector()?,
        },
        DispatchType::SetPinned => DispatchCommand::SetPinned {
            enabled: required_boolean(request.x, "SetPinned")?,
        },
        DispatchType::SetWindowPinned => DispatchCommand::SetWindowPinned {
            selector: window_selector()?,
            enabled: required_boolean(request.x, "SetWindowPinned")?,
        },
        DispatchType::SetFloating => DispatchCommand::SetFloating {
            enabled: required_boolean(request.x, "SetFloating")?,
        },
        DispatchType::SetWindowFloating => DispatchCommand::SetWindowFloating {
            selector: window_selector()?,
            enabled: required_boolean(request.x, "SetWindowFloating")?,
        },
        DispatchType::CenterActive => DispatchCommand::CenterActive,
        DispatchType::ToggleFloatingAside => DispatchCommand::ToggleFloatingAside,
        DispatchType::CenterWindow => DispatchCommand::CenterWindow {
            selector: window_selector()?,
        },
        DispatchType::RaiseActive => DispatchCommand::RaiseActive,
        DispatchType::RaiseWindow => DispatchCommand::RaiseWindow {
            selector: window_selector()?,
        },
        DispatchType::LowerActive => DispatchCommand::LowerActive,
        DispatchType::LowerWindow => DispatchCommand::LowerWindow {
            selector: window_selector()?,
        },
        DispatchType::ClearWindowTags => DispatchCommand::ClearWindowTags,
        DispatchType::ClearSelectedWindowTags => DispatchCommand::ClearSelectedWindowTags {
            selector: window_selector()?,
        },
        DispatchType::AddWindowTag => DispatchCommand::AddWindowTag {
            tag: required_argument(&request.argument, "AddWindowTag")?,
        },
        DispatchType::AddSelectedWindowTag => DispatchCommand::AddSelectedWindowTag {
            selector: window_selector()?,
            tag: required_argument(&request.secondary_argument, "AddSelectedWindowTag")?,
        },
        DispatchType::RemoveWindowTag => DispatchCommand::RemoveWindowTag {
            tag: required_argument(&request.argument, "RemoveWindowTag")?,
        },
        DispatchType::RemoveSelectedWindowTag => DispatchCommand::RemoveSelectedWindowTag {
            selector: window_selector()?,
            tag: required_argument(&request.secondary_argument, "RemoveSelectedWindowTag")?,
        },
        DispatchType::ToggleWindowTag => DispatchCommand::ToggleWindowTag {
            tag: required_argument(&request.argument, "ToggleWindowTag")?,
        },
        DispatchType::ToggleSelectedWindowTag => DispatchCommand::ToggleSelectedWindowTag {
            selector: window_selector()?,
            tag: required_argument(&request.secondary_argument, "ToggleSelectedWindowTag")?,
        },
        DispatchType::EnableRule => DispatchCommand::EnableRule {
            name: required_argument(&request.argument, "EnableRule")?,
        },
        DispatchType::DisableRule => DispatchCommand::DisableRule {
            name: required_argument(&request.argument, "DisableRule")?,
        },
        DispatchType::ToggleRule => DispatchCommand::ToggleRule {
            name: required_argument(&request.argument, "ToggleRule")?,
        },
        DispatchType::SwitchWorkspace => DispatchCommand::SwitchWorkspaceSelector {
            selector: request
                .argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveToWorkspace => DispatchCommand::MoveToWorkspaceSelector {
            selector: request
                .argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveToWorkspaceAndFollow => DispatchCommand::MoveToWorkspaceAndFollow {
            selector: request
                .argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveSelectedToWorkspace => DispatchCommand::MoveWindowToWorkspace {
            window: window_selector()?,
            workspace: request
                .secondary_argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveSelectedToWorkspaceAndFollow => {
            DispatchCommand::MoveWindowToWorkspaceAndFollow {
                window: window_selector()?,
                workspace: request
                    .secondary_argument
                    .parse::<WorkspaceSelector>()
                    .map_err(Status::invalid_argument)?,
            }
        }
        DispatchType::RenameWorkspace => DispatchCommand::RenameWorkspace {
            selector: request
                .argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
            name: required_argument(&request.secondary_argument, "RenameWorkspace name")?,
        },
        DispatchType::FocusMonitor => DispatchCommand::FocusMonitor {
            target: request
                .argument
                .parse::<MonitorTarget>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveWorkspaceToMonitor => DispatchCommand::MoveWorkspaceToMonitor {
            target: request
                .argument
                .parse::<MonitorTarget>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::SwapActiveWorkspaces => DispatchCommand::SwapActiveWorkspaces {
            target: request
                .argument
                .parse::<MonitorTarget>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveWindowToMonitor => DispatchCommand::MoveWindowToMonitor {
            window: window_selector()?,
            target: request
                .secondary_argument
                .parse::<MonitorTarget>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::MoveWindowToMonitorAndFollow => {
            DispatchCommand::MoveWindowToMonitorAndFollow {
                window: window_selector()?,
                target: request
                    .secondary_argument
                    .parse::<MonitorTarget>()
                    .map_err(Status::invalid_argument)?,
            }
        }
        DispatchType::Retile => DispatchCommand::Retile,
        DispatchType::ToggleTiling => DispatchCommand::ToggleTiling,
        DispatchType::ToggleWorkspaceTiling => DispatchCommand::ToggleWorkspaceTiling {
            selector: request
                .argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::ToggleWorkspaceFloatOverride => {
            DispatchCommand::ToggleWorkspaceFloatOverride {
                selector: request
                    .argument
                    .parse::<WorkspaceSelector>()
                    .map_err(Status::invalid_argument)?,
            }
        }
        DispatchType::NewWorkspace => {
            let name = request.argument.trim();
            DispatchCommand::NewWorkspace {
                name: (!name.is_empty()).then(|| name.to_string()),
            }
        }
        DispatchType::CloseWorkspace => DispatchCommand::CloseWorkspace,
        DispatchType::EnsureWorkspaces => {
            if !(1..=128).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Workspace count must be between 1 and 128",
                ));
            }
            DispatchCommand::EnsureWorkspaces {
                target: request
                    .argument
                    .parse::<MonitorTarget>()
                    .map_err(Status::invalid_argument)?,
                count: request.x as usize,
            }
        }
        DispatchType::EnsureNamedWorkspace => DispatchCommand::EnsureNamedWorkspace {
            target: request
                .argument
                .parse::<MonitorTarget>()
                .map_err(Status::invalid_argument)?,
            name: required_argument(&request.secondary_argument, "EnsureNamedWorkspace name")?,
            layout: None,
        },
        DispatchType::ToggleWindowContainerBehaviour => {
            DispatchCommand::ToggleWindowContainerBehaviour
        }
        DispatchType::ToggleWorkspaceWindowContainerBehaviour => {
            DispatchCommand::ToggleWorkspaceWindowContainerBehaviour {
                selector: request
                    .argument
                    .parse::<WorkspaceSelector>()
                    .map_err(Status::invalid_argument)?,
            }
        }
        DispatchType::ToggleTitleBar => DispatchCommand::ToggleTitleBar,
        DispatchType::ToggleWindowTitleBar => DispatchCommand::ToggleWindowTitleBar {
            selector: window_selector()?,
        },
        DispatchType::FocusMonitorAtCursor => DispatchCommand::FocusMonitorAtCursor,
        DispatchType::FocusMonitorWorkspace => DispatchCommand::FocusMonitorWorkspace {
            target: request
                .argument
                .parse::<MonitorTarget>()
                .map_err(Status::invalid_argument)?,
            workspace: request
                .secondary_argument
                .parse::<WorkspaceSelector>()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::ToggleMouseFollowsFocus => DispatchCommand::ToggleMouseFollowsFocus,
        DispatchType::ManageWindow => DispatchCommand::ManageWindow {
            selector: window_selector()?,
        },
        DispatchType::UnmanageWindow => DispatchCommand::UnmanageWindow {
            selector: window_selector()?,
        },
        DispatchType::RestoreWindows => DispatchCommand::RestoreWindows,
        DispatchType::ResizeEdge => DispatchCommand::ResizeEdge {
            edge: direction()?,
            sizing: request.argument.parse().map_err(Status::invalid_argument)?,
        },
        DispatchType::ResizeAxis => DispatchCommand::ResizeAxis {
            axis: request.argument.parse().map_err(Status::invalid_argument)?,
            sizing: request
                .secondary_argument
                .parse()
                .map_err(Status::invalid_argument)?,
        },
        DispatchType::SetResizeDelta => {
            if !(1..=1000).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Resize delta must be between 1 and 1000 pixels",
                ));
            }
            DispatchCommand::SetResizeDelta { pixels: request.x }
        }
        DispatchType::FlipLayout => DispatchCommand::FlipLayout {
            axis: request.argument.parse().map_err(Status::invalid_argument)?,
        },
        DispatchType::ToggleMonocle => DispatchCommand::ToggleMonocle,
        DispatchType::ToggleLayoutLock => DispatchCommand::ToggleLayoutLock,
        DispatchType::StackAll => DispatchCommand::StackAll,
        DispatchType::UnstackAll => DispatchCommand::UnstackAll,
        DispatchType::ToggleStackbar => DispatchCommand::ToggleStackbar,
        DispatchType::SaveResize => DispatchCommand::SaveResize {
            path: required_argument(&request.argument, "SaveResize")?,
        },
        DispatchType::LoadResize => DispatchCommand::LoadResize {
            path: required_argument(&request.argument, "LoadResize")?,
        },
        DispatchType::QuickSaveResize => DispatchCommand::QuickSaveResize,
        DispatchType::QuickLoadResize => DispatchCommand::QuickLoadResize,
        DispatchType::WatchConfiguration => DispatchCommand::WatchConfiguration {
            enabled: required_boolean(request.x, "WatchConfiguration")?,
        },
        DispatchType::ToggleTransparency => DispatchCommand::ToggleTransparency,
        DispatchType::SetTransparencyAlpha => {
            if !(20..=255).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Transparency alpha must be between 20 and 255",
                ));
            }
            DispatchCommand::SetTransparencyAlpha {
                alpha: request.x as u8,
            }
        }
        DispatchType::SetGlobalWorkAreaOffset => {
            let (left, top, right, bottom) =
                work_area_offset(&request.argument, "SetGlobalWorkAreaOffset")
                    .map_err(Status::invalid_argument)?;
            DispatchCommand::SetGlobalWorkAreaOffset {
                left,
                top,
                right,
                bottom,
            }
        }
        DispatchType::SetWorkspaceWorkAreaOffset => {
            let (left, top, right, bottom) =
                work_area_offset(&request.argument, "SetWorkspaceWorkAreaOffset")
                    .map_err(Status::invalid_argument)?;
            DispatchCommand::SetWorkspaceWorkAreaOffset {
                left,
                top,
                right,
                bottom,
            }
        }
        DispatchType::SetWorkspacePadding => {
            if !(0..=500).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Workspace padding must be between 0 and 500 pixels",
                ));
            }
            DispatchCommand::SetWorkspacePadding { padding: request.x }
        }
        DispatchType::AdjustWorkspacePadding => {
            DispatchCommand::AdjustWorkspacePadding { delta: request.x }
        }
        DispatchType::SetContainerPadding => {
            if !(0..=500).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Container padding must be between 0 and 500 pixels",
                ));
            }
            DispatchCommand::SetContainerPadding { padding: request.x }
        }
        DispatchType::AdjustContainerPadding => {
            DispatchCommand::AdjustContainerPadding { delta: request.x }
        }
        DispatchType::Promote => DispatchCommand::Promote,
        DispatchType::PromoteFocus => DispatchCommand::PromoteFocus,
        DispatchType::PromoteWindow => DispatchCommand::PromoteWindow {
            direction: direction()?,
        },
        DispatchType::CycleNextLayout => DispatchCommand::CycleNextLayout,
        DispatchType::CyclePreviousLayout => DispatchCommand::CyclePreviousLayout,
        DispatchType::SetLayout => DispatchCommand::SetLayout {
            layout: request.argument.parse().map_err(Status::invalid_argument)?,
        },
        DispatchType::SetLayoutRatio => {
            let ratio = request.x as f32 / 1000.0;
            if !(0.1..=0.9).contains(&ratio) {
                return Err(Status::invalid_argument(
                    "Layout ratio must be between 0.1 and 0.9",
                ));
            }
            DispatchCommand::SetLayoutRatio { ratio }
        }
        DispatchType::AdjustLayoutRatio => DispatchCommand::AdjustLayoutRatio {
            delta: request.x as f32 / 1000.0,
        },
        DispatchType::SetMasterCount => {
            if !(1..=8).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Master count must be between 1 and 8",
                ));
            }
            DispatchCommand::SetMasterCount {
                count: request.x as usize,
            }
        }
        DispatchType::AdjustMasterCount => DispatchCommand::AdjustMasterCount { delta: request.x },
        DispatchType::RotateLayout => DispatchCommand::RotateLayout,
        DispatchType::SetLayoutGap => {
            if !(0..=500).contains(&request.x) {
                return Err(Status::invalid_argument(
                    "Layout gap must be between 0 and 500 pixels",
                ));
            }
            DispatchCommand::SetLayoutGap { gap: request.x }
        }
        DispatchType::AdjustLayoutGap => DispatchCommand::AdjustLayoutGap { delta: request.x },
        DispatchType::PromoteToMaster => DispatchCommand::PromoteToMaster,
        DispatchType::SetDwindleDirection => DispatchCommand::SetDwindleDirection {
            direction: request.argument.parse().map_err(Status::invalid_argument)?,
        },
        DispatchType::CycleDwindleDirection => DispatchCommand::CycleDwindleDirection,
        DispatchType::ToggleDwindlePreserveSplit => DispatchCommand::ToggleDwindlePreserveSplit,
        DispatchType::TogglePseudotile => DispatchCommand::TogglePseudotile,
        DispatchType::ToggleWindowPseudotile => DispatchCommand::ToggleWindowPseudotile {
            selector: window_selector()?,
        },
        DispatchType::ToggleGroup => DispatchCommand::ToggleGroup,
        DispatchType::ToggleWindowGroup => DispatchCommand::ToggleWindowGroup {
            selector: window_selector()?,
        },
        DispatchType::ToggleGroupLock => DispatchCommand::ToggleGroupLock,
        DispatchType::ToggleWindowGroupLock => DispatchCommand::ToggleWindowGroupLock {
            selector: window_selector()?,
        },
        DispatchType::CycleGroupNext => DispatchCommand::CycleGroupNext,
        DispatchType::CycleGroupPrevious => DispatchCommand::CycleGroupPrevious,
        DispatchType::CycleWindowGroup => DispatchCommand::CycleWindowGroup {
            selector: window_selector()?,
            forward: required_boolean(request.x, "CycleWindowGroup")?,
        },
        DispatchType::FocusGroupIndex => DispatchCommand::FocusGroupIndex {
            index: usize::try_from(request.x)
                .map_err(|_| Status::invalid_argument("Group index cannot be negative"))?,
        },
        DispatchType::FocusWindowGroupIndex => DispatchCommand::FocusWindowGroupIndex {
            selector: window_selector()?,
            index: usize::try_from(request.x)
                .map_err(|_| Status::invalid_argument("Group index cannot be negative"))?,
        },
        DispatchType::MoveGroupWindow => DispatchCommand::MoveGroupWindow {
            forward: required_boolean(request.x, "MoveGroupWindow")?,
        },
        DispatchType::MoveWindowInGroup => DispatchCommand::MoveWindowInGroup {
            selector: window_selector()?,
            forward: required_boolean(request.x, "MoveWindowInGroup")?,
        },
        DispatchType::MergeGroupNext => DispatchCommand::MergeGroupNext,
        DispatchType::MergeGroupPrevious => DispatchCommand::MergeGroupPrevious,
        DispatchType::UngroupActive => DispatchCommand::UngroupActive,
        DispatchType::ToggleExpose => DispatchCommand::ToggleExpose,
        DispatchType::ShowShortcuts => DispatchCommand::ShowShortcuts,
        DispatchType::ToggleScratchpad => {
            if request.argument.trim().is_empty() {
                return Err(Status::invalid_argument(
                    "ToggleScratchpad requires a scratchpad ID",
                ));
            }
            DispatchCommand::ToggleScratchpad {
                id: request.argument,
            }
        }
        DispatchType::Exec => DispatchCommand::Exec {
            program: required_argument(&request.argument, "Exec")?,
            args: if request.secondary_argument.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&request.secondary_argument).map_err(|error| {
                    Status::invalid_argument(format!(
                        "Exec arguments must be a JSON array of strings: {error}"
                    ))
                })?
            },
        },
        DispatchType::LaunchStartup => DispatchCommand::LaunchStartup {
            id: required_argument(&request.argument, "LaunchStartup")?,
        },
        DispatchType::StopStartup => DispatchCommand::StopStartup {
            id: required_argument(&request.argument, "StopStartup")?,
        },
        DispatchType::RestartStartup => DispatchCommand::RestartStartup {
            id: required_argument(&request.argument, "RestartStartup")?,
        },
        DispatchType::SnapActiveToZone => {
            if request.x < 0 {
                return Err(Status::invalid_argument(
                    "Zone index must be zero or greater",
                ));
            }
            DispatchCommand::SnapActiveToZone {
                index: request.x as usize,
            }
        }
        DispatchType::CycleActiveZoneNext => DispatchCommand::CycleActiveZoneNext,
        DispatchType::CycleActiveZonePrevious => DispatchCommand::CycleActiveZonePrevious,
        DispatchType::SetBindingSubmap => DispatchCommand::SetBindingSubmap {
            name: required_argument(&request.argument, "SetBindingSubmap")?,
        },
        DispatchType::ResetBindingSubmap => DispatchCommand::ResetBindingSubmap,
        DispatchType::Unspecified => {
            return Err(Status::invalid_argument("A dispatcher must be selected"));
        }
    })
}

fn required_argument(value: &str, dispatcher: &str) -> Result<String, Status> {
    let value = value.trim();
    if value.is_empty() {
        Err(Status::invalid_argument(format!(
            "{dispatcher} requires a non-empty argument"
        )))
    } else {
        Ok(value.to_string())
    }
}

fn required_boolean(value: i32, dispatcher: &str) -> Result<bool, Status> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Status::invalid_argument(format!(
            "{dispatcher} requires x to be 0 (off) or 1 (on)"
        ))),
    }
}

/// Parse a comma-separated `left,top,right,bottom` work-area offset
///. Returns the
/// message for an `invalid_argument` status on malformed input.
fn work_area_offset(value: &str, dispatcher: &str) -> Result<(i32, i32, i32, i32), String> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!(
            "{dispatcher} requires four comma-separated edges (left,top,right,bottom)"
        ));
    }
    let mut edges = [0i32; 4];
    for (edge, part) in edges.iter_mut().zip(parts) {
        *edge = part
            .parse::<i32>()
            .map_err(|_| format!("{dispatcher} edges must be integers: '{part}'"))?;
    }
    Ok((edges[0], edges[1], edges[2], edges[3]))
}

fn dispatcher_argument_kind(command: DispatchType) -> &'static str {
    match command {
        DispatchType::FocusDirection | DispatchType::SwapDirection => "direction",
        DispatchType::FocusWindow | DispatchType::ForceFocusWindow => "window_selector",
        DispatchType::CloseWindow
        | DispatchType::ToggleWindowPinned
        | DispatchType::ToggleWindowFloating
        | DispatchType::ToggleWindowFullscreen
        | DispatchType::ToggleWindowPseudotile
        | DispatchType::ToggleWindowGroup
        | DispatchType::ToggleWindowGroupLock
        | DispatchType::ToggleWindowTitleBar
        | DispatchType::ClearSelectedWindowTags
        | DispatchType::CenterWindow
        | DispatchType::RaiseWindow
        | DispatchType::LowerWindow => "window_selector",
        DispatchType::AddSelectedWindowTag
        | DispatchType::RemoveSelectedWindowTag
        | DispatchType::ToggleSelectedWindowTag => "window_selector_and_tag",
        DispatchType::CycleWindowGroup | DispatchType::MoveWindowInGroup => {
            "window_selector_and_boolean"
        }
        DispatchType::FocusWindowGroupIndex => "window_selector_and_integer",
        DispatchType::FocusGroupIndex => "integer",
        DispatchType::MoveGroupWindow => "boolean",
        DispatchType::SetWindowPinned | DispatchType::SetWindowFloating => {
            "window_selector_and_boolean"
        }
        DispatchType::MoveWindow | DispatchType::ResizeWindow => {
            "window_selector_xy_and_coordinate_mode"
        }
        DispatchType::MoveSelectedToWorkspace | DispatchType::MoveSelectedToWorkspaceAndFollow => {
            "window_selector_and_workspace_selector"
        }
        DispatchType::MoveWindowToMonitor | DispatchType::MoveWindowToMonitorAndFollow => {
            "window_selector_and_monitor_selector"
        }
        DispatchType::FocusMonitorWorkspace => "monitor_selector_and_workspace_selector",
        DispatchType::ManageWindow | DispatchType::UnmanageWindow => "window_selector",
        DispatchType::MoveActive | DispatchType::ResizeActive => "x_y_pixels",
        DispatchType::SwitchWorkspace | DispatchType::MoveToWorkspace => "workspace_selector",
        DispatchType::MoveToWorkspaceAndFollow => "workspace_selector",
        DispatchType::ToggleWorkspaceTiling | DispatchType::ToggleWorkspaceFloatOverride => {
            "workspace_selector"
        }
        DispatchType::ToggleWorkspaceWindowContainerBehaviour => "workspace_selector",
        DispatchType::NewWorkspace => "optional_workspace_name",
        DispatchType::EnsureWorkspaces => "monitor_selector_and_integer",
        DispatchType::EnsureNamedWorkspace => "monitor_selector_and_name",
        DispatchType::RenameWorkspace => "workspace_selector_and_name",
        DispatchType::FocusMonitor
        | DispatchType::MoveWorkspaceToMonitor
        | DispatchType::SwapActiveWorkspaces => "monitor_selector",
        DispatchType::SetLayout => "layout",
        DispatchType::ResizeEdge => "direction_and_sizing",
        DispatchType::ResizeAxis => "axis_and_sizing",
        DispatchType::SetResizeDelta => "pixels",
        DispatchType::FlipLayout => "axis",
        DispatchType::PromoteWindow => "direction",
        DispatchType::SetLayoutRatio | DispatchType::AdjustLayoutRatio => "ratio_thousandths",
        DispatchType::SetMasterCount | DispatchType::AdjustMasterCount => "integer",
        DispatchType::SetLayoutGap | DispatchType::AdjustLayoutGap => "pixels",
        DispatchType::SetDwindleDirection => "dwindle_direction",
        DispatchType::SetPinned | DispatchType::SetFloating => "boolean",
        DispatchType::WatchConfiguration => "boolean",
        DispatchType::SetTransparencyAlpha => "integer",
        DispatchType::SetGlobalWorkAreaOffset | DispatchType::SetWorkspaceWorkAreaOffset => {
            "work_area_offsets"
        }
        DispatchType::SetWorkspacePadding
        | DispatchType::AdjustWorkspacePadding
        | DispatchType::SetContainerPadding
        | DispatchType::AdjustContainerPadding => "pixels",
        DispatchType::SaveResize | DispatchType::LoadResize => "path",
        DispatchType::AddWindowTag
        | DispatchType::RemoveWindowTag
        | DispatchType::ToggleWindowTag => "window_tag",
        DispatchType::EnableRule | DispatchType::DisableRule | DispatchType::ToggleRule => {
            "rule_name"
        }
        DispatchType::ToggleScratchpad => "scratchpad_id",
        DispatchType::SnapActiveToZone => "zone_index",
        DispatchType::SetBindingSubmap => "submap_name",
        DispatchType::Exec => "program",
        DispatchType::LaunchStartup | DispatchType::StopStartup | DispatchType::RestartStartup => {
            "startup_id"
        }
        _ => "none",
    }
}

fn dispatcher_display_name(command: DispatchType) -> String {
    command
        .as_str_name()
        .trim_start_matches("DISPATCH_TYPE_")
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| {
                    first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[tonic::async_trait]
impl WMService for WMServer {
    async fn push_graph(
        &self,
        request: Request<NodeGraph>,
    ) -> Result<Response<PushGraphResponse>, Status> {
        if gevrek_host::lock::is_locked() {
            return Err(locked_status());
        }
        log::info!("Received PushGraph request - applying working graph to running config");
        let proto_graph = request.into_inner();

        // If a graph is provided, update the working graph first
        // Otherwise, use the current working graph
        let core_graph = if proto_graph.json.is_empty() {
            log::info!("No graph provided - using current working graph");
            self.working_graph
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        } else {
            log::info!(
                "Graph provided: {} nodes, {} connections - updating working graph first",
                proto_graph.node_count,
                proto_graph.connection_count
            );

            let core_graph = gevrek_protocol::conversions::proto_to_node_graph(&proto_graph)
                .map_err(|e| Status::invalid_argument(format!("Failed to convert graph: {}", e)))?;

            // Update working graph
            {
                let mut working = self.working_graph.lock().unwrap_or_else(|p| p.into_inner());
                *working = core_graph.clone();
            }
            core_graph
        };

        let node_count = core_graph.nodes.len() as u32;
        let connection_count = core_graph.connections.len() as u32;

        log::info!(
            "Applying graph to running config: {} nodes, {} connections",
            node_count,
            connection_count
        );

        // Replace only the graph of the *running* configuration, so runtime
        // toggles and watcher reloads are kept. PushGraph is an explicit
        // apply, so it also creates the file on a fresh installation.
        let revision = {
            let _writer = self.config_write.lock().await;
            // Like UpdateConfiguration: a graph edited from an older
            // revision would replace someone else's change unseen.
            let running = self.config.borrow().revision;
            if proto_graph.base_revision != 0 && proto_graph.base_revision != running {
                return Ok(Response::new(PushGraphResponse {
                    success: false,
                    error: Some(format!(
                        "The configuration changed since revision {} (it is now at {}); reload it and push again",
                        proto_graph.base_revision, running
                    )),
                    node_count,
                    connection_count,
                    revision: running,
                    conflict: true,
                }));
            }
            let mut file = (*self.running_config()).clone();
            file.node_graph = core_graph.clone();
            // The runtime applies the graph onto the config it runs at that
            // moment, so a concurrent toggle is not reverted.
            let revision = self
                .apply_configuration(&file, ConfigRequest::ReplaceGraph(core_graph.clone()))
                .await
                .inspect_err(|error| {
                    log::error!("Graph push failed: {}", error.message());
                })?;
            // Announced with the pushing editor's id, so it recognises its
            // own change.
            announce_configuration(
                &self.event_broadcaster,
                &self.announced_revision,
                revision,
                &proto_graph.editor_id,
            );
            revision
        };
        log::info!("Pushed graph saved and applied by the runtime");

        // Broadcast graph update event
        let event = ExecutionEvent {
            event: Some(execution_event::Event::GraphStarted(
                GraphExecutionStarted {
                    trigger_type: "GraphPush".to_string(),
                    timestamp_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64,
                },
            )),
        };
        let _ = self.event_broadcaster.send(event);

        Ok(Response::new(PushGraphResponse {
            success: true,
            error: None,
            node_count,
            connection_count,
            revision,
            conflict: false,
        }))
    }

    async fn get_configuration(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ConfigurationDocument>, Status> {
        // One snapshot: the JSON and the revision belong together.
        let snapshot = self.config.borrow().clone();
        let json = serde_json::to_string_pretty(&*snapshot.config)
            .map_err(|error| Status::internal(format!("Failed to serialize config: {error}")))?;
        Ok(Response::new(ConfigurationDocument {
            json,
            version: snapshot.config.version.clone(),
            revision: snapshot.revision,
            ..Default::default()
        }))
    }

    async fn update_configuration(
        &self,
        request: Request<ConfigurationDocument>,
    ) -> Result<Response<UpdateConfigurationResponse>, Status> {
        if gevrek_host::lock::is_locked() {
            return Err(locked_status());
        }
        let document = request.into_inner();
        let mut config = Config::from_json(&document.json).map_err(|error| {
            Status::invalid_argument(format!("Invalid configuration JSON: {error}"))
        })?;
        // The same check as loading the file. A client must not send graph
        // data that would be dropped silently, so a cleanup is an error here.
        let cleanup = config.normalize().map_err(|issue| {
            Status::invalid_argument(format!(
                "Invalid configuration at {}: {}",
                issue.path, issue.message
            ))
        })?;
        if !cleanup.is_clean() {
            return Err(Status::invalid_argument(format!(
                "Configuration contains invalid graph data ({} nodes and {} connections)",
                cleanup.removed_nodes, cleanup.removed_connections
            )));
        }
        let graph = config.node_graph.clone();
        let revision = {
            let _writer = self.config_write.lock().await;
            // Optimistic concurrency: a document edited from an older
            // revision would silently overwrite someone else's change.
            let running = self.config.borrow().revision;
            if document.base_revision != 0 && document.base_revision != running {
                return Ok(Response::new(UpdateConfigurationResponse {
                    success: false,
                    error: Some(format!(
                        "The configuration changed since revision {} (it is now at {}); reload it and apply again",
                        document.base_revision, running
                    )),
                    revision: running,
                    conflict: true,
                }));
            }
            let request = if document.base_revision == 0 {
                ConfigRequest::Replace(Box::new(config.clone()))
            } else {
                // The runtime checks again: it may have changed the
                // configuration itself (a hotkey toggle) since `running`
                // was read.
                ConfigRequest::ReplaceFrom {
                    config: Box::new(config.clone()),
                    base_revision: document.base_revision,
                }
            };
            let revision = self.apply_configuration(&config, request).await?;
            if revision == gevrek_host::config_store::REFUSED {
                let running = self.config.borrow().revision;
                return Ok(Response::new(UpdateConfigurationResponse {
                    success: false,
                    error: Some(format!(
                        "The configuration changed since revision {} (it is now at {}); reload it and apply again",
                        document.base_revision, running
                    )),
                    revision: running,
                    conflict: true,
                }));
            }
            announce_configuration(
                &self.event_broadcaster,
                &self.announced_revision,
                revision,
                &document.client_id,
            );
            revision
        };
        {
            let mut working = self.working_graph.lock().unwrap_or_else(|p| p.into_inner());
            *working = graph;
        }

        let _ = self.event_broadcaster.send(ExecutionEvent {
            event: Some(execution_event::Event::GraphStarted(
                GraphExecutionStarted {
                    trigger_type: "ConfigurationUpdate".to_string(),
                    timestamp_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64,
                },
            )),
        });
        Ok(Response::new(UpdateConfigurationResponse {
            success: true,
            error: None,
            revision,
            conflict: false,
        }))
    }

    async fn pause_execution(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<StatusResponse>, Status> {
        log::info!("Pausing execution");
        let mut paused = self
            .execution_paused
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *paused = true;

        Ok(Response::new(StatusResponse {
            success: true,
            error: None,
        }))
    }

    async fn resume_execution(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<StatusResponse>, Status> {
        log::info!("Resuming execution");
        let mut paused = self
            .execution_paused
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *paused = false;

        Ok(Response::new(StatusResponse {
            success: true,
            error: None,
        }))
    }

    async fn trigger_manual_event(
        &self,
        request: Request<TriggerRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        if gevrek_host::lock::is_locked() {
            return Err(locked_status());
        }
        let node_id = request.into_inner().node_id;
        log::info!("Triggering manual event for node: {}", node_id);

        if *self
            .execution_paused
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            return Ok(Response::new(StatusResponse {
                success: false,
                error: Some("Execution is paused".to_string()),
            }));
        }

        let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
        self.manual_command_sender
            .send(ManualCommand {
                command: RuntimeCommand::Trigger(node_id),
                response: response_sender,
            })
            .map_err(|_| Status::unavailable("Window manager runtime is not running"))?;
        // A loop that waits for work is told there is some.
        gevrek_host::wake::now();

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), response_receiver)
            .await
            .map_err(|_| Status::deadline_exceeded("Manual trigger timed out"))?
            .map_err(|_| Status::unavailable("Window manager runtime stopped before replying"))?;

        Ok(Response::new(match result {
            Ok(_) => StatusResponse {
                success: true,
                error: None,
            },
            Err(error) => StatusResponse {
                success: false,
                error: Some(error),
            },
        }))
    }

    async fn dispatch(
        &self,
        request: Request<DispatchRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        if *self
            .execution_paused
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            return Ok(Response::new(StatusResponse {
                success: false,
                error: Some("Execution is paused".to_string()),
            }));
        }

        let command = dispatch_request_to_core(request.into_inner())?;
        self.run_command(command).await
    }

    async fn inject_input(
        &self,
        request: Request<gevrek_protocol::proto::CommandDocument>,
    ) -> Result<Response<StatusResponse>, Status> {
        if !self.input_injection {
            return Err(Status::permission_denied(
                "This window manager does not take made-up input (it was not started with --test-input)",
            ));
        }
        let steps: Vec<gevrek_host::input_injection::InjectedInput> =
            serde_json::from_str(&request.into_inner().json)
                .map_err(|error| Status::invalid_argument(format!("Not input steps: {error}")))?;
        let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
        self.manual_command_sender
            .send(ManualCommand {
                command: RuntimeCommand::InjectInput(steps),
                response: response_sender,
            })
            .map_err(|_| Status::unavailable("Window manager runtime is not running"))?;
        // A loop that waits for work is told there is some.
        gevrek_host::wake::now();
        let result = tokio::time::timeout(std::time::Duration::from_secs(20), response_receiver)
            .await
            .map_err(|_| Status::deadline_exceeded("Input injection timed out"))?
            .map_err(|_| Status::unavailable("Window manager runtime stopped before replying"))?;
        Ok(Response::new(match result {
            Ok(_) => StatusResponse {
                success: true,
                error: None,
            },
            Err(error) => StatusResponse {
                success: false,
                error: Some(error),
            },
        }))
    }

    async fn dispatch_command(
        &self,
        request: Request<gevrek_protocol::proto::CommandDocument>,
    ) -> Result<Response<StatusResponse>, Status> {
        if *self
            .execution_paused
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            return Ok(Response::new(StatusResponse {
                success: false,
                error: Some("Execution is paused".to_string()),
            }));
        }
        let json = request.into_inner().json;
        let command: gevrek_model::command::DispatchCommand = serde_json::from_str(&json)
            .map_err(|error| Status::invalid_argument(format!("Not a command: {error}")))?;
        if let gevrek_model::command::DispatchCommand::Unknown(_) = command {
            return Err(Status::invalid_argument(format!(
                "This window manager does not know the command {json}"
            )));
        }
        self.run_command(command).await
    }

    async fn dispatch_batch(
        &self,
        request: Request<DispatchBatchRequest>,
    ) -> Result<Response<DispatchBatchResponse>, Status> {
        if gevrek_host::lock::is_locked() {
            return Err(locked_status());
        }
        let request = request.into_inner();
        if request.requests.is_empty() {
            return Err(Status::invalid_argument(
                "A dispatch batch must contain at least one command",
            ));
        }
        if request.requests.len() > 256 {
            return Err(Status::invalid_argument(
                "A dispatch batch cannot contain more than 256 commands",
            ));
        }

        // Validate and convert the complete request before anything reaches the runtime.
        let commands = request
            .requests
            .into_iter()
            .map(dispatch_request_to_core)
            .collect::<Result<Vec<_>, _>>()?;

        if *self
            .execution_paused
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            return Ok(Response::new(DispatchBatchResponse {
                success: false,
                results: commands
                    .iter()
                    .enumerate()
                    .map(|(index, _)| DispatchBatchItemResult {
                        index: index as u32,
                        success: false,
                        skipped: true,
                        error: Some("Execution is paused".to_string()),
                    })
                    .collect(),
            }));
        }

        let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
        self.manual_command_sender
            .send(ManualCommand {
                command: RuntimeCommand::DispatchBatch {
                    commands,
                    stop_on_error: request.stop_on_error,
                },
                response: response_sender,
            })
            .map_err(|_| Status::unavailable("Window manager runtime is not running"))?;
        // A loop that waits for work is told there is some.
        gevrek_host::wake::now();

        let result = tokio::time::timeout(std::time::Duration::from_secs(10), response_receiver)
            .await
            .map_err(|_| Status::deadline_exceeded("Dispatcher batch timed out"))?
            .map_err(|_| Status::unavailable("Window manager runtime stopped before replying"))?
            .map_err(Status::internal)?;

        let gevrek_host::command::RuntimeCommandResult::Batch(results) = result else {
            return Err(Status::internal(
                "Runtime returned a non-batch response for a batch request",
            ));
        };
        let results = results
            .into_iter()
            .enumerate()
            .map(|(index, result)| DispatchBatchItemResult {
                index: index as u32,
                success: result.success,
                skipped: result.skipped,
                error: result.error,
            })
            .collect::<Vec<_>>();
        Ok(Response::new(DispatchBatchResponse {
            success: results.iter().all(|result| result.success),
            results,
        }))
    }

    async fn query_state(&self, _request: Request<Empty>) -> Result<Response<WmState>, Status> {
        log::info!("Querying WM state");

        let paused = match self.execution_paused.try_lock() {
            Ok(paused) => *paused,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => *poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(Status::unavailable(
                    "Execution state is busy; retry the state query",
                ));
            }
        };

        // Snapshot runtime state immediately and release the shared mutex
        // before acquiring configuration state or building the protobuf
        // response. The editor polls this endpoint while the runtime publishes
        // new overlay frames, so retaining the guard here can create a lock
        // convoy (or a lock-order deadlock with another handler).
        // (Waited for, not given up on: whoever holds it holds it for
        // one copy or one assignment. Two that asked at the same moment
        // used to be told that the state is busy.)
        let overlay_data = self
            .overlay_data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();

        let active_animations = overlay_data
            .as_ref()
            .map(|data| data.active_animations as u32)
            .unwrap_or(0);
        let rules = overlay_data
            .as_ref()
            .map(|data| {
                data.rule_states
                    .iter()
                    .map(|rule| gevrek_protocol::proto::RuleState {
                        name: rule.name.clone(),
                        enabled: rule.enabled,
                        dynamic: rule.dynamic,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let scratchpads = overlay_data
            .as_ref()
            .map(|data| {
                data.scratchpad_states
                    .iter()
                    .map(|scratchpad| gevrek_protocol::proto::ScratchpadState {
                        id: scratchpad.id.clone(),
                        enabled: scratchpad.enabled,
                        window_id: scratchpad.handle.map(|handle| handle.0 as u64),
                        visible: scratchpad.visible,
                        pending_spawn: scratchpad.pending_spawn,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let startup = overlay_data
            .as_ref()
            .map(|data| {
                data.startup_states
                    .iter()
                    .map(|state| gevrek_protocol::proto::StartupState {
                        id: state.id.clone(),
                        name: state.name.clone(),
                        enabled: state.enabled,
                        status: state.status.clone(),
                        pid: state.pid,
                        restart_count: state.restart_count,
                        last_exit_code: state.last_exit_code,
                        last_error: state.last_error.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (workspaces, windows, monitors, active_workspace_id) = overlay_data
            .as_ref()
            .map(overlay_state_to_proto)
            .unwrap_or_default();

        let (animation, appearance, shadow) = {
            // A snapshot read never waits on a writer.
            let config = self.running_config();
            (
                config.desktop.animations.clone(),
                config.desktop.appearance.clone(),
                config.desktop.platform.windows.shadow,
            )
        };
        Ok(Response::new(WmState {
            workspaces,
            windows,
            monitors,
            active_workspace_id,
            execution_paused: paused,
            animations_enabled: animation.enabled,
            active_animations,
            animation_duration_ms: animation.layout.duration_ms,
            animation_curve: format!("{:?}", animation.layout.curve),
            animate_layout: animation.layout.enabled,
            animate_workspace: animation.workspace.enabled,
            animate_borders: animation.border.enabled,
            rules,
            scratchpads,
            active_binding_submap: overlay_data
                .as_ref()
                .map(|data| data.active_binding_submap.clone())
                .unwrap_or_else(|| "default".to_string()),
            tiling_suspended: overlay_data
                .as_ref()
                .map(|data| data.tiling_suspended)
                .unwrap_or(false),
            dim_inactive: appearance.dim_inactive,
            inactive_window_opacity: appearance.inactive_window_opacity as u32,
            dim_fullscreen: appearance.dim_fullscreen,
            shadow_policy: format!("{:?}", shadow),
            startup,
            animation_channels: animation
                .channels()
                .into_iter()
                .map(|(scope, channel)| AnimationChannelState {
                    scope: scope.to_string(),
                    enabled: animation.enabled && channel.enabled,
                    duration_ms: channel.duration_ms,
                    curve: format!("{:?}", channel.curve),
                    enter_style: if scope == "window" {
                        format!("{:?}", animation.window_enter_style)
                    } else {
                        String::new()
                    },
                    exit_style: if scope == "window" {
                        format!("{:?}", animation.window_exit_style)
                    } else {
                        String::new()
                    },
                })
                .collect(),
            gesture: Some(
                overlay_data
                    .as_ref()
                    .map(|data| gesture_state_to_proto(&data.gesture))
                    .unwrap_or_else(|| gevrek_protocol::GestureState {
                        backend: "Windows Precision Touchpad Raw Input".to_string(),
                        ..gevrek_protocol::GestureState::default()
                    }),
            ),
        }))
    }

    type SubscribeToEventsStream =
        tokio_stream::wrappers::ReceiverStream<Result<ExecutionEvent, Status>>;

    async fn subscribe_to_events(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<Self::SubscribeToEventsStream>, Status> {
        let subscriber_count = self.event_broadcaster.receiver_count();
        log::info!(
            "🔔 New event subscription (total subscribers: {})",
            subscriber_count + 1
        );
        let receiver = self.event_broadcaster.subscribe();

        // Convert broadcast receiver to a regular receiver for streaming
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        tokio::spawn(forward_events(receiver, tx));

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Response::new(stream))
    }

    async fn health_check(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<HealthStatus>, Status> {
        let running = self.runtime_healthy.load(Ordering::Acquire);
        let mut runtime_error = self
            .runtime_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        // The thread is alive but its loop stopped passing: commands would
        // queue and time out, so do not call that healthy.
        let stalled = gevrek_host::heartbeat::time_since_last_beat()
            .filter(|age| running && *age > RUNTIME_STALL_THRESHOLD);
        if let (Some(age), None) = (stalled, &runtime_error) {
            runtime_error = Some(format!(
                "Window manager runtime is not responding (its loop has not run for {} s)",
                age.as_secs()
            ));
        }
        let healthy = running && stalled.is_none();

        let config_error = self.current_config_load_error();
        Ok(Response::new(HealthStatus {
            healthy,
            // With the commit it was built from: what is running.
            version: gevrek_build::version(),
            uptime_seconds: self.get_uptime(),
            message: Some(runtime_error.unwrap_or_else(|| {
                let state = if running {
                    "Window manager runtime running"
                } else {
                    "Window manager runtime starting"
                };
                match config_error {
                    Some(error) => format!("{state} (read-only: {error})"),
                    None => state.to_string(),
                }
            })),
        }))
    }

    async fn get_capabilities(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        // Up to the newest command; a new one goes at the end.
        let dispatchers = (1..=DispatchType::Exec as i32)
            .filter_map(|value| DispatchType::try_from(value).ok())
            .map(|command| {
                let id = command
                    .as_str_name()
                    .trim_start_matches("DISPATCH_TYPE_")
                    .to_ascii_lowercase();
                DispatcherCapability {
                    value: command as i32,
                    id,
                    display_name: dispatcher_display_name(command),
                    argument_kind: dispatcher_argument_kind(command).to_string(),
                }
            })
            .collect();
        let node_types = self
            .node_registry
            .get_definitions()
            .iter()
            .map(|definition| definition.type_id.clone())
            .collect();
        // What every backend can do, then what this one says it can.
        let mut features: Vec<String> = [
            "typed_dispatch",
            "dispatch_batch",
            "dispatch_json",
            "state_query",
            "event_stream",
            "configuration_read_write",
            "collaborative_node_graph",
            "advanced_bindings_v8",
            "appearance_effects_v9",
            "supervised_startup_v10",
            "animation_channels_v11",
            "extension_events_v1",
            "configuration_revisions_v1",
            "graph_json_v1",
            "focus_steal_prevention",
        ]
        .iter()
        .map(|feature| feature.to_string())
        .collect();
        features.extend(self.backend.features.iter().cloned());
        Ok(Response::new(CapabilitiesResponse {
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: "2".to_string(),
            config_schema_version: DESKTOP_CONFIG_SCHEMA_VERSION,
            platform: self.backend.platform.clone(),
            backend: self.backend.backend.clone(),
            features,
            dispatchers,
            node_types,
        }))
    }

    async fn get_node_definitions(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<GetNodeDefinitionsResponse>, Status> {
        log::info!("Sending node definitions to editor");

        let definitions = self.node_registry.get_definitions().to_vec();

        Ok(Response::new(GetNodeDefinitionsResponse {
            definitions,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn get_current_graph(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<NodeGraph>, Status> {
        log::info!("Sending current node graph to editor");

        let config = self.running_config();
        let proto_graph = gevrek_protocol::conversions::node_graph_to_proto(&config.node_graph);

        log::info!(
            "Sending graph with {} nodes, {} connections",
            proto_graph.node_count,
            proto_graph.connection_count
        );

        Ok(Response::new(proto_graph))
    }

    async fn update_working_graph(
        &self,
        request: Request<NodeGraph>,
    ) -> Result<Response<StatusResponse>, Status> {
        if gevrek_host::lock::is_locked() {
            return Err(locked_status());
        }
        let proto_graph = request.into_inner();

        let node_count = proto_graph.node_count;
        let connection_count = proto_graph.connection_count;
        let editor_id = proto_graph.editor_id.clone();

        log::info!(
            "Received UpdateWorkingGraph from editor {}: {} nodes, {} connections",
            editor_id,
            node_count,
            connection_count
        );

        // Convert protobuf NodeGraph to core NodeGraph
        let core_graph = gevrek_protocol::conversions::proto_to_node_graph(&proto_graph)
            .map_err(|e| Status::invalid_argument(format!("Failed to convert graph: {}", e)))?;

        // A draft made from an older revision would put the graph of that
        // revision back into the shared draft, over what runs now. Like
        // PushGraph, 0 is the unconditional form.
        let running = self.config.borrow().revision;
        if proto_graph.base_revision != 0 && proto_graph.base_revision != running {
            log::info!(
                "Refused the working graph of editor {}: made from revision {}, the WM is at {}",
                editor_id,
                proto_graph.base_revision,
                running
            );
            return Ok(Response::new(StatusResponse {
                success: false,
                error: Some(format!(
                    "The configuration changed since revision {} (it is now at {}); the draft was not shared",
                    proto_graph.base_revision, running
                )),
            }));
        }

        // Update the working graph
        {
            let mut working = self.working_graph.lock().unwrap_or_else(|p| p.into_inner());
            *working = core_graph;
        }

        // Broadcast working graph update event with editor_id
        // The receiving editors will filter out their own updates
        let event = ExecutionEvent {
            event: Some(execution_event::Event::WorkingGraphUpdated(
                WorkingGraphUpdated {
                    timestamp_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64,
                    editor_id: editor_id.clone(),
                },
            )),
        };

        let subscriber_count = self.event_broadcaster.receiver_count();
        match self.event_broadcaster.send(event) {
            Ok(_) => {
                log::info!(
                    "📡 Working graph update from editor {} broadcast to {} subscribers",
                    editor_id,
                    subscriber_count
                );
            }
            Err(e) => {
                log::warn!(
                    "Failed to broadcast event: {} (subscribers: {})",
                    e,
                    subscriber_count
                );
            }
        }

        Ok(Response::new(StatusResponse {
            success: true,
            error: None,
        }))
    }

    async fn get_working_graph(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<NodeGraph>, Status> {
        log::info!("Sending working node graph to editor");

        let working = self.working_graph.lock().unwrap_or_else(|p| p.into_inner());
        let proto_graph = gevrek_protocol::conversions::node_graph_to_proto(&working);

        log::info!(
            "Sending working graph with {} nodes, {} connections",
            proto_graph.node_count,
            proto_graph.connection_count
        );

        Ok(Response::new(proto_graph))
    }

    async fn get_overlay_data(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<OverlayDataResponse>, Status> {
        log::info!("GetOverlayData RPC called");

        // Snapshot each shared state independently. The editor polls this RPC,
        // so never retain the high-frequency overlay guard while waiting for
        // configuration state.
        // (Waited for, not given up on: whoever holds it holds it for
        // one copy or one assignment. Two that asked at the same moment
        // used to be told that the state is busy.)
        let overlay_data = self
            .overlay_data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let appearance = self.running_config().desktop.appearance.clone();
        let shadow = self.running_config().desktop.platform.windows.shadow;

        if let Some(data) = overlay_data.as_ref() {
            // Convert core OverlayData to proto OverlayDataResponse
            let timestamp_ms = data
                .timestamp
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;

            let monitors = data
                .monitors
                .iter()
                .map(|m| MonitorOverlayData {
                    name: m.name.clone(),
                    output_id: m.handle.0 as u64,
                    connector: m.connector.clone(),
                    is_primary: m.is_primary,
                    position: Some(Position {
                        x: m.x as f32,
                        y: m.y as f32,
                    }),
                    size: Some(Size {
                        width: m.width,
                        height: m.height,
                    }),
                    work_position: Some(Position {
                        x: m.work_x as f32,
                        y: m.work_y as f32,
                    }),
                    work_size: Some(Size {
                        width: m.work_width,
                        height: m.work_height,
                    }),
                    reserved_left: m.reserved_left,
                    reserved_top: m.reserved_top,
                    reserved_right: m.reserved_right,
                    reserved_bottom: m.reserved_bottom,
                    workspace: Some(workspace_overlay_to_proto(&m.workspace)),
                    workspaces: m
                        .workspaces
                        .iter()
                        .map(workspace_overlay_to_proto)
                        .collect(),
                    active_workspace_id: m.workspace.id as u32,
                    is_focused: m.is_focused,
                    bar_position: m.bar.map(|(x, y, _, _)| Position {
                        x: x as f32,
                        y: y as f32,
                    }),
                    bar_size: m.bar.map(|(_, _, width, height)| Size { width, height }),
                    scale: m.scale,
                    refresh_millihertz: m.detail.refresh_millihertz,
                    modes: m
                        .detail
                        .modes
                        .iter()
                        .map(|mode| gevrek_protocol::proto::OutputModeInfo {
                            width: mode.width,
                            height: mode.height,
                            refresh_millihertz: mode.refresh_millihertz,
                            preferred: mode.preferred,
                            current: mode.current,
                        })
                        .collect(),
                })
                .collect();

            Ok(Response::new(OverlayDataResponse {
                timestamp_ms,
                monitors,
                graph_node_count: data.graph_node_count as u32,
                total_windows: data.total_windows as u32,
                event_loop_fps: data.event_loop_fps,
                last_event: data.last_event.clone(),
                scratchpads: data
                    .scratchpad_states
                    .iter()
                    .map(|scratchpad| gevrek_protocol::proto::ScratchpadState {
                        id: scratchpad.id.clone(),
                        enabled: scratchpad.enabled,
                        window_id: scratchpad.handle.map(|handle| handle.0 as u64),
                        visible: scratchpad.visible,
                        pending_spawn: scratchpad.pending_spawn,
                    })
                    .collect(),
                active_binding_submap: data.active_binding_submap.clone(),
                dim_inactive: appearance.dim_inactive,
                inactive_window_opacity: appearance.inactive_window_opacity as u32,
                dim_fullscreen: appearance.dim_fullscreen,
                shadow_policy: format!("{:?}", shadow),
                startup: data
                    .startup_states
                    .iter()
                    .map(|state| gevrek_protocol::proto::StartupState {
                        id: state.id.clone(),
                        name: state.name.clone(),
                        enabled: state.enabled,
                        status: state.status.clone(),
                        pid: state.pid,
                        restart_count: state.restart_count,
                        last_exit_code: state.last_exit_code,
                        last_error: state.last_error.clone(),
                    })
                    .collect(),
                gesture: Some(gesture_state_to_proto(&data.gesture)),
                notices: data
                    .notices
                    .iter()
                    .map(|notice| gevrek_protocol::proto::Notice {
                        id: notice.id.clone(),
                        title: notice.title.clone(),
                        text: notice.text.clone(),
                    })
                    .collect(),
                accent_rgb: data.accent.map(|(red, green, blue)| {
                    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
                }),
                keyboard_layout: data.keyboard_layout.as_ref().map(|layout| {
                    gevrek_protocol::proto::KeyboardLayoutState {
                        short: layout.short.clone(),
                        name: layout.name.clone(),
                        index: layout.index,
                        count: layout.count,
                    }
                }),
                frames: data
                    .frames
                    .iter()
                    .map(|frames| gevrek_protocol::proto::DisplayFrames {
                        output: frames.output.clone(),
                        direct: frames.direct,
                        composed: frames.composed,
                        direct_now: frames.direct_now,
                    })
                    .collect(),
            }))
        } else {
            // No data available yet
            Ok(Response::new(OverlayDataResponse {
                timestamp_ms: 0,
                monitors: vec![],
                graph_node_count: 0,
                total_windows: 0,
                event_loop_fps: 0.0,
                last_event: None,
                scratchpads: vec![],
                active_binding_submap: "default".to_string(),
                dim_inactive: false,
                inactive_window_opacity: 255,
                dim_fullscreen: false,
                shadow_policy: "Default".to_string(),
                startup: vec![],
                gesture: Some(gevrek_protocol::GestureState {
                    backend: "Windows Precision Touchpad Raw Input".to_string(),
                    ..gevrek_protocol::GestureState::default()
                }),
                notices: vec![],
                accent_rgb: None,
                keyboard_layout: None,
                frames: vec![],
            }))
        }
    }

    async fn shutdown(
        &self,
        request: Request<ShutdownRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let req = request.into_inner();
        log::info!(
            "Received shutdown request (save_config: {}, delay_ms: {})",
            req.save_config,
            req.delay_ms
        );

        // Optionally save config before shutdown (only if we have a config file)
        let read_only = req.save_config && self.ensure_config_writable().is_err();
        if read_only {
            log::warn!("Skipping config save - the config file failed to parse (read-only)");
        } else if req.save_config && self.has_config_file {
            let config = self.running_config();
            if let Err(e) = config.save(&self.config_path) {
                log::error!("Failed to save config before shutdown: {}", e);
                return Ok(Response::new(StatusResponse {
                    success: false,
                    error: Some(format!("Failed to save config: {}", e)),
                }));
            }
            log::info!("Config saved successfully before shutdown");
        } else if req.save_config && !self.has_config_file {
            log::info!("Skipping config save - running with defaults (no config file)");
        }

        let stop_flag = Arc::clone(&self.stop_flag);
        if req.delay_ms > 0 {
            let delay = std::time::Duration::from_millis(req.delay_ms as u64);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                *stop_flag.lock().unwrap_or_else(|p| p.into_inner()) = true;
                log::info!("Shutdown delay elapsed; stop flag set");
            });
        } else {
            *stop_flag.lock().unwrap_or_else(|p| p.into_inner()) = true;
            log::info!("Stop flag set");
        }

        Ok(Response::new(StatusResponse {
            success: true,
            error: None,
        }))
    }
}

/// Called by the WM loop with every line the automation graph logs.
pub type LogCallback = Arc<dyn Fn(String, gevrek_model::graph::LogLevel, i64) + Send + Sync>;
/// Called by the WM loop when a node runs and for runtime events.
pub type NodeEventCallback = Arc<dyn Fn(gevrek_engine::executor::NodeExecutionEvent) + Send + Sync>;

/// The callbacks a WM loop is given so that what it logs and what its
/// nodes do reaches the subscribers of the event stream.
pub fn event_callbacks(
    event_broadcaster: &Arc<broadcast::Sender<ExecutionEvent>>,
) -> (LogCallback, NodeEventCallback) {
    // Create log broadcast callback for Runtime
    let log_event_broadcaster = Arc::clone(event_broadcaster);
    let log_broadcast_callback: LogCallback = Arc::new(
        move |message: String, level: gevrek_model::graph::LogLevel, timestamp_ms: i64| {
            use gevrek_protocol::proto::execution_event::Event;
            use gevrek_protocol::proto::{ExecutionEvent, LogEvent, LogLevel};

            // Convert core LogLevel to proto LogLevel
            let proto_level = match level {
                gevrek_model::graph::LogLevel::Debug => LogLevel::Debug,
                gevrek_model::graph::LogLevel::Info => LogLevel::Info,
                gevrek_model::graph::LogLevel::Warning => LogLevel::Warn,
                gevrek_model::graph::LogLevel::Error => LogLevel::Error,
            };

            let event = ExecutionEvent {
                event: Some(Event::LogEvent(LogEvent {
                    message,
                    level: proto_level as i32,
                    timestamp_ms,
                })),
            };

            let _ = log_event_broadcaster.send(event);
        },
    );

    let node_event_broadcaster = Arc::clone(event_broadcaster);
    let node_event_callback: NodeEventCallback = Arc::new(move |event| {
        use gevrek_engine::executor::NodeExecutionEvent as CoreEvent;
        use gevrek_protocol::proto::execution_event::Event;

        let event = match event {
            CoreEvent::Started {
                node_id,
                node_type,
                timestamp_ms,
            } => Event::NodeStarted(NodeExecutionStarted {
                node_id: node_id.to_string(),
                node_type: gevrek_protocol::conversions::node_type_to_proto(&node_type) as i32,
                timestamp_ms,
            }),
            CoreEvent::Completed {
                node_id,
                node_type,
                timestamp_ms,
                duration_ms,
            } => Event::NodeCompleted(NodeExecutionCompleted {
                node_id: node_id.to_string(),
                node_type: gevrek_protocol::conversions::node_type_to_proto(&node_type) as i32,
                timestamp_ms,
                duration_ms,
                outputs: std::collections::HashMap::new(),
            }),
            CoreEvent::Failed {
                node_id,
                node_type,
                timestamp_ms,
                error,
            } => Event::NodeFailed(NodeExecutionFailed {
                node_id: node_id.to_string(),
                node_type: gevrek_protocol::conversions::node_type_to_proto(&node_type) as i32,
                timestamp_ms,
                error,
            }),
            CoreEvent::Runtime(runtime_event) => {
                Event::RuntimeEvent(gevrek_protocol::proto::RuntimeEvent {
                    kind: runtime_event.kind.as_str().to_string(),
                    timestamp_ms: runtime_event.timestamp_ms,
                    window_id: runtime_event.window_handle.map(|handle| handle.0 as u64),
                    fields: runtime_event.fields.into_iter().collect(),
                })
            }
        };

        let _ = node_event_broadcaster.send(ExecutionEvent { event: Some(event) });
    });

    (log_broadcast_callback, node_event_callback)
}

/// Load the configuration the WM starts with. Returns it with whether it
/// came from the file, and the reason the file could not be read if it
/// exists but does not parse (the WM then runs read-only on the defaults).
pub fn load_config(config_path: &Path) -> (Config, bool, Option<String>) {
    let mut config_load_error = None;

    // Load config if it exists, otherwise use sane defaults without creating a file
    // The WM runs perfectly fine with defaults - the editor is only needed for customization
    let (config, has_config_file) = if config_path.exists() {
        log::info!("Found config file: {}", config_path.display());
        match Config::load(config_path) {
            Ok(mut config) => {
                log::info!(
                    "Loaded configuration with {} nodes, {} connections",
                    config.node_graph.nodes.len(),
                    config.node_graph.connections.len()
                );

                // If the loaded config has no nodes, use defaults (but don't overwrite the file)
                if config.node_graph.nodes.is_empty() {
                    log::warn!("Config file has no nodes, using default graph instead");
                    config = Config::with_default_graph();
                    log::info!(
                        "Using default graph: {} nodes, {} connections",
                        config.node_graph.nodes.len(),
                        config.node_graph.connections.len()
                    );
                }

                (config, true)
            }
            Err(e) => {
                log::error!(
                    "Failed to parse config file ({}); running read-only with defaults.                      The file will not be written until it parses again.",
                    e
                );
                config_load_error = Some(config_load_error_message(config_path, &e));
                let config = Config::with_default_graph();
                log::info!(
                    "Using default graph: {} nodes, {} connections",
                    config.node_graph.nodes.len(),
                    config.node_graph.connections.len()
                );
                (config, false)
            }
        }
    } else {
        log::info!(
            "No config file found, running with the starting configuration (the example Aurora)"
        );
        let mut config = gevrek_model::examples::starting_configuration();
        if gevrek_tutorial::first_launch(false) {
            // Nobody was asked yet: windows stay where the person has
            // them until the welcome is answered.
            log::info!("The first launch: every workspace floats until the welcome is answered");
            gevrek_tutorial::float_first(&mut config);
        }
        log::info!(
            "Default graph: {} nodes, {} connections",
            config.node_graph.nodes.len(),
            config.node_graph.connections.len()
        );
        log::info!(
            "Input bindings are loaded from the active schema-v7 configuration and automation graph"
        );
        (config, false)
    };
    (config, has_config_file, config_load_error)
}

/// Serve the WM over gRPC until the stop flag is set.
pub async fn serve(
    addr: SocketAddr,
    wm_server: WMServer,
    stop_flag: Arc<Mutex<bool>>,
) -> Result<()> {
    serve_on(
        gevrek_protocol::Endpoint::Tcp(format!("http://{addr}")),
        wm_server,
        stop_flag,
    )
    .await
}

/// Serve the WM on an endpoint (`gevrek_protocol::endpoint`): the local one
/// of an instance, or a TCP address, until the stop flag is set.
pub async fn serve_on(
    endpoint: gevrek_protocol::Endpoint,
    wm_server: WMServer,
    stop_flag: Arc<Mutex<bool>>,
) -> Result<()> {
    let (told, mut asked) = tokio::sync::watch::channel(false);
    let stopped = async move {
        loop {
            if *stop_flag
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let _ = told.send(true);
    };
    // Asked to stop, the server lets every call end that is under way,
    // and a client that listens to what happens (the shell, the settings
    // program) never ends its call: the server waited for it for good,
    // the program stayed with its endpoint taken, and whoever had asked
    // it to stop ended it by force. So it waits a moment, and no longer.
    let waited_enough = async move {
        while !*asked.borrow() {
            if asked.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    };
    tokio::pin!(waited_enough);
    let router = Server::builder().add_service(WindowManagerServer::new(wm_server));
    match endpoint {
        gevrek_protocol::Endpoint::Local(instance) => {
            let incoming = gevrek_protocol::endpoint::bind_local(&instance)?;
            log::info!(
                "Listening on {}",
                gevrek_protocol::Endpoint::Local(instance.clone()).describe()
            );
            let served = tokio::select! {
                served = router.serve_with_incoming_shutdown(incoming, stopped) => served,
                () = &mut waited_enough => {
                    log::info!("Stopping though a client still listens");
                    Ok(())
                }
            };
            gevrek_protocol::endpoint::release_local(&instance);
            served?;
        }
        gevrek_protocol::Endpoint::Tcp(url) => {
            let addr: SocketAddr = url
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .parse()
                .map_err(|error| anyhow::anyhow!("{url} is not ip:port: {error}"))?;
            log::warn!(
                "Listening on TCP {addr}: every local program of every user can control this window manager there"
            );
            tokio::select! {
                served = router.serve_with_shutdown(addr, stopped) => served?,
                () = &mut waited_enough => {
                    log::info!("Stopping though a client still listens");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod overlay_conversion_tests {
    use super::*;
    use gevrek_engine::outputs::MonitorHandle;
    use gevrek_engine::overlay_data::{
        DetailedWindowInfo as CoreDetailedWindowInfo, GestureStateData, MonitorData, OverlayData,
        WorkspaceData,
    };
    use gevrek_engine::WindowHandle;

    fn workspace(id: usize, name: &str) -> WorkspaceData {
        WorkspaceData {
            id,
            name: name.to_string(),
            layout_type: "Dwindle".to_string(),
            total_windows: 0,
            tiled_windows: 0,
            floating_windows: 0,
            container_count: 0,
            dwindle_direction: "alternate".to_string(),
            dwindle_preserve_split: true,
            pseudotiled_windows: 0,
            master_position: "Left".to_string(),
            new_window_placement: "StackEnd".to_string(),
            tiling_enabled: true,
            float_override: false,
            window_container_behaviour: "Column".to_string(),
            windows: Vec::new(),
            detailed_windows: Vec::new(),
        }
    }

    #[test]
    fn state_snapshot_includes_inactive_workspaces_and_their_windows() {
        let active = workspace(10, "code");
        let mut inactive = workspace(11, "notes");
        inactive.layout_type = "Monocle".to_string();
        inactive.total_windows = 1;
        inactive.floating_windows = 1;
        inactive.detailed_windows.push(CoreDetailedWindowInfo {
            handle: WindowHandle(99),
            title: "Hidden notes".to_string(),
            class: "Notepad".to_string(),
            exe: "notepad.exe".to_string(),
            app_id: String::new(),
            icon: String::new(),
            x: 40,
            y: 50,
            width: 800,
            height: 600,
            is_focused: false,
            is_minimized: false,
            is_maximized: false,
            is_managed: true,
            is_tiled: false,
            is_floating: true,
            has_border: true,
            is_border_window: false,
            workspace_id: 11,
            tags: vec!["notes".to_string()],
            matched_rules: vec!["Notes float".to_string()],
            effective_rule_effects: vec!["floating".to_string()],
            effective_opacity: 230,
            is_dimmed: true,
            shadow_policy: "Enabled".to_string(),
        });
        let overlay = OverlayData {
            timestamp: SystemTime::now(),
            monitors: vec![MonitorData {
                scale: 1.0,
                detail: Default::default(),
                handle: MonitorHandle(1),
                name: "Test monitor".to_string(),
                connector: "TEST-1".to_string(),
                is_primary: true,
                is_focused: true,
                workspace: active.clone(),
                workspaces: vec![active, inactive.clone()],
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                work_x: 0,
                work_y: 0,
                work_width: 1920,
                work_height: 1040,
                reserved_left: 0,
                reserved_top: 0,
                reserved_right: 0,
                reserved_bottom: 40,
                bar: Some((6, 6, 1908, 36)),
            }],
            graph_node_count: 0,
            total_windows: 1,
            event_loop_fps: 60.0,
            last_event: None,
            active_animations: 0,
            active_binding_submap: "default".to_string(),
            tiling_suspended: false,
            gesture: GestureStateData::default(),
            rule_states: Vec::new(),
            scratchpad_states: Vec::new(),
            startup_states: Vec::new(),
            metrics: None,
            notices: Vec::new(),
            accent: None,
            keyboard_layout: None,
            frames: Vec::new(),
        };

        let (workspaces, windows, monitors, active_workspace_id) = overlay_state_to_proto(&overlay);
        assert_eq!(workspaces.len(), 2);
        assert_eq!(windows.len(), 1);
        assert_eq!(monitors.len(), 1);
        assert!(monitors[0].is_focused);
        assert_eq!(active_workspace_id.as_deref(), Some("10"));
        assert_eq!(workspaces[1].window_ids, vec!["99"]);
        assert_eq!(windows[0].title, "Hidden notes");

        let inactive_overlay = workspace_overlay_to_proto(&inactive);
        assert_eq!(inactive_overlay.detailed_windows.len(), 1);
        assert_eq!(inactive_overlay.detailed_windows[0].effective_opacity, 230);
    }

    #[tokio::test]
    async fn a_lagging_event_subscriber_is_told_and_keeps_streaming() {
        let (events, receiver) = broadcast::channel(2);
        let log = |n: i64| ExecutionEvent {
            event: Some(execution_event::Event::LogEvent(
                gevrek_protocol::proto::LogEvent {
                    message: n.to_string(),
                    level: 0,
                    timestamp_ms: n,
                },
            )),
        };
        for n in 0..5 {
            events.send(log(n)).unwrap();
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(forward_events(receiver, tx));

        let first = rx.recv().await.unwrap().unwrap();
        assert!(matches!(
            first.event,
            Some(execution_event::Event::EventsDropped(ref dropped)) if dropped.count == 3
        ));
        assert_eq!(rx.recv().await.unwrap().unwrap(), log(3));
        assert_eq!(rx.recv().await.unwrap().unwrap(), log(4));
        // Still subscribed after the lag.
        events.send(log(5)).unwrap();
        assert_eq!(rx.recv().await.unwrap().unwrap(), log(5));
    }

    /// Stands in for the runtime: applies every replacement by publishing it.
    fn spawn_fake_runtime(
        receiver: std::sync::mpsc::Receiver<ConfigRequest>,
        publisher: gevrek_host::config_store::ConfigPublisher,
    ) {
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let config = request.resolve(&publisher.current());
                publisher.publish_request(&config);
            }
        });
    }

    #[tokio::test]
    async fn the_running_configuration_is_what_clients_read_and_push_onto() {
        let config_path = std::env::temp_dir().join(format!(
            "gevrek-config-owner-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (config_sender, config_receiver) = std::sync::mpsc::channel();
        let (manual_sender, _manual_receiver) = std::sync::mpsc::channel();
        let (events, _) = tokio::sync::broadcast::channel(8);
        let (publisher, config_watch) = gevrek_host::config_store::config_channel(Config::new());

        // A runtime toggle (e.g. ToggleStackbar) changes the running config.
        let mut toggled = Config::new();
        toggled.desktop.appearance.stackbar.enabled = !toggled.desktop.appearance.stackbar.enabled;
        let stackbar = toggled.desktop.appearance.stackbar.enabled;
        publisher.publish(&toggled);
        spawn_fake_runtime(config_receiver, publisher);

        let server = WMServer::new(
            ConfigSource {
                config: config_watch,
                config_path: config_path.clone(),
                has_config_file: false,
                config_load_error: None,
            },
            RuntimeLinks {
                runtime_config_sender: config_sender,
                manual_command_sender: manual_sender,
                execution_paused: Arc::new(Mutex::new(false)),
                stop_flag: Arc::new(Mutex::new(false)),
                overlay_data: Arc::new(Mutex::new(None)),
                event_broadcaster: Arc::new(events),
                runtime_healthy: Arc::new(AtomicBool::new(true)),
                runtime_error: Arc::new(Mutex::new(None)),
            },
        );
        let read = |server: &WMServer| -> Config {
            serde_json::from_str(&server.running_config_json()).unwrap()
        };
        assert_eq!(read(&server).desktop.appearance.stackbar.enabled, stackbar);

        // Pushing a graph replaces only the graph of the running config.
        let mut graph = gevrek_model::graph::NodeGraph::new();
        graph.add_node(gevrek_model::graph::NodeType::OnStartup, [0.0, 0.0]);
        server
            .push_graph(Request::new(
                gevrek_protocol::conversions::node_graph_to_proto(&graph),
            ))
            .await
            .unwrap();
        let after = read(&server);
        assert_eq!(after.node_graph.nodes.len(), 1);
        assert_eq!(
            after.desktop.appearance.stackbar.enabled, stackbar,
            "a graph push reverted a runtime toggle"
        );
        assert_eq!(
            Config::load(&config_path)
                .unwrap()
                .desktop
                .appearance
                .stackbar
                .enabled,
            stackbar
        );
        let _ = std::fs::remove_file(&config_path);
    }

    #[tokio::test]
    async fn unparsable_config_file_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!(
            "gevrek-readonly-config-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");
        let broken = "{ \"schema_version\": 7, oops";
        std::fs::write(&config_path, broken).unwrap();
        let load_error = Config::load(&config_path).unwrap_err();

        let (config_sender, config_receiver) = std::sync::mpsc::channel();
        let (manual_sender, _manual_receiver) = std::sync::mpsc::channel();
        let (events, _) = tokio::sync::broadcast::channel(8);
        let (publisher, config_watch) =
            gevrek_host::config_store::config_channel(Config::with_default_graph());
        spawn_fake_runtime(config_receiver, publisher);
        let server = WMServer::new(
            ConfigSource {
                config: config_watch,
                config_path: config_path.clone(),
                has_config_file: false,
                config_load_error: Some(config_load_error_message(&config_path, &load_error)),
            },
            RuntimeLinks {
                runtime_config_sender: config_sender,
                manual_command_sender: manual_sender,
                execution_paused: Arc::new(Mutex::new(false)),
                stop_flag: Arc::new(Mutex::new(false)),
                overlay_data: Arc::new(Mutex::new(None)),
                event_broadcaster: Arc::new(events),
                runtime_healthy: Arc::new(AtomicBool::new(true)),
                runtime_error: Arc::new(Mutex::new(None)),
            },
        );
        let document = || {
            Request::new(ConfigurationDocument {
                json: serde_json::to_string(&Config::with_default_graph()).unwrap(),
                version: String::new(),
                ..Default::default()
            })
        };

        let error = server.update_configuration(document()).await.unwrap_err();
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), broken);
        let health = server
            .health_check(Request::new(Empty {}))
            .await
            .unwrap()
            .into_inner();
        assert!(health.healthy);
        assert!(health.message.unwrap().contains("read-only"));

        // Once the user repairs the file, applying works again without a restart.
        Config::with_default_graph().save(&config_path).unwrap();
        server.update_configuration(document()).await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn overlay_rpc_is_not_blocked_by_a_configuration_write() {
        let (config_sender, _config_receiver) = std::sync::mpsc::channel();
        let (manual_sender, _manual_receiver) = std::sync::mpsc::channel();
        let overlay_data = Arc::new(Mutex::new(None));
        let (events, _) = tokio::sync::broadcast::channel(8);
        let (_publisher, config_watch) = gevrek_host::config_store::config_channel(Config::new());
        let server = WMServer::new(
            ConfigSource {
                config: config_watch,
                config_path: PathBuf::from("unused-test-config.json"),
                has_config_file: false,
                config_load_error: None,
            },
            RuntimeLinks {
                runtime_config_sender: config_sender,
                manual_command_sender: manual_sender,
                execution_paused: Arc::new(Mutex::new(false)),
                stop_flag: Arc::new(Mutex::new(false)),
                overlay_data: Arc::clone(&overlay_data),
                event_broadcaster: Arc::new(events),
                runtime_healthy: Arc::new(AtomicBool::new(true)),
                runtime_error: Arc::new(Mutex::new(None)),
            },
        );

        // Reads use the published snapshot, so a writer waiting on the
        // runtime (it holds `config_write`) cannot make them fail or block.
        let _writer = server.config_write.lock().await;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            server.get_overlay_data(Request::new(Empty {})),
        )
        .await
        .expect("overlay RPC must not wait for the configuration writer");
        if let Err(error) = result {
            assert!(
                !error.message().contains("Configuration"),
                "overlay RPC failed because of the configuration: {}",
                error.message()
            );
        }
        assert!(
            overlay_data.try_lock().is_ok(),
            "overlay RPC retained the live-state lock"
        );
    }

    #[test]
    fn monitor_dispatch_requests_share_the_typed_selector_parser() {
        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::FocusMonitor, "next")).unwrap(),
            DispatchCommand::FocusMonitor {
                target: MonitorTarget::Next
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(
                DispatchType::MoveWorkspaceToMonitor,
                "name:Side display"
            ))
            .unwrap(),
            DispatchCommand::MoveWorkspaceToMonitor {
                target: MonitorTarget::Name("Side display".to_string())
            }
        );
        assert!(
            dispatch_request_to_core(request(DispatchType::SwapActiveWorkspaces, "name:")).is_err()
        );

        let mut rename = request(DispatchType::RenameWorkspace, "name:web");
        rename.secondary_argument = "research".to_string();
        assert_eq!(
            dispatch_request_to_core(rename).unwrap(),
            DispatchCommand::RenameWorkspace {
                selector: WorkspaceSelector::Name("web".to_string()),
                name: "research".to_string(),
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::MoveToWorkspaceAndFollow, "next"))
                .unwrap(),
            DispatchCommand::MoveToWorkspaceAndFollow {
                selector: WorkspaceSelector::Next,
            }
        );
        let mut set_floating = request(DispatchType::SetFloating, "");
        set_floating.x = 1;
        assert_eq!(
            dispatch_request_to_core(set_floating).unwrap(),
            DispatchCommand::SetFloating { enabled: true }
        );
        let mut invalid_boolean = request(DispatchType::SetPinned, "");
        invalid_boolean.x = 2;
        assert!(dispatch_request_to_core(invalid_boolean).is_err());
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::CenterActive, "")).unwrap(),
            DispatchCommand::CenterActive
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ClearWindowTags, "")).unwrap(),
            DispatchCommand::ClearWindowTags
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::FocusWindow, "class:WindowsTerminal"))
                .unwrap(),
            DispatchCommand::FocusWindow {
                selector: WindowSelector::Class("WindowsTerminal".to_string())
            }
        );
        assert!(dispatch_request_to_core(request(DispatchType::FocusWindow, "title:")).is_err());
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::FocusLast, "")).unwrap(),
            DispatchCommand::FocusLast
        );
        let mut move_window = request(DispatchType::MoveWindow, "tag:scratch");
        move_window.x = 40;
        move_window.y = -20;
        move_window.secondary_argument = "relative".to_string();
        assert_eq!(
            dispatch_request_to_core(move_window).unwrap(),
            DispatchCommand::MoveWindow {
                selector: WindowSelector::Tag("scratch".to_string()),
                x: 40,
                y: -20,
                relative: true,
            }
        );
        let mut set_floating = request(DispatchType::SetWindowFloating, "class:Dialog");
        set_floating.x = 1;
        assert_eq!(
            dispatch_request_to_core(set_floating).unwrap(),
            DispatchCommand::SetWindowFloating {
                selector: WindowSelector::Class("Dialog".to_string()),
                enabled: true,
            }
        );
        let mut invalid_mode = request(DispatchType::ResizeWindow, "tag:scratch");
        invalid_mode.secondary_argument = "diagonal".to_string();
        assert!(dispatch_request_to_core(invalid_mode).is_err());
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleWindowFullscreen, "pid:4242"))
                .unwrap(),
            DispatchCommand::ToggleWindowFullscreen {
                selector: WindowSelector::ProcessId(4242),
            }
        );
        let mut tag = request(DispatchType::ToggleSelectedWindowTag, "tag:communication");
        tag.secondary_argument = "urgent".to_string();
        assert_eq!(
            dispatch_request_to_core(tag).unwrap(),
            DispatchCommand::ToggleSelectedWindowTag {
                selector: WindowSelector::Tag("communication".to_string()),
                tag: "urgent".to_string(),
            }
        );
        let invalid_tag = request(DispatchType::AddSelectedWindowTag, "class:Dialog");
        assert!(dispatch_request_to_core(invalid_tag).is_err());
        let mut move_selected = request(DispatchType::MoveSelectedToWorkspaceAndFollow, "pid:4242");
        move_selected.secondary_argument = "name:chat".to_string();
        assert_eq!(
            dispatch_request_to_core(move_selected).unwrap(),
            DispatchCommand::MoveWindowToWorkspaceAndFollow {
                window: WindowSelector::ProcessId(4242),
                workspace: WorkspaceSelector::Name("chat".to_string()),
            }
        );
        let mut cycle_group = request(DispatchType::CycleWindowGroup, "tag:tabs");
        cycle_group.x = 1;
        assert_eq!(
            dispatch_request_to_core(cycle_group).unwrap(),
            DispatchCommand::CycleWindowGroup {
                selector: WindowSelector::Tag("tabs".to_string()),
                forward: true,
            }
        );
        let mut focus_group = request(DispatchType::FocusWindowGroupIndex, "class:Browser");
        focus_group.x = 2;
        assert_eq!(
            dispatch_request_to_core(focus_group).unwrap(),
            DispatchCommand::FocusWindowGroupIndex {
                selector: WindowSelector::Class("Browser".to_string()),
                index: 2,
            }
        );
        let mut invalid_group_index = request(DispatchType::FocusWindowGroupIndex, "class:Browser");
        invalid_group_index.x = -1;
        assert!(dispatch_request_to_core(invalid_group_index).is_err());
    }

    #[test]
    fn tiling_and_monitor_move_dispatch_requests_parse() {
        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        let mut silent = request(DispatchType::MoveWindowToMonitor, "active");
        silent.secondary_argument = "next".to_string();
        assert_eq!(
            dispatch_request_to_core(silent).unwrap(),
            DispatchCommand::MoveWindowToMonitor {
                window: WindowSelector::Active,
                target: MonitorTarget::Next,
            }
        );
        let mut follow = request(DispatchType::MoveWindowToMonitorAndFollow, "handle:42");
        follow.secondary_argument = "name:Side display".to_string();
        assert_eq!(
            dispatch_request_to_core(follow).unwrap(),
            DispatchCommand::MoveWindowToMonitorAndFollow {
                window: WindowSelector::Handle(42),
                target: MonitorTarget::Name("Side display".to_string()),
            }
        );
        let mut invalid_target = request(DispatchType::MoveWindowToMonitor, "active");
        invalid_target.secondary_argument = "name:".to_string();
        assert!(dispatch_request_to_core(invalid_target).is_err());

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::Retile, "")).unwrap(),
            DispatchCommand::Retile
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleTiling, "")).unwrap(),
            DispatchCommand::ToggleTiling
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleWorkspaceTiling, "name:code"))
                .unwrap(),
            DispatchCommand::ToggleWorkspaceTiling {
                selector: WorkspaceSelector::Name("code".to_string()),
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleWorkspaceFloatOverride, "2"))
                .unwrap(),
            DispatchCommand::ToggleWorkspaceFloatOverride {
                selector: WorkspaceSelector::Index(2),
            }
        );
        assert!(
            dispatch_request_to_core(request(DispatchType::ToggleWorkspaceTiling, "name:"))
                .is_err()
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleTitleBar, "")).unwrap(),
            DispatchCommand::ToggleTitleBar
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleWindowTitleBar, "pid:4242"))
                .unwrap(),
            DispatchCommand::ToggleWindowTitleBar {
                selector: WindowSelector::ProcessId(4242),
            }
        );
    }

    #[test]
    fn layout_depth_dispatch_requests_parse() {
        use gevrek_engine::layouts::{LayoutAxis, Sizing};

        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        let mut edge = request(DispatchType::ResizeEdge, "increase");
        edge.direction = Direction::Right as i32;
        assert_eq!(
            dispatch_request_to_core(edge).unwrap(),
            DispatchCommand::ResizeEdge {
                edge: CoreDirection::Right,
                sizing: Sizing::Increase,
            }
        );
        let mut invalid_edge = request(DispatchType::ResizeEdge, "sideways");
        invalid_edge.direction = Direction::Left as i32;
        assert!(dispatch_request_to_core(invalid_edge).is_err());
        let mut missing_direction = request(DispatchType::ResizeEdge, "increase");
        missing_direction.direction = Direction::Unspecified as i32;
        assert!(dispatch_request_to_core(missing_direction).is_err());

        let mut axis = request(DispatchType::ResizeAxis, "horizontal-and-vertical");
        axis.secondary_argument = "decrease".to_string();
        assert_eq!(
            dispatch_request_to_core(axis).unwrap(),
            DispatchCommand::ResizeAxis {
                axis: LayoutAxis::HorizontalAndVertical,
                sizing: Sizing::Decrease,
            }
        );
        assert!(dispatch_request_to_core(request(DispatchType::ResizeAxis, "diagonal")).is_err());

        let mut delta = request(DispatchType::SetResizeDelta, "");
        delta.x = 80;
        assert_eq!(
            dispatch_request_to_core(delta).unwrap(),
            DispatchCommand::SetResizeDelta { pixels: 80 }
        );
        let mut invalid_delta = request(DispatchType::SetResizeDelta, "");
        invalid_delta.x = 0;
        assert!(dispatch_request_to_core(invalid_delta).is_err());

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::FlipLayout, "vertical")).unwrap(),
            DispatchCommand::FlipLayout {
                axis: LayoutAxis::Vertical,
            }
        );
        assert!(dispatch_request_to_core(request(DispatchType::FlipLayout, "spin")).is_err());

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleMonocle, "")).unwrap(),
            DispatchCommand::ToggleMonocle
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleLayoutLock, "")).unwrap(),
            DispatchCommand::ToggleLayoutLock
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::StackAll, "")).unwrap(),
            DispatchCommand::StackAll
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::UnstackAll, "")).unwrap(),
            DispatchCommand::UnstackAll
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleStackbar, "")).unwrap(),
            DispatchCommand::ToggleStackbar
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::Promote, "")).unwrap(),
            DispatchCommand::Promote
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::PromoteFocus, "")).unwrap(),
            DispatchCommand::PromoteFocus
        );
        let mut promote_window = request(DispatchType::PromoteWindow, "");
        promote_window.direction = Direction::Up as i32;
        assert_eq!(
            dispatch_request_to_core(promote_window).unwrap(),
            DispatchCommand::PromoteWindow {
                direction: CoreDirection::Up,
            }
        );
    }

    #[test]
    fn focus_manage_and_restore_dispatch_requests_parse() {
        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::FocusMonitorAtCursor, "")).unwrap(),
            DispatchCommand::FocusMonitorAtCursor
        );
        let mut combined = request(DispatchType::FocusMonitorWorkspace, "1");
        combined.secondary_argument = "name:code".to_string();
        assert_eq!(
            dispatch_request_to_core(combined).unwrap(),
            DispatchCommand::FocusMonitorWorkspace {
                target: MonitorTarget::Index(1),
                workspace: WorkspaceSelector::Name("code".to_string()),
            }
        );
        let mut invalid_combined = request(DispatchType::FocusMonitorWorkspace, "1");
        invalid_combined.secondary_argument = "name:".to_string();
        assert!(dispatch_request_to_core(invalid_combined).is_err());

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleMouseFollowsFocus, "")).unwrap(),
            DispatchCommand::ToggleMouseFollowsFocus
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ManageWindow, "handle:42")).unwrap(),
            DispatchCommand::ManageWindow {
                selector: WindowSelector::Handle(42),
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::UnmanageWindow, "active")).unwrap(),
            DispatchCommand::UnmanageWindow {
                selector: WindowSelector::Active,
            }
        );
        assert!(
            dispatch_request_to_core(request(DispatchType::ManageWindow, "mystery:1")).is_err()
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::RestoreWindows, "")).unwrap(),
            DispatchCommand::RestoreWindows
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ForceFocusWindow, "handle:42")).unwrap(),
            DispatchCommand::ForceFocusWindow {
                selector: WindowSelector::Handle(42),
            }
        );
        assert!(
            dispatch_request_to_core(request(DispatchType::ForceFocusWindow, "mystery:1")).is_err()
        );
    }

    #[test]
    fn workspace_lifecycle_and_container_behaviour_dispatch_requests_parse() {
        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::NewWorkspace, "")).unwrap(),
            DispatchCommand::NewWorkspace { name: None }
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::NewWorkspace, "mail")).unwrap(),
            DispatchCommand::NewWorkspace {
                name: Some("mail".to_string()),
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::CloseWorkspace, "")).unwrap(),
            DispatchCommand::CloseWorkspace
        );

        let mut ensure = request(DispatchType::EnsureWorkspaces, "name:Side display");
        ensure.x = 3;
        assert_eq!(
            dispatch_request_to_core(ensure).unwrap(),
            DispatchCommand::EnsureWorkspaces {
                target: MonitorTarget::Name("Side display".to_string()),
                count: 3,
            }
        );
        let mut invalid_count = request(DispatchType::EnsureWorkspaces, "primary");
        invalid_count.x = 0;
        assert!(dispatch_request_to_core(invalid_count).is_err());

        let mut named = request(DispatchType::EnsureNamedWorkspace, "primary");
        named.secondary_argument = "chat".to_string();
        assert_eq!(
            dispatch_request_to_core(named).unwrap(),
            DispatchCommand::EnsureNamedWorkspace {
                target: MonitorTarget::Primary,
                name: "chat".to_string(),
                layout: None,
            }
        );
        let mut missing_name = request(DispatchType::EnsureNamedWorkspace, "primary");
        missing_name.secondary_argument = "  ".to_string();
        assert!(dispatch_request_to_core(missing_name).is_err());

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleWindowContainerBehaviour, ""))
                .unwrap(),
            DispatchCommand::ToggleWindowContainerBehaviour
        );
        assert_eq!(
            dispatch_request_to_core(request(
                DispatchType::ToggleWorkspaceWindowContainerBehaviour,
                "name:code"
            ))
            .unwrap(),
            DispatchCommand::ToggleWorkspaceWindowContainerBehaviour {
                selector: WorkspaceSelector::Name("code".to_string()),
            }
        );
    }

    #[test]
    fn resize_persistence_and_watch_dispatch_requests_parse() {
        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::SaveResize, "C:\\layouts\\m.json"))
                .unwrap(),
            DispatchCommand::SaveResize {
                path: "C:\\layouts\\m.json".to_string(),
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::LoadResize, "resume.json")).unwrap(),
            DispatchCommand::LoadResize {
                path: "resume.json".to_string(),
            }
        );
        assert!(dispatch_request_to_core(request(DispatchType::SaveResize, "")).is_err());
        assert!(dispatch_request_to_core(request(DispatchType::LoadResize, "  ")).is_err());
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::QuickSaveResize, "")).unwrap(),
            DispatchCommand::QuickSaveResize
        );
        assert_eq!(
            dispatch_request_to_core(request(DispatchType::QuickLoadResize, "")).unwrap(),
            DispatchCommand::QuickLoadResize
        );

        let mut enable_watch = request(DispatchType::WatchConfiguration, "");
        enable_watch.x = 1;
        assert_eq!(
            dispatch_request_to_core(enable_watch).unwrap(),
            DispatchCommand::WatchConfiguration { enabled: true }
        );
        let disable_watch = request(DispatchType::WatchConfiguration, "");
        assert_eq!(
            dispatch_request_to_core(disable_watch).unwrap(),
            DispatchCommand::WatchConfiguration { enabled: false }
        );
        let mut invalid_watch = request(DispatchType::WatchConfiguration, "");
        invalid_watch.x = 2;
        assert!(dispatch_request_to_core(invalid_watch).is_err());
    }

    #[test]
    fn transparency_padding_and_offset_dispatch_requests_parse() {
        let request = |command: DispatchType, argument: &str| DispatchRequest {
            command: command as i32,
            argument: argument.to_string(),
            ..DispatchRequest::default()
        };

        assert_eq!(
            dispatch_request_to_core(request(DispatchType::ToggleTransparency, "")).unwrap(),
            DispatchCommand::ToggleTransparency
        );

        let mut alpha = request(DispatchType::SetTransparencyAlpha, "");
        alpha.x = 180;
        assert_eq!(
            dispatch_request_to_core(alpha).unwrap(),
            DispatchCommand::SetTransparencyAlpha { alpha: 180 }
        );
        let mut invalid_alpha = request(DispatchType::SetTransparencyAlpha, "");
        invalid_alpha.x = 10;
        assert!(dispatch_request_to_core(invalid_alpha).is_err());

        assert_eq!(
            dispatch_request_to_core(request(
                DispatchType::SetGlobalWorkAreaOffset,
                "10,20,30,40"
            ))
            .unwrap(),
            DispatchCommand::SetGlobalWorkAreaOffset {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            }
        );
        assert_eq!(
            dispatch_request_to_core(request(
                DispatchType::SetWorkspaceWorkAreaOffset,
                "1, 2, 3, 4"
            ))
            .unwrap(),
            DispatchCommand::SetWorkspaceWorkAreaOffset {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4,
            }
        );
        assert!(dispatch_request_to_core(request(
            DispatchType::SetGlobalWorkAreaOffset,
            "10,20,30"
        ))
        .is_err());
        assert!(dispatch_request_to_core(request(
            DispatchType::SetWorkspaceWorkAreaOffset,
            "10,20,30,nope"
        ))
        .is_err());

        let mut padding = request(DispatchType::SetWorkspacePadding, "");
        padding.x = 12;
        assert_eq!(
            dispatch_request_to_core(padding).unwrap(),
            DispatchCommand::SetWorkspacePadding { padding: 12 }
        );
        let mut invalid_padding = request(DispatchType::SetWorkspacePadding, "");
        invalid_padding.x = 600;
        assert!(dispatch_request_to_core(invalid_padding).is_err());

        let mut adjust_workspace = request(DispatchType::AdjustWorkspacePadding, "");
        adjust_workspace.x = -5;
        assert_eq!(
            dispatch_request_to_core(adjust_workspace).unwrap(),
            DispatchCommand::AdjustWorkspacePadding { delta: -5 }
        );

        let mut container_padding = request(DispatchType::SetContainerPadding, "");
        container_padding.x = 8;
        assert_eq!(
            dispatch_request_to_core(container_padding).unwrap(),
            DispatchCommand::SetContainerPadding { padding: 8 }
        );
        let mut invalid_container = request(DispatchType::SetContainerPadding, "");
        invalid_container.x = -1;
        assert!(dispatch_request_to_core(invalid_container).is_err());

        let mut adjust_container = request(DispatchType::AdjustContainerPadding, "");
        adjust_container.x = 3;
        assert_eq!(
            dispatch_request_to_core(adjust_container).unwrap(),
            DispatchCommand::AdjustContainerPadding { delta: 3 }
        );
    }
}
