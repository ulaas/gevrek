//! The line to the window manager.
//!
//! Everything that waits on the window manager happens on a thread of its
//! own; the window never does. The window says what it wants
//! ([`Request`]) and is told what happened ([`Event`]), and each event
//! wakes the window so that it draws.
//!
//! The line is kept: when the window manager is not there, or goes away,
//! it is tried again every few seconds until it answers.

use gevrek_model::Config;
use gevrek_protocol::proto::execution_event::Event as Wire;
use gevrek_protocol::{ConfigurationDocument, WMClient};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// How long a call may take before it counts as unanswered. Longer than
/// the window manager's own wait for its loop (10 s), so that its answer
/// is the one shown.
const CALL: Duration = Duration::from_secs(12);
const RETRY: Duration = Duration::from_secs(3);

/// What the window manager said about itself when the line came up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Peer {
    pub address: String,
    pub version: String,
    pub backend: String,
    pub platform: String,
    pub features: Vec<String>,
    pub config_schema_version: u32,
}

pub enum Request {
    /// Send a configuration, based on a revision.
    Apply {
        config: Box<Config>,
        base_revision: u64,
        client_id: String,
    },
    /// Ask for the running configuration.
    Fetch,
    /// Run a command (`{"command": "quit"}`).
    Dispatch(String),
    /// Ask what windows, workspaces and outputs there are.
    State,
    /// Ask the same a moment later: what the window manager reports
    /// follows an event by a tick, so asking at once can get the old.
    StateAfter(Duration),
    /// Start the running graph at one of its manual trigger nodes.
    Trigger(usize),
    /// Stop the running graph from running (`true`), or let it run again.
    Pause(bool),
}

/// What the running graph does: the graph page lights its nodes with it.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphEvent {
    /// A node (by its id) began to run.
    Started(usize),
    Completed(usize),
    /// A node failed, and why.
    Failed(usize, String),
    /// Whether the graph is kept from running.
    Paused(bool),
}

pub enum Event {
    Connected(Peer),
    /// The line is down, and why.
    Disconnected(String),
    /// The running configuration and its revision.
    Configuration(Box<Config>, u64),
    Applied(u64),
    /// Someone else changed the configuration first.
    Conflict(String),
    ApplyFailed(String),
    /// The configuration changed, by whom (`origin` is a client id, empty
    /// for a file edit or a toggle), to which revision.
    Changed {
        revision: u64,
        origin: String,
    },
    /// Something happened in the window manager (`window.opened`, ...).
    Runtime {
        kind: String,
        timestamp_ms: i64,
        window: Option<u64>,
        fields: Vec<(String, String)>,
    },
    Log {
        level: Level,
        message: String,
        timestamp_ms: i64,
    },
    /// A command was run, or not.
    Dispatched {
        command: String,
        error: Option<String>,
    },
    /// The answer to [`Request::State`], as the window manager's JSON.
    State(serde_json::Value),
    Graph(GraphEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

/// The window's end of the line.
pub struct Link {
    requests: UnboundedSender<Request>,
    events: Receiver<Event>,
}

impl Link {
    /// Start the line to `address`. `wake` is called after every event.
    pub fn start(address: String, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (requests, incoming) = unbounded_channel();
        let (outgoing, events) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("gevrek-link".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = outgoing.send(Event::Disconnected(format!("no runtime: {error}")));
                        return;
                    }
                };
                let tell = Teller {
                    events: outgoing,
                    wake: std::sync::Arc::new(wake),
                };
                runtime.block_on(run(address, incoming, tell));
            });
        if let Err(error) = spawned {
            log::error!("the line to the window manager could not be started: {error}");
        }
        Self { requests, events }
    }

    /// A line that leads nowhere (for a window that only edits a file).
    pub fn none() -> Self {
        let (requests, _) = unbounded_channel();
        let (_, events) = std::sync::mpsc::channel();
        Self { requests, events }
    }

    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    /// A way to send requests from another thread (what is waited for
    /// there and then told to the window manager).
    pub fn teller(&self) -> impl Fn(Request) + Send + 'static {
        let requests = self.requests.clone();
        move |request| {
            let _ = requests.send(request);
        }
    }

    /// What happened since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

#[derive(Clone)]
struct Teller {
    events: Sender<Event>,
    wake: std::sync::Arc<dyn Fn() + Send + Sync>,
}

impl Teller {
    fn tell(&self, event: Event) {
        let _ = self.events.send(event);
        (self.wake)();
    }
}

async fn run(address: String, mut requests: UnboundedReceiver<Request>, tell: Teller) {
    let endpoint = gevrek_protocol::Endpoint::parse(&address).address();
    loop {
        let mut client = match connect(&endpoint).await {
            Ok((client, peer)) => {
                tell.tell(Event::Connected(peer));
                client
            }
            Err(error) => {
                tell.tell(Event::Disconnected(error));
                // Requests made while there is no line are not kept for
                // later: the window knows it is not connected.
                let wait = tokio::time::sleep(RETRY);
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        _ = &mut wait => break,
                        request = requests.recv() => {
                            if request.is_none() {
                                return;
                            }
                        }
                    }
                }
                continue;
            }
        };
        fetch(&mut client, &tell).await;
        // Whether the graph runs is not in the configuration.
        if let Ok(Ok(state)) = tokio::time::timeout(CALL, client.query_state()).await {
            tell.tell(Event::Graph(GraphEvent::Paused(state.execution_paused)));
        }
        let mut stream = match client.subscribe_to_events().await {
            Ok(stream) => stream,
            Err(error) => {
                tell.tell(Event::Disconnected(format!("no event stream: {error}")));
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        // When the state is to be asked for. Asking is never waited for
        // inside this loop: a request that says "a little later" is a
        // time kept here, so that what comes meanwhile (a press on a
        // workspace, the next event) is carried at once; and requests
        // close to each other are one question.
        let mut due = Due::default();
        let lost = loop {
            use tokio_stream::StreamExt;
            let next = due.next();
            tokio::select! {
                event = stream.next() => match event {
                    Some(Ok(event)) => relay(event.event, &tell),
                    Some(Err(error)) => break format!("the window manager went away: {}", error.message()),
                    None => break "the window manager went away".to_string(),
                },
                request = requests.recv() => match request {
                    Some(Request::State) => due.ask(std::time::Instant::now()),
                    Some(Request::StateAfter(wait)) => {
                        due.ask(std::time::Instant::now() + wait.min(Duration::from_secs(2)))
                    }
                    Some(request) => serve(&mut client, request, &tell).await,
                    // The window is closed.
                    None => return,
                },
                _ = tokio::time::sleep_until(next.unwrap_or_else(std::time::Instant::now).into()), if next.is_some() => {
                    due.taken(std::time::Instant::now());
                    serve(&mut client, Request::State, &tell).await;
                }
            }
        };
        tell.tell(Event::Disconnected(lost));
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
}

/// The times at which the state is to be asked for. Times that are close
/// to one already kept are that one: a burst of events is a few
/// questions, not one for each.
#[derive(Debug, Default)]
pub struct Due {
    times: Vec<std::time::Instant>,
}

/// Times closer together than this are one question.
const TOGETHER: Duration = Duration::from_millis(40);

impl Due {
    pub fn ask(&mut self, at: std::time::Instant) {
        let near = |known: &std::time::Instant| {
            known.max(&at).saturating_duration_since(*known.min(&at)) < TOGETHER
        };
        if !self.times.iter().any(near) {
            self.times.push(at);
        }
    }

    /// The earliest time kept.
    pub fn next(&self) -> Option<std::time::Instant> {
        self.times.iter().min().copied()
    }

    /// The state was asked for at `now`: every time up to then is done.
    pub fn taken(&mut self, now: std::time::Instant) {
        self.times.retain(|time| *time > now);
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
}

async fn connect(endpoint: &str) -> Result<(WMClient, Peer), String> {
    let attempt = async {
        let mut client = WMClient::connect(endpoint.to_string())
            .await
            .map_err(|error| error.to_string())?;
        let health = client
            .health_check()
            .await
            .map_err(|error| error.to_string())?;
        let mut peer = Peer {
            address: endpoint.to_string(),
            version: health.version,
            ..Default::default()
        };
        if let Ok(capabilities) = client.get_capabilities().await {
            peer.backend = capabilities.backend;
            peer.platform = capabilities.platform;
            peer.features = capabilities.features;
            peer.config_schema_version = capabilities.config_schema_version;
            if !capabilities.server_version.is_empty() {
                peer.version = capabilities.server_version;
            }
        }
        Ok((client, peer))
    };
    match tokio::time::timeout(Duration::from_secs(5), attempt).await {
        Ok(result) => result,
        Err(_) => Err("no answer".to_string()),
    }
}

async fn fetch(client: &mut WMClient, tell: &Teller) {
    let document = tokio::time::timeout(CALL, client.get_configuration()).await;
    match document {
        Ok(Ok(document)) => match serde_json::from_str::<Config>(&document.json) {
            Ok(mut config) => {
                // Reading does not build the graph's lookup tables.
                config.node_graph.rebuild_connection_indices();
                tell.tell(Event::Configuration(Box::new(config), document.revision));
            }
            Err(error) => log::warn!("the window manager's configuration is not read: {error}"),
        },
        Ok(Err(error)) => log::warn!("the configuration could not be fetched: {error}"),
        Err(_) => log::warn!("the configuration could not be fetched: no answer"),
    }
}

async fn serve(client: &mut WMClient, request: Request, tell: &Teller) {
    match request {
        Request::Fetch => fetch(client, tell).await,
        Request::Apply {
            config,
            base_revision,
            client_id,
        } => {
            let json = match serde_json::to_string_pretty(&*config) {
                Ok(json) => json,
                Err(error) => {
                    return tell.tell(Event::ApplyFailed(format!("not written as JSON: {error}")))
                }
            };
            let document = ConfigurationDocument {
                json,
                version: config.version.clone(),
                base_revision,
                client_id,
                ..Default::default()
            };
            let event = match tokio::time::timeout(CALL, client.update_configuration(document)).await {
                Ok(Ok(response)) if response.success => Event::Applied(response.revision),
                Ok(Ok(response)) if response.conflict => {
                    Event::Conflict("Someone else changed the configuration in the meantime.".to_string())
                }
                Ok(Ok(response)) => Event::ApplyFailed(response.error.unwrap_or_else(|| "refused".to_string())),
                Ok(Err(error)) => Event::ApplyFailed(error.to_string()),
                Err(_) => Event::ApplyFailed(
                    "No answer from the window manager. The configuration may still be applied; it is shown when the window manager reports it."
                        .to_string(),
                ),
            };
            tell.tell(event);
        }
        Request::Dispatch(command) => {
            let error = match tokio::time::timeout(CALL, client.dispatch_command(command.clone()))
                .await
            {
                Ok(Ok(response)) if response.success => None,
                Ok(Ok(response)) => Some(response.error.unwrap_or_else(|| "refused".to_string())),
                Ok(Err(error)) => Some(error.to_string()),
                Err(_) => Some("no answer".to_string()),
            };
            tell.tell(Event::Dispatched { command, error });
        }
        Request::StateAfter(wait) => {
            tokio::time::sleep(wait.min(Duration::from_secs(2))).await;
            if let Ok(Ok(state)) = tokio::time::timeout(CALL, client.get_overlay_data()).await {
                if let Ok(json) = serde_json::to_value(&state) {
                    tell.tell(Event::State(json));
                }
            }
        }
        Request::State => {
            if let Ok(Ok(state)) = tokio::time::timeout(CALL, client.get_overlay_data()).await {
                if let Ok(json) = serde_json::to_value(&state) {
                    tell.tell(Event::State(json));
                }
            }
        }
        Request::Trigger(node) => {
            let error = said(
                tokio::time::timeout(CALL, client.trigger_manual_event(node.to_string())).await,
            );
            tell.tell(Event::Dispatched {
                command: "Manual trigger".to_string(),
                error,
            });
        }
        Request::Pause(pause) => {
            let answer = if pause {
                tokio::time::timeout(CALL, client.pause_execution()).await
            } else {
                tokio::time::timeout(CALL, client.resume_execution()).await
            };
            match said(answer) {
                None => tell.tell(Event::Graph(GraphEvent::Paused(pause))),
                Some(error) => tell.tell(Event::Dispatched {
                    command: if pause {
                        "Pause the graph"
                    } else {
                        "Let the graph run"
                    }
                    .to_string(),
                    error: Some(error),
                }),
            }
        }
    }
}

/// Why a call that answers with a status did not do what was asked, if
/// it did not.
fn said<E: std::fmt::Display>(
    answer: Result<Result<gevrek_protocol::proto::StatusResponse, E>, tokio::time::error::Elapsed>,
) -> Option<String> {
    match answer {
        Ok(Ok(response)) if response.success => None,
        Ok(Ok(response)) => Some(response.error.unwrap_or_else(|| "refused".to_string())),
        Ok(Err(error)) => Some(error.to_string()),
        Err(_) => Some("no answer".to_string()),
    }
}

fn relay(event: Option<Wire>, tell: &Teller) {
    // A node is named by its number in the graph.
    let graph = |node: &str, event: fn(usize) -> GraphEvent| {
        if let Ok(node) = node.parse::<usize>() {
            tell.tell(Event::Graph(event(node)));
        }
    };
    match event {
        Some(Wire::NodeStarted(started)) => graph(&started.node_id, GraphEvent::Started),
        Some(Wire::NodeCompleted(completed)) => graph(&completed.node_id, GraphEvent::Completed),
        Some(Wire::NodeFailed(failed)) => {
            if let Ok(node) = failed.node_id.parse::<usize>() {
                tell.tell(Event::Graph(GraphEvent::Failed(node, failed.error)));
            }
        }
        Some(Wire::ConfigurationChanged(changed)) => tell.tell(Event::Changed {
            revision: changed.revision,
            origin: changed.origin_client_id,
        }),
        // Events were missed: a change of the configuration may be among them.
        Some(Wire::EventsDropped(_)) => tell.tell(Event::Changed {
            revision: u64::MAX,
            origin: String::new(),
        }),
        Some(Wire::RuntimeEvent(event)) => {
            let mut fields: Vec<(String, String)> = event.fields.into_iter().collect();
            fields.sort();
            tell.tell(Event::Runtime {
                kind: event.kind,
                timestamp_ms: event.timestamp_ms,
                window: event.window_id,
                fields,
            });
        }
        Some(Wire::LogEvent(entry)) => {
            use gevrek_protocol::proto::LogLevel;
            let level = match LogLevel::try_from(entry.level).unwrap_or(LogLevel::Info) {
                LogLevel::Debug => Level::Debug,
                LogLevel::Warn => Level::Warning,
                LogLevel::Error => Level::Error,
                _ => Level::Info,
            };
            tell.tell(Event::Log {
                level,
                message: entry.message,
                timestamp_ms: entry.timestamp_ms,
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod due_tests {
    use super::*;

    #[test]
    fn a_burst_of_requests_for_the_state_is_a_few_questions() {
        let start = std::time::Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut due = Due::default();
        assert!(due.next().is_none());
        // Ten events within a moment, each asking now, 180 ms and 400 ms later.
        for event in 0..10 {
            for after in [0, 180, 400] {
                due.ask(at(event * 3 + after));
            }
        }
        assert_eq!(due.len(), 3, "one question now and two later, not thirty");
        assert_eq!(due.next(), Some(at(0)));
        due.taken(at(5));
        assert_eq!(due.next(), Some(at(180)));
        // Asked late: everything that was due by then is done with it.
        due.taken(at(450));
        assert!(due.is_empty());
        // Times far enough apart are each kept.
        due.ask(at(1000));
        due.ask(at(1100));
        assert_eq!(due.len(), 2);
    }
}
