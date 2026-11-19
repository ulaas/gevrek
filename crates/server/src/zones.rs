//! The zones, as far as the window manager has to do with them.
//!
//! The zones on the desktop live in the shell. The window manager is the
//! one place other programs talk to, so it stands between: it keeps what
//! the shell last said of the zones and answers with it, passes on what
//! happens to them to whoever listens, and sends what is asked of them to
//! the shell and waits for its answer.
//!
//! Nothing here reads the zones' JSON beyond seeing that a command is an
//! object: its shapes are gevrek-zones' and still move, and the shell that
//! runs the zones is the one that says what a command means.

use gevrek_protocol::proto::{
    execution_event, zones_report, ExecutionEvent, RuntimeEvent, ZonesCommandReply, ZonesReport,
    ZonesRequest, ZonesStateResponse,
};
use gevrek_engine::shell_room::{ShellRoom, Side};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot};
use tonic::Status;

/// How long the shell is waited for where the asker names no time. The
/// shell answers from its own loop, which draws many times a second.
pub const WAIT: Duration = Duration::from_secs(5);
/// The longest an asker can make that.
pub const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// What is said where no shell runs the zones. One sentence, the same
/// for a question and for a command, so a script can tell it.
pub const NOT_RUNNING: &str =
    "Zones are not running: no gevrek-shell that runs them is attached to this window manager";

/// How much may wait for the shell at once. Beyond it the shell is not
/// taking what it is sent, and an asker is told so at once.
const QUEUE: usize = 64;

type Requests = mpsc::Sender<Result<ZonesRequest, Status>>;

/// The shell that runs the zones now.
struct Attached {
    session: u64,
    requests: Requests,
}

/// What a shell last said of the zones.
struct Kept {
    session: u64,
    version: u32,
    sequence: u64,
    json: String,
    published_ms: i64,
}

/// Someone who waits for the shell's answer.
struct Waiting {
    session: u64,
    answer: oneshot::Sender<ZonesCommandReply>,
}

#[derive(Default)]
struct Inner {
    attached: Option<Attached>,
    /// Counts up with every shell that attaches.
    sessions: u64,
    snapshot: Option<Kept>,
    /// Counts up with every snapshot.
    sequence: u64,
    /// Counts up with every command sent.
    requests: u64,
    waiting: HashMap<u64, Waiting>,
    /// The room the zones keep clear of tiled windows, as the attached
    /// shell last said it. None while no shell is attached or the one
    /// that is has not said how the zones are: what is served as stale
    /// keeps no room.
    room: Vec<ShellRoom>,
}

/// The room a snapshot says the zones keep (`reserved`: for each region
/// the display's lasting id, the side, and how deep). What is not
/// written so is left out.
pub fn room_of(snapshot: &str) -> Vec<ShellRoom> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(snapshot) else {
        return Vec::new();
    };
    value["reserved"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|region| {
            Some(ShellRoom {
                display: region["display"].as_str().filter(|id| !id.is_empty())?.to_string(),
                side: Side::named(region["edge"].as_str()?)?,
                depth: i32::try_from(region["thickness"].as_i64()?).ok().filter(|depth| *depth > 0)?,
            })
        })
        .collect()
}

/// What the window manager knows of the zones and of the shell that runs
/// them.
pub struct ZonesHub {
    inner: Mutex<Inner>,
    events: Arc<broadcast::Sender<ExecutionEvent>>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn not_answered(error: impl Into<String>, delivered: bool) -> ZonesCommandReply {
    ZonesCommandReply {
        success: false,
        error: Some(error.into()),
        events: Vec::new(),
        delivered,
    }
}

impl ZonesHub {
    /// `events` is where subscribers of the event stream are told from.
    pub fn new(events: Arc<broadcast::Sender<ExecutionEvent>>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            events,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Tell subscribers of something about the zones, as the runtime's own
    /// events are told: a kind and fields.
    fn tell(&self, kind: String, fields: Vec<(String, String)>) {
        let _ = self.events.send(ExecutionEvent {
            event: Some(execution_event::Event::RuntimeEvent(RuntimeEvent {
                kind,
                timestamp_ms: now_ms(),
                window_id: None,
                fields: fields.into_iter().collect(),
            })),
        });
    }

    /// A shell says it runs the zones. It is given a number for what it
    /// reports and the commands it is to carry out. Refused while another
    /// shell is attached: two that both draw the desktop's icons would
    /// take the line from each other for ever, and the one that is there
    /// is the one whose state is being served.
    pub fn attach(&self) -> Result<(u64, mpsc::Receiver<Result<ZonesRequest, Status>>), Status> {
        let (requests, receiver) = mpsc::channel(QUEUE);
        // (A shell that ended is taken out when its call ends. One whose
        // call is over while that has not been seen yet is found here,
        // and taken out as it would have been.)
        let over = {
            let inner = self.lock();
            inner
                .attached
                .as_ref()
                .filter(|attached| attached.requests.is_closed())
                .map(|attached| attached.session)
        };
        if let Some(over) = over {
            self.detach(over);
        }
        let session = {
            let mut inner = self.lock();
            if inner.attached.is_some() {
                return Err(Status::already_exists(
                    "Another gevrek-shell runs the zones for this window manager",
                ));
            }
            inner.sessions += 1;
            let session = inner.sessions;
            inner.attached = Some(Attached { session, requests });
            // (Until it says how the zones are, none keeps room.)
            inner.keep_room(Vec::new());
            session
        };
        log::info!("A shell runs the zones (session {session})");
        self.tell(
            "zones.attached".to_string(),
            vec![("session".to_string(), session.to_string())],
        );
        Ok((session, receiver))
    }

    /// The shell of a session is gone (its call ended). What it last said
    /// is kept and served as stale; whoever waits for it is told.
    pub fn detach(&self, session: u64) {
        let waiting: Vec<Waiting> = {
            let mut inner = self.lock();
            if inner
                .attached
                .as_ref()
                .is_some_and(|attached| attached.session == session)
            {
                inner.attached = None;
                // The room its zones kept is the windows' again.
                inner.keep_room(Vec::new());
            } else {
                return;
            }
            let ids: Vec<u64> = inner
                .waiting
                .iter()
                .filter(|(_, waiting)| waiting.session == session)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| inner.waiting.remove(&id))
                .collect()
        };
        for waiting in waiting {
            let _ = waiting.answer.send(not_answered(
                "The shell that runs the zones went away before it answered",
                true,
            ));
        }
        log::info!("The shell that ran the zones is gone (session {session})");
        self.tell(
            "zones.detached".to_string(),
            vec![("session".to_string(), session.to_string())],
        );
    }

    /// Something the attached shell said. What a shell that is no longer
    /// the attached one says is dropped.
    pub fn report(&self, session: u64, report: ZonesReport) {
        match report.report {
            Some(zones_report::Report::Snapshot(snapshot)) => {
                let mut inner = self.lock();
                if !inner.is_attached(session) {
                    return;
                }
                inner.sequence += 1;
                let sequence = inner.sequence;
                let room = room_of(&snapshot.json);
                inner.keep_room(room);
                inner.snapshot = Some(Kept {
                    session,
                    version: snapshot.version,
                    sequence,
                    json: snapshot.json,
                    published_ms: now_ms(),
                });
            }
            Some(zones_report::Report::Event(event)) => {
                let sequence = {
                    let inner = self.lock();
                    if !inner.is_attached(session) {
                        return;
                    }
                    inner.sequence
                };
                // The name is part of an event's kind; one that is not a
                // plain word is not passed on as one.
                let name: String = event
                    .name
                    .chars()
                    .filter(|letter| letter.is_ascii_alphanumeric() || *letter == '_')
                    .collect();
                self.tell(
                    format!("zones.{}", if name.is_empty() { "event" } else { &name }),
                    vec![
                        ("json".to_string(), event.json),
                        ("version".to_string(), event.version.to_string()),
                        // The snapshot there was when this happened: the
                        // one that tells of it has a higher number.
                        ("sequence".to_string(), sequence.to_string()),
                    ],
                );
            }
            Some(zones_report::Report::Answer(answer)) => {
                let waiting = {
                    let mut inner = self.lock();
                    match inner.waiting.get(&answer.request_id) {
                        Some(waiting) if waiting.session == session => {
                            inner.waiting.remove(&answer.request_id)
                        }
                        // (Answered too late, or twice: nobody waits.)
                        _ => None,
                    }
                };
                if let Some(waiting) = waiting {
                    let _ = waiting.answer.send(ZonesCommandReply {
                        success: answer.success,
                        error: answer.error,
                        events: answer.events,
                        delivered: true,
                    });
                }
            }
            None => {}
        }
    }

    /// The room the zones keep clear of tiled windows now.
    pub fn room(&self) -> Vec<ShellRoom> {
        self.lock().room.clone()
    }

    /// How the zones are, as far as this window manager was told: all of
    /// what the shell said.
    pub fn state(&self) -> ZonesStateResponse {
        self.state_of(false, "")
    }

    /// The same with less in it: without the items (`without_items`),
    /// and of one zone alone (`zone`, unless it is empty). A desktop of
    /// thousands of items makes a snapshot of a megabyte; whoever only
    /// wants to know which zones there are is spared reading it. The
    /// trimming is done here, outside the lock, on a copy: the shell
    /// tells one snapshot for everybody and is asked for nothing more.
    pub fn state_of(&self, without_items: bool, zone: &str) -> ZonesStateResponse {
        let mut state = self.whole_state();
        if (without_items || !zone.is_empty()) && !state.json.is_empty() {
            if let Some(json) = trimmed(&state.json, without_items, zone) {
                state.json = json;
            }
        }
        state
    }

    fn whole_state(&self) -> ZonesStateResponse {
        let inner = self.lock();
        let running = inner.attached.is_some();
        let fresh = match (&inner.attached, &inner.snapshot) {
            (Some(attached), Some(kept)) => attached.session == kept.session,
            _ => false,
        };
        let message = match (running, inner.snapshot.is_some(), fresh) {
            (_, _, true) => String::new(),
            (false, false, _) => NOT_RUNNING.to_string(),
            (false, true, _) => format!(
                "{NOT_RUNNING}. This is what the shell that ran them last reported"
            ),
            (true, false, _) => "The shell has not reported the zones yet".to_string(),
            (true, true, _) => {
                "The shell has not reported the zones yet. This is what the shell before it last reported"
                    .to_string()
            }
        };
        let kept = inner.snapshot.as_ref();
        ZonesStateResponse {
            running,
            stale: kept.is_some() && !fresh,
            version: kept.map_or(0, |kept| kept.version),
            sequence: kept.map_or(0, |kept| kept.sequence),
            json: kept.map(|kept| kept.json.clone()).unwrap_or_default(),
            published_ms: kept.map_or(0, |kept| kept.published_ms),
            message,
        }
    }

    /// Send a command to the shell and wait for what it says. That no
    /// shell runs the zones, or that it did not answer, is an answer too
    /// (`success` false with the reason), not a failed call: only what is
    /// no command at all is refused as one.
    pub async fn command(
        &self,
        json: String,
        version: u32,
        timeout_ms: u32,
    ) -> Result<ZonesCommandReply, Status> {
        let is_command = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .is_some_and(|value| value.get("command").is_some_and(|word| word.is_string()));
        if !is_command {
            return Err(Status::invalid_argument(
                "Not a zones command: it is a JSON object with a \"command\", as {\"command\": \"quick_hide\"}",
            ));
        }
        let wait = if timeout_ms == 0 {
            WAIT
        } else {
            Duration::from_millis(u64::from(timeout_ms)).min(LONGEST_WAIT)
        };
        let (id, answered) = {
            let mut inner = self.lock();
            let Some(attached) = &inner.attached else {
                return Ok(not_answered(NOT_RUNNING, false));
            };
            let (session, requests) = (attached.session, attached.requests.clone());
            inner.requests += 1;
            let id = inner.requests;
            let request = ZonesRequest {
                request_id: id,
                version,
                json,
            };
            if requests.try_send(Ok(request)).is_err() {
                return Ok(not_answered(
                    "The shell that runs the zones is not taking what is asked of them",
                    false,
                ));
            }
            let (answer, answered) = oneshot::channel();
            inner.waiting.insert(id, Waiting { session, answer });
            (id, answered)
        };
        match tokio::time::timeout(wait, answered).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Ok(not_answered(
                "The shell that runs the zones went away before it answered",
                true,
            )),
            Err(_) => {
                self.lock().waiting.remove(&id);
                Ok(not_answered(
                    format!(
                        "The shell did not answer within {} ms; what was asked may still be carried out",
                        wait.as_millis()
                    ),
                    true,
                ))
            }
        }
    }
}

/// A snapshot with less in it (see `ZonesHub::state_of`). This is the one
/// place the window manager looks into the zones' JSON, and it knows of
/// it only what `GetZonesState` promises by name: `zones` is a list of
/// objects with an `id` and `items`, `loose` is a list. Everything else
/// is handed on as it came. None where the text is not such an object;
/// it is then served whole, and its reader says what is wrong with it.
fn trimmed(json: &str, without_items: bool, zone: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(json).ok()?;
    let snapshot = value.as_object_mut()?;
    if let Some(zones) = snapshot.get_mut("zones").and_then(|zones| zones.as_array_mut()) {
        if !zone.is_empty() {
            zones.retain(|one| one.get("id").and_then(|id| id.as_str()) == Some(zone));
        }
        if without_items {
            for one in zones.iter_mut().filter_map(|one| one.as_object_mut()) {
                if let Some(items) = one.remove("items") {
                    // (A shell that does not count them itself.)
                    if !one.contains_key("item_count") {
                        let count = items.as_array().map_or(0, Vec::len);
                        one.insert("item_count".to_string(), count.into());
                    }
                }
            }
        }
    }
    if without_items {
        if let Some(loose) = snapshot.remove("loose") {
            if !snapshot.contains_key("loose_count") {
                let count = loose.as_array().map_or(0, Vec::len);
                snapshot.insert("loose_count".to_string(), count.into());
            }
        }
    }
    serde_json::to_string(&value).ok()
}

impl Inner {
    /// The zones keep this room now: kept here, and left for the window
    /// manager's loop, which takes it off the displays where it is the
    /// one that keeps room (`gevrek_engine::shell_room`).
    fn keep_room(&mut self, room: Vec<ShellRoom>) {
        if self.room != room {
            self.room = room.clone();
        }
        gevrek_engine::shell_room::tell(room);
    }

    fn is_attached(&self, session: u64) -> bool {
        self.attached
            .as_ref()
            .is_some_and(|attached| attached.session == session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_protocol::proto::{ZonesCommandAnswer, ZonesEventReport, ZonesSnapshotReport};

    fn hub() -> (Arc<ZonesHub>, broadcast::Receiver<ExecutionEvent>) {
        let (events, receiver) = broadcast::channel(32);
        (Arc::new(ZonesHub::new(Arc::new(events))), receiver)
    }

    fn snapshot(json: &str) -> ZonesReport {
        ZonesReport {
            report: Some(zones_report::Report::Snapshot(ZonesSnapshotReport {
                version: 1,
                json: json.to_string(),
            })),
        }
    }

    fn answer(request_id: u64, error: Option<&str>) -> ZonesReport {
        ZonesReport {
            report: Some(zones_report::Report::Answer(ZonesCommandAnswer {
                request_id,
                success: error.is_none(),
                error: error.map(str::to_string),
                events: vec![r#"{"event":"layout_changed"}"#.to_string()],
            })),
        }
    }

    #[tokio::test]
    async fn the_room_the_zones_keep_is_the_attached_shells_and_given_back_with_it() {
        let (hub, _events) = hub();
        let reserving = r#"{"version":1,"zones":[],"reserved":[
            {"display":"edid:DEL-A1","rect":{"x":0,"y":40,"width":1920,"height":120},
             "edge":"top","thickness":120,"zones":["dock"]},
            {"display":"edid:SAM-B2","rect":{"x":3560,"y":0,"width":280,"height":1080},
             "edge":"right","thickness":280,"zones":["side"]}]}"#;
        let room = |display: &str, side: Side, depth: i32| ShellRoom {
            display: display.to_string(),
            side,
            depth,
        };
        assert!(hub.room().is_empty());
        let (first, _line) = hub.attach().expect("attached");
        // Attached, and nothing said yet: no room.
        assert!(hub.room().is_empty());
        hub.report(first, snapshot(reserving));
        assert_eq!(
            hub.room(),
            [
                room("edid:DEL-A1", Side::Top, 120),
                room("edid:SAM-B2", Side::Right, 280)
            ]
        );
        // The zones keep less: what the last snapshot says holds.
        hub.report(first, snapshot(r#"{"version":1,"zones":[],"reserved":[]}"#));
        assert!(hub.room().is_empty());
        hub.report(first, snapshot(reserving));
        assert_eq!(hub.room().len(), 2);
        // The shell is gone: its snapshot is served as stale, and keeps
        // no room.
        hub.detach(first);
        assert!(hub.state().stale);
        assert!(hub.room().is_empty());
        // What a shell that is not attached says keeps none either.
        hub.report(first, snapshot(reserving));
        assert!(hub.room().is_empty());
        // The next shell: none until it has said how the zones are.
        let (second, _line) = hub.attach().expect("attached");
        assert!(hub.room().is_empty());
        hub.report(second, snapshot(reserving));
        assert_eq!(hub.room().len(), 2);
    }

    #[test]
    fn a_snapshots_room_is_read_and_what_is_not_room_left_out() {
        assert!(room_of("").is_empty());
        assert!(room_of("[1]").is_empty());
        assert!(room_of(r#"{"zones":[]}"#).is_empty());
        let read = room_of(
            r#"{"reserved":[
                {"display":"edid:A","edge":"left","thickness":64},
                {"display":"edid:A","edge":"middle","thickness":64},
                {"display":"","edge":"top","thickness":64},
                {"display":"edid:A","edge":"top","thickness":0},
                {"display":"edid:A","edge":"top","thickness":-5},
                {"display":"edid:A","edge":"top"},
                {"edge":"top","thickness":10},
                {"display":"edid:B","edge":"bottom","thickness":48,"zones":["a","b"]}]}"#,
        );
        assert_eq!(
            read,
            [
                ShellRoom { display: "edid:A".to_string(), side: Side::Left, depth: 64 },
                ShellRoom { display: "edid:B".to_string(), side: Side::Bottom, depth: 48 },
            ]
        );
    }

    /// The next thing subscribers were told: its kind and fields.
    fn told(receiver: &mut broadcast::Receiver<ExecutionEvent>) -> (String, HashMap<String, String>) {
        match receiver.try_recv().expect("something was told").event {
            Some(execution_event::Event::RuntimeEvent(event)) => (event.kind, event.fields),
            other => panic!("told {other:?}"),
        }
    }

    const HIDE: &str = r#"{"command":"quick_hide"}"#;

    #[tokio::test]
    async fn without_a_shell_the_answer_is_that_zones_are_not_running() {
        let (hub, _events) = hub();
        let state = hub.state();
        assert!(!state.running && !state.stale);
        assert_eq!((state.sequence, state.json.as_str()), (0, ""));
        assert_eq!(state.message, NOT_RUNNING);

        let reply = hub.command(HIDE.to_string(), 1, 0).await.unwrap();
        assert!(!reply.success && !reply.delivered);
        assert_eq!(reply.error.as_deref(), Some(NOT_RUNNING));
    }

    #[tokio::test]
    async fn what_is_no_command_is_refused_before_anything_is_sent() {
        let (hub, _events) = hub();
        let (_session, mut requests) = hub.attach().unwrap();
        for text in ["", "quick_hide", "[]", "{}", r#"{"command":7}"#] {
            let refused = hub.command(text.to_string(), 1, 0).await.unwrap_err();
            assert_eq!(refused.code(), tonic::Code::InvalidArgument, "{text}");
        }
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_snapshot_is_kept_numbered_and_stale_once_its_shell_is_gone() {
        let (hub, mut events) = hub();
        let (first, _requests) = hub.attach().unwrap();
        assert_eq!(told(&mut events).0, "zones.attached");
        let state = hub.state();
        assert!(state.running && !state.stale);
        assert_eq!(state.message, "The shell has not reported the zones yet");

        hub.report(first, snapshot(r#"{"version":1,"zones":[]}"#));
        hub.report(first, snapshot(r#"{"version":1,"zones":[{"id":"a"}]}"#));
        let state = hub.state();
        assert!(state.running && !state.stale);
        assert_eq!((state.sequence, state.version), (2, 1));
        assert_eq!(state.json, r#"{"version":1,"zones":[{"id":"a"}]}"#);
        assert!(state.published_ms > 0 && state.message.is_empty());

        hub.detach(first);
        assert_eq!(told(&mut events).0, "zones.detached");
        let state = hub.state();
        assert!(!state.running && state.stale);
        assert_eq!(state.sequence, 2);
        assert!(state.json.contains("\"a\""));
        assert!(state.message.starts_with(NOT_RUNNING));

        // What a shell that is gone still says is not taken.
        hub.report(first, snapshot("{}"));
        assert_eq!(hub.state().sequence, 2);

        // The next shell: stale until it has said how the zones are.
        let (second, _requests) = hub.attach().unwrap();
        let state = hub.state();
        assert!(state.running && state.stale);
        hub.report(second, snapshot(r#"{"version":1}"#));
        let state = hub.state();
        assert!(state.running && !state.stale);
        assert_eq!(state.sequence, 3);
    }

    /// A snapshot as a shell with `items` items tells it: a tenth of them
    /// in each of two zones, the rest loose.
    fn large_snapshot(items: usize) -> String {
        let item = |index: usize| {
            serde_json::json!({
                "id": format!(r"C:\Users\ada\Desktop\Quarterly report {index:05}.xlsx"),
                "name": format!("Quarterly report {index:05}.xlsx"),
                "kind": "file",
                "path": format!(r"C:\Users\ada\Desktop\Quarterly report {index:05}.xlsx"),
            })
        };
        let tenth = items / 10;
        let zone = |id: &str, from: usize| {
            serde_json::json!({
                "id": id, "title": id, "kind": {"type": "manual"}, "visible": true,
                "item_count": tenth,
                "items": (from..from + tenth).map(item).collect::<Vec<_>>(),
            })
        };
        serde_json::json!({
            "version": 1,
            "enabled": true,
            "zones": [zone("work", 0), zone("play", tenth)],
            "loose": (2 * tenth..items).map(item).collect::<Vec<_>>(),
        })
        .to_string()
    }

    #[tokio::test]
    async fn the_state_can_be_asked_for_without_the_items_and_for_one_zone() {
        let (hub, _events) = hub();
        let (session, _requests) = hub.attach().unwrap();
        let whole = large_snapshot(5000);
        hub.report(session, snapshot(&whole));

        // All of it, as it came, where nothing less is asked for.
        assert_eq!(hub.state().json, whole);
        assert_eq!(hub.state_of(false, "").json, whole);

        let began = std::time::Instant::now();
        let small = hub.state_of(true, "");
        let took = began.elapsed();
        let read: serde_json::Value = serde_json::from_str(&small.json).unwrap();
        assert_eq!(read["zones"].as_array().unwrap().len(), 2);
        assert_eq!(read["zones"][0]["item_count"], 500);
        assert!(read["zones"][0].get("items").is_none());
        assert!(read.get("loose").is_none());
        assert_eq!(read["loose_count"], 4000);
        assert_eq!((small.sequence, small.running, small.stale), (1, true, false));
        println!(
            "5000 items: the whole snapshot is {} bytes, without items {} bytes, trimmed in {took:?}",
            whole.len(),
            small.json.len()
        );
        assert!(whole.len() > 500_000, "the measure is of a large one");
        assert!(small.json.len() < 1_000);
        // (Generous: an unoptimized build on a busy machine.)
        assert!(took < Duration::from_secs(2), "{took:?}");

        // One zone, with its items and without.
        let one: serde_json::Value = serde_json::from_str(&hub.state_of(false, "play").json).unwrap();
        assert_eq!(one["zones"].as_array().unwrap().len(), 1);
        assert_eq!(one["zones"][0]["id"], "play");
        assert_eq!(one["zones"][0]["items"].as_array().unwrap().len(), 500);
        assert_eq!(one["loose"].as_array().unwrap().len(), 4000);
        let one: serde_json::Value = serde_json::from_str(&hub.state_of(true, "play").json).unwrap();
        assert_eq!(one["zones"][0]["item_count"], 500);
        assert!(one["zones"][0].get("items").is_none());
        // A zone that is not there: none, and the rest as ever.
        let none: serde_json::Value = serde_json::from_str(&hub.state_of(true, "nowhere").json).unwrap();
        assert_eq!(none["zones"].as_array().unwrap().len(), 0);
        assert_eq!(none["enabled"], true);

        // What is no object is handed on as it came; its reader says so.
        hub.report(session, snapshot("[1]"));
        assert_eq!(hub.state_of(true, "work").json, "[1]");
        // A shell that does not count: counted here.
        hub.report(session, snapshot(r#"{"zones":[{"id":"a","items":[1,2]}],"loose":[1]}"#));
        assert_eq!(
            hub.state_of(true, "").json,
            r#"{"zones":[{"id":"a","item_count":2}],"loose_count":1}"#
        );
    }

    #[tokio::test]
    async fn one_shell_runs_the_zones_at_a_time() {
        let (hub, _events) = hub();
        let (first, requests) = hub.attach().unwrap();
        let refused = hub.attach().unwrap_err();
        assert_eq!(refused.code(), tonic::Code::AlreadyExists);
        // Its call ended on its side: the next one is let in, and the
        // late end of the first does not take the second out.
        drop(requests);
        let (second, _requests) = hub.attach().unwrap();
        hub.detach(first);
        assert!(hub.state().running);
        hub.detach(second);
        assert!(!hub.state().running);
    }

    #[tokio::test]
    async fn a_command_reaches_the_shell_and_its_answer_comes_back() {
        let (hub, _events) = hub();
        let (session, mut requests) = hub.attach().unwrap();
        let shell = Arc::clone(&hub);
        tokio::spawn(async move {
            let done = requests.recv().await.unwrap().unwrap();
            assert_eq!((done.json.as_str(), done.version), (HIDE, 1));
            shell.report(session, answer(done.request_id, None));
            let refused = requests.recv().await.unwrap().unwrap();
            assert!(refused.request_id > done.request_id);
            shell.report(session, answer(refused.request_id, Some("the zone is locked")));
            // (Kept, so that the line stays up.)
            std::future::pending::<()>().await;
        });
        let reply = hub.command(HIDE.to_string(), 1, 0).await.unwrap();
        assert!(reply.success && reply.delivered && reply.error.is_none());
        assert_eq!(reply.events, [r#"{"event":"layout_changed"}"#]);

        let reply = hub.command(HIDE.to_string(), 1, 0).await.unwrap();
        assert!(!reply.success && reply.delivered);
        assert_eq!(reply.error.as_deref(), Some("the zone is locked"));
        assert!(hub.lock().waiting.is_empty());
    }

    #[tokio::test]
    async fn a_shell_that_does_not_answer_is_not_waited_for_for_ever() {
        let (hub, _events) = hub();
        let (session, mut requests) = hub.attach().unwrap();
        let began = std::time::Instant::now();
        let reply = hub.command(HIDE.to_string(), 1, 60).await.unwrap();
        assert!(began.elapsed() < Duration::from_secs(3));
        assert!(!reply.success && reply.delivered);
        assert!(reply.error.unwrap().starts_with("The shell did not answer within 60 ms"));
        assert!(hub.lock().waiting.is_empty());
        // An answer that comes after all is nobody's.
        let late = requests.recv().await.unwrap().unwrap();
        hub.report(session, answer(late.request_id, None));
    }

    #[tokio::test]
    async fn whoever_waits_is_told_when_the_shell_goes() {
        let (hub, _events) = hub();
        let (session, mut requests) = hub.attach().unwrap();
        let shell = Arc::clone(&hub);
        tokio::spawn(async move {
            let _ = requests.recv().await;
            shell.detach(session);
        });
        let reply = hub.command(HIDE.to_string(), 1, 0).await.unwrap();
        assert!(!reply.success && reply.delivered);
        assert_eq!(
            reply.error.as_deref(),
            Some("The shell that runs the zones went away before it answered")
        );
    }

    #[tokio::test]
    async fn events_are_passed_on_under_their_names() {
        let (hub, mut events) = hub();
        let (session, _requests) = hub.attach().unwrap();
        assert_eq!(told(&mut events).0, "zones.attached");
        hub.report(session, snapshot("{}"));
        let event = |name: &str| ZonesReport {
            report: Some(zones_report::Report::Event(ZonesEventReport {
                version: 1,
                name: name.to_string(),
                json: r#"{"event":"zone_created","zone":"a"}"#.to_string(),
            })),
        };
        hub.report(session, event("zone_created"));
        let (kind, fields) = told(&mut events);
        assert_eq!(kind, "zones.zone_created");
        assert_eq!(fields["json"], r#"{"event":"zone_created","zone":"a"}"#);
        assert_eq!((fields["sequence"].as_str(), fields["version"].as_str()), ("1", "1"));
        // A name that is no plain word does not become part of a kind.
        hub.report(session, event("a.b c"));
        assert_eq!(told(&mut events).0, "zones.abc");
        hub.report(session, event(""));
        assert_eq!(told(&mut events).0, "zones.event");
        // Nothing is passed on from a shell that is not the attached one.
        hub.report(session + 1, event("zone_created"));
        assert!(events.try_recv().is_err());
    }
}
