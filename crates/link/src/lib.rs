//! The line to the window manager.
//!
//! Everything that waits on the window manager happens on a thread of its
//! own; the window never does. The window says what it wants
//! ([`Request`]) and is told what happened ([`Event`]), and each event
//! wakes the window so that it draws.
//!
//! The line is kept: when the window manager is not there, or goes away,
//! it is tried again every few seconds until it answers.
//!
//! The program that runs the zones on the desktop has a part of the line
//! of its own (`zones`).

pub mod zones;
pub use zones::{ZonesAsk, ZonesHeard, ZonesTell};

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
    /// Ask something of the zones on the desktop: a zones command as
    /// JSON (`{"command": "quick_hide"}`). Answered with
    /// [`Event::ZonesAnswered`], whatever comes of it. (This is the
    /// asking side, for any program; the program that runs the zones
    /// uses [`Link::host_zones`] and what goes with it.)
    Zones(String),
    /// Ask how the zones are, as the shell that runs them last told the
    /// window manager. Answered with [`Event::ZonesState`].
    ZonesState,
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
    /// What came of a [`Request::Zones`]. `success`: it was carried out,
    /// and `events` is what happened because of it (the zones' events as
    /// JSON). Otherwise `error` says why: the zones' own reason, or that
    /// no shell runs them (`delivered` is false then), or that the shell
    /// or the window manager did not answer.
    ZonesAnswered {
        command: String,
        success: bool,
        delivered: bool,
        error: Option<String>,
        events: Vec<String>,
    },
    /// The answer to [`Request::ZonesState`]. `json` is the zones'
    /// snapshot (`gevrek_zones::ipc::ZonesSnapshot`), empty where there
    /// is none; `running`: a shell runs the zones now; `stale`: `json` is
    /// from a shell that has gone since; `message` says in words why
    /// there is nothing to show or why it is old.
    ZonesState {
        running: bool,
        stale: bool,
        json: String,
        message: String,
    },
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
    zones: UnboundedSender<ZonesTell>,
    zones_heard: Receiver<ZonesHeard>,
}

impl Link {
    /// Start the line to `address`. `wake` is called after every event.
    pub fn start(address: String, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (requests, incoming) = unbounded_channel();
        let (outgoing, events) = std::sync::mpsc::channel();
        let (zones, zones_told) = unbounded_channel();
        let (zones_hearing, zones_heard) = std::sync::mpsc::channel();
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
                let zones = zones::ZonesSide::new(
                    zones_told,
                    zones::ZonesTeller {
                        heard: zones_hearing,
                        wake: std::sync::Arc::clone(&tell.wake),
                    },
                );
                runtime.block_on(run(address, incoming, tell, zones));
            });
        if let Err(error) = spawned {
            log::error!("the line to the window manager could not be started: {error}");
        }
        Self {
            requests,
            events,
            zones,
            zones_heard,
        }
    }

    /// A line that leads nowhere (for a window that only edits a file).
    pub fn none() -> Self {
        let (requests, _) = unbounded_channel();
        let (_, events) = std::sync::mpsc::channel();
        let (zones, _) = unbounded_channel();
        let (_, zones_heard) = std::sync::mpsc::channel();
        Self {
            requests,
            events,
            zones,
            zones_heard,
        }
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

    /// Say that this program runs the zones on the desktop. Said once,
    /// at any time; it holds for as long as the line does, over every
    /// loss of the window manager. From then on [`Link::poll_zones`] tells
    /// what is asked of the zones. Only one program does this (the
    /// shell): a second one is not let in by the window manager while the
    /// first is there.
    pub fn host_zones(&self) {
        let _ = self.zones.send(ZonesTell::Host);
    }

    /// Tell how the zones are now (`ZonesSnapshot::of(&core, now)`): after
    /// everything that changes them, and once at the start. Not with every
    /// movement of a drag: when it ends, or a few times a second. The
    /// last one is kept and told again by the line itself whenever the
    /// window manager comes (back), so there is nothing to do on
    /// [`ZonesHeard::Attached`].
    pub fn publish_zones(&self, snapshot: gevrek_zones::ipc::ZonesSnapshot) {
        let _ = self.zones.send(ZonesTell::Snapshot(Box::new(snapshot)));
    }

    /// Tell of something that happened to the zones (each
    /// `Effect::Notify` the core gives; `gevrek_zones::ipc::events_of`
    /// picks them out), before the snapshot that shows it. Let go while
    /// the window manager is not there.
    pub fn publish_zones_event(&self, event: gevrek_zones::ZonesEvent) {
        let _ = self.zones.send(ZonesTell::Event(event));
    }

    /// Answer what was asked with a number ([`ZonesAsk::request`]):
    /// `Answer::of(&effects)` for a command the core was given,
    /// `Answer::done()` or `Answer::refused(why)` for what is not the
    /// core's. Whoever asked waits a few seconds, no longer.
    pub fn answer_zones(&self, request: u64, answer: gevrek_zones::ipc::Answer) {
        let _ = self.zones.send(ZonesTell::Answer { request, answer });
    }

    /// The same four from another thread.
    pub fn zones_teller(&self) -> impl Fn(ZonesTell) + Send + 'static {
        let zones = self.zones.clone();
        move |tell| {
            let _ = zones.send(tell);
        }
    }

    /// What was asked of the zones, and whether the window manager
    /// listens, since the last call. Nothing ever for a program that did
    /// not call [`Link::host_zones`]. The window is woken for each, as it
    /// is for an [`Event`].
    pub fn poll_zones(&self) -> Vec<ZonesHeard> {
        self.zones_heard.try_iter().collect()
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

async fn run(
    address: String,
    mut requests: UnboundedReceiver<Request>,
    tell: Teller,
    mut zones: zones::ZonesSide,
) {
    let endpoint = gevrek_protocol::Endpoint::parse(&address).address();
    // That the window manager is not reached is said once, not at each
    // try: it is tried every few seconds for as long as it is not there.
    let mut unreached_said = false;
    loop {
        log::trace!("the line to the window manager at {endpoint} is tried");
        let (mut client, features) = match connect(&endpoint).await {
            Ok((client, peer)) => {
                log::debug!(
                    "the line to the window manager at {endpoint} is up: version {}, backend {:?} on {:?}",
                    peer.version,
                    peer.backend,
                    peer.platform
                );
                unreached_said = false;
                let features = peer.features.clone();
                tell.tell(Event::Connected(peer));
                (client, features)
            }
            Err(error) => {
                if !unreached_said {
                    log::debug!(
                        "the window manager at {endpoint} is not reached, tried again every {} s: {error}",
                        RETRY.as_secs()
                    );
                    unreached_said = true;
                }
                tell.tell(Event::Disconnected(error));
                // Requests made while there is no line are not kept for
                // later: the window knows it is not connected.
                let wait = tokio::time::sleep(RETRY);
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        _ = &mut wait => break,
                        request = requests.recv() => {
                            match request {
                                None => return,
                                // A configuration that cannot be
                                // handed over is said to have failed:
                                // whoever sent it waits for an answer.
                                Some(Request::Apply { .. }) => tell.tell(Event::ApplyFailed(
                                    "the window manager is not connected".to_string(),
                                )),
                                Some(_) => {}
                            }
                        }
                        told = zones.tells.recv(), if !zones.closed => match told {
                            Some(told) => zones.keep(told),
                            None => zones.closed = true,
                        },
                    }
                }
                continue;
            }
        };
        fetch(&mut client, &tell).await;
        // Whether the graph runs is not in the configuration.
        match tokio::time::timeout(CALL, client.query_state()).await {
            Ok(Ok(state)) => tell.tell(Event::Graph(GraphEvent::Paused(state.execution_paused))),
            Ok(Err(error)) => {
                log::debug!(
                    "whether the graph runs is not known: the state was not answered: {error:#}"
                )
            }
            Err(_) => log::debug!(
                "whether the graph runs is not known: the state was not answered in time"
            ),
        }
        let mut stream = match client.subscribe_to_events().await {
            Ok(stream) => stream,
            Err(error) => {
                log::debug!("the line is given up: the events were not subscribed to, tried again in {} s: {error:#}", RETRY.as_secs());
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
        zones.attach(&mut client, &features).await;
        let lost = loop {
            use tokio_stream::StreamExt;
            let next = due.next();
            let zones_retry = zones.retry_at;
            tokio::select! {
                event = stream.next() => match event {
                    Some(Ok(event)) => {
                        if let Some(Wire::RuntimeEvent(event)) = &event.event {
                            zones.heard(&event.kind, &event.fields);
                        }
                        relay(event.event, &tell)
                    }
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
                told = zones.tells.recv(), if !zones.closed => match told {
                    Some(told) => zones.take(told, &mut client, &features).await,
                    None => zones.closed = true,
                },
                asked = zones::next(&mut zones.call) => match asked {
                    Some(Ok(request)) => zones.asked(request),
                    Some(Err(error)) => zones.lost(format!("the zones' call ended: {}", error.message())),
                    None => zones.lost("the zones' call ended".to_string()),
                },
                _ = tokio::time::sleep_until(zones_retry.unwrap_or_else(std::time::Instant::now).into()), if zones_retry.is_some() => {
                    zones.attach(&mut client, &features).await;
                }
            }
        };
        zones.line_down();
        log::debug!("the line to the window manager at {endpoint} is down: {lost}");
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
    // (Asked for again where no answer came: a window manager that has
    // just started may be busy for a moment.)
    let mut why = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        let document = tokio::time::timeout(CALL, client.get_configuration()).await;
        match document {
            Ok(Ok(document)) => match serde_json::from_str::<Config>(&document.json) {
                Ok(mut config) => {
                    // Reading does not build the graph's lookup tables.
                    config.node_graph.rebuild_connection_indices();
                    log::debug!(
                        "the running configuration was fetched: revision {}",
                        document.revision
                    );
                    tell.tell(Event::Configuration(Box::new(config), document.revision));
                    return;
                }
                // (Asking again would read the same: a window manager of
                // another version, whose configuration this one cannot read.)
                Err(error) => {
                    why = format!("the window manager's configuration is not read: {error}");
                    break;
                }
            },
            Ok(Err(error)) => why = format!("the configuration could not be fetched: {error}"),
            Err(_) => why = "the configuration could not be fetched: no answer".to_string(),
        }
        if attempt < 2 {
            log::debug!("{why}; it is asked for again");
        }
    }
    log::warn!("{why}");
    // Told, so that whoever shows the configuration can say that what it
    // shows is not the window manager's.
    tell.tell(Event::Log {
        level: Level::Error,
        message: format!("{FETCH_FAILED}{why}"),
        timestamp_ms: 0,
    });
}

/// How the message begins that tells of a configuration that could not be
/// fetched (an [`Event::Log`] of the level `Error`).
pub const FETCH_FAILED: &str = "configuration not fetched: ";

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
            match &event {
                Event::Applied(revision) => {
                    log::debug!("the configuration was applied as revision {revision}")
                }
                Event::Conflict(_) => log::debug!(
                    "the configuration was not applied: it changed meanwhile (from revision {base_revision})"
                ),
                Event::ApplyFailed(why) => log::debug!("the configuration was not applied: {why}"),
                _ => {}
            }
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
            let name = command_name(&command);
            match &error {
                None => log::debug!("the command {name} was run"),
                Some(why) => log::debug!("the command {name} was not run: {why}"),
            }
            tell.tell(Event::Dispatched { command, error });
        }
        Request::StateAfter(wait) => {
            tokio::time::sleep(wait.min(Duration::from_secs(2))).await;
            state(client, tell).await;
        }
        Request::State => {
            state(client, tell).await;
        }
        Request::Zones(command) => {
            let answer = tokio::time::timeout(
                CALL,
                client.send_zones_command(command.clone(), gevrek_zones::ipc::VERSION, 0),
            )
            .await;
            let event = zones_answered(command, answer);
            if let Event::ZonesAnswered {
                command,
                success,
                error,
                ..
            } = &event
            {
                let name = command_name(command);
                match (success, error) {
                    (true, _) => log::debug!("the zones did {name}"),
                    (false, why) => log::debug!(
                        "the zones did not do {name}: {}",
                        why.as_deref().unwrap_or("no reason given")
                    ),
                }
            }
            tell.tell(event);
        }
        Request::ZonesState => {
            tell.tell(zones_state(
                tokio::time::timeout(CALL, client.get_zones_state()).await,
            ));
        }
        Request::Trigger(node) => {
            let error = said(
                tokio::time::timeout(CALL, client.trigger_manual_event(node.to_string())).await,
            );
            match &error {
                None => log::debug!("node {node} was triggered"),
                Some(why) => log::debug!("node {node} was not triggered: {why}"),
            }
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
                None => {
                    log::debug!(
                        "the graph is {} now",
                        if pause { "paused" } else { "let run" }
                    );
                    tell.tell(Event::Graph(GraphEvent::Paused(pause)))
                }
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

/// Ask for the state the window manager reports and tell it. Asked after
/// each event, so what goes wrong is said at trace.
async fn state(client: &mut WMClient, tell: &Teller) {
    match tokio::time::timeout(CALL, client.get_overlay_data()).await {
        Ok(Ok(state)) => match serde_json::to_value(&state) {
            Ok(json) => tell.tell(Event::State(json)),
            Err(error) => log::trace!("the state was not read as JSON: {error}"),
        },
        Ok(Err(error)) => log::trace!("the state was not answered: {error:#}"),
        Err(_) => log::trace!("the state was not answered in time"),
    }
}

/// The name of a command in its JSON (`{"command": "quit", ..}`), without
/// its arguments, for the log.
fn command_name(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| value.get("command")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "(not read)".to_string())
}

/// Why a call that answers with a status did not do what was asked, if
/// it did not.
/// What is told of a zones command, from what the window manager
/// answered, or did not.
fn zones_answered<E: std::fmt::Display>(
    command: String,
    answer: Result<
        Result<gevrek_protocol::proto::ZonesCommandReply, E>,
        tokio::time::error::Elapsed,
    >,
) -> Event {
    let not_asked = |error: String| Event::ZonesAnswered {
        command: command.clone(),
        success: false,
        delivered: false,
        error: Some(error),
        events: Vec::new(),
    };
    match answer {
        Ok(Ok(reply)) => Event::ZonesAnswered {
            command,
            success: reply.success,
            delivered: reply.delivered,
            error: reply.error,
            events: reply.events,
        },
        Ok(Err(error)) => not_asked(error.to_string()),
        Err(_) => not_asked("no answer".to_string()),
    }
}

/// What is told of the zones' state, from what the window manager
/// answered, or did not (said as a state with nothing in it and the
/// reason, so that whoever asked always hears back).
fn zones_state<E: std::fmt::Display>(
    answer: Result<
        Result<gevrek_protocol::proto::ZonesStateResponse, E>,
        tokio::time::error::Elapsed,
    >,
) -> Event {
    let nothing = |message: String| Event::ZonesState {
        running: false,
        stale: false,
        json: String::new(),
        message,
    };
    match answer {
        Ok(Ok(state)) => Event::ZonesState {
            running: state.running,
            stale: state.stale,
            json: state.json,
            message: state.message,
        },
        Ok(Err(error)) => nothing(format!("The zones' state was not answered: {error}")),
        Err(_) => nothing("The zones' state was not answered: no answer".to_string()),
    }
}

#[cfg(test)]
mod zones_asking_tests {
    use super::*;
    use gevrek_protocol::proto::{ZonesCommandReply, ZonesStateResponse};

    type Failed = Result<ZonesCommandReply, String>;

    #[test]
    fn a_zones_answer_is_told_as_it_came_or_as_not_asked() {
        let reply = ZonesCommandReply {
            success: false,
            error: Some("no zone is called nowhere".to_string()),
            events: vec![r#"{"event":"refused","reason":"no zone is called nowhere"}"#.to_string()],
            delivered: true,
        };
        let Event::ZonesAnswered {
            command,
            success,
            delivered,
            error,
            events,
        } = zones_answered("{}".to_string(), Ok(Failed::Ok(reply)))
        else {
            panic!("not an answer of the zones");
        };
        assert_eq!((command.as_str(), success, delivered), ("{}", false, true));
        assert_eq!(error.as_deref(), Some("no zone is called nowhere"));
        assert_eq!(events.len(), 1);

        let Event::ZonesAnswered {
            success,
            delivered,
            error,
            ..
        } = zones_answered(
            "{}".to_string(),
            Ok(Failed::Err("the line broke".to_string())),
        )
        else {
            panic!("not an answer of the zones");
        };
        assert!(!success && !delivered);
        assert_eq!(error.as_deref(), Some("the line broke"));
    }

    #[test]
    fn the_zones_state_is_told_also_where_nothing_answered() {
        let state = ZonesStateResponse {
            running: true,
            json: r#"{"version":1}"#.to_string(),
            ..Default::default()
        };
        let Event::ZonesState {
            running,
            stale,
            json,
            message,
        } = zones_state(Ok(Ok::<_, String>(state)))
        else {
            panic!("not the zones' state");
        };
        assert!(running && !stale && message.is_empty());
        assert_eq!(json, r#"{"version":1}"#);
        let Event::ZonesState {
            running,
            json,
            message,
            ..
        } = zones_state(Ok(Err::<ZonesStateResponse, _>("gone".to_string())))
        else {
            panic!("not the zones' state");
        };
        assert!(!running && json.is_empty());
        assert_eq!(message, "The zones' state was not answered: gone");
    }
}

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
