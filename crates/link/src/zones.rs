//! The line's part for the zones on the desktop.
//!
//! The zones live in the shell, and other programs reach them through the
//! window manager. The program that runs the zones says so once
//! ([`crate::Link::host_zones`]); from then on the line keeps a call to the
//! window manager open over which it tells how the zones are
//! ([`crate::Link::publish_zones`]), what happens to them
//! ([`crate::Link::publish_zones_event`]) and what came of what was asked
//! ([`crate::Link::answer_zones`]); and it is told what is asked of them
//! ([`crate::Link::poll_zones`]).
//!
//! The call is kept as the line is: when the window manager goes away and
//! comes back, it is made again and told the snapshot that was published
//! last, so the program that runs the zones publishes when something
//! changes and never has to ask whether anyone knows.
//!
//! A program that does not run the zones (the settings program) calls none
//! of this, and nothing here happens for it.

use gevrek_protocol::proto;
use gevrek_protocol::WMClient;
use gevrek_zones::ipc::{self, Answer, ZonesSnapshot};
use gevrek_zones::ZonesEvent;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;

/// The capability of a window manager that has the zones' call.
pub const FEATURE: &str = "zones_ipc_v1";

/// How many reports may wait for the window manager. More than that, and
/// events are let go: the snapshot that follows them says how things are.
const WAITING: usize = 256;

/// What the program that runs the zones tells the window manager.
pub enum ZonesTell {
    /// This program runs the zones, from now on.
    Host,
    /// How the zones are now.
    Snapshot(Box<ZonesSnapshot>),
    /// Something happened to them.
    Event(ZonesEvent),
    /// What came of something that was asked with a number.
    Answer { request: u64, answer: Answer },
}

/// Something asked of the zones.
#[derive(Debug, Clone, PartialEq)]
pub struct ZonesAsk {
    /// The number to answer with ([`crate::Link::answer_zones`]): someone
    /// waits for the answer. None: it came from a key, and nothing is to
    /// be answered.
    pub request: Option<u64>,
    pub ask: ipc::Request,
}

/// What the program that runs the zones is told.
#[derive(Debug, Clone, PartialEq)]
pub enum ZonesHeard {
    /// The window manager listens: it has been told the last snapshot, if
    /// one was published.
    Attached,
    /// It does not (any more), and why. What is published meanwhile is
    /// kept (the last snapshot) or let go (events).
    Detached(String),
    Asked(ZonesAsk),
}

#[derive(Clone)]
pub(crate) struct ZonesTeller {
    pub heard: Sender<ZonesHeard>,
    pub wake: std::sync::Arc<dyn Fn() + Send + Sync>,
}

impl ZonesTeller {
    fn tell(&self, heard: ZonesHeard) {
        let _ = self.heard.send(heard);
        (self.wake)();
    }
}

/// The call to the window manager while it is up.
pub(crate) struct Call {
    reports: tokio::sync::mpsc::Sender<proto::ZonesReport>,
    requests: tonic::Streaming<proto::ZonesRequest>,
}

/// What the line keeps for the zones, over every connection.
pub(crate) struct ZonesSide {
    pub tells: UnboundedReceiver<ZonesTell>,
    /// The window's end is gone: nothing more is told.
    pub closed: bool,
    teller: ZonesTeller,
    hosting: bool,
    /// The snapshot published last, told anew with every call.
    latest: Option<proto::ZonesReport>,
    pub call: Option<Call>,
    /// When the call is tried again.
    pub retry_at: Option<Instant>,
    /// Why there is no call, as it was last said (said once, not with
    /// every try).
    said: Option<String>,
}

/// The report a snapshot travels as.
pub fn snapshot_report(snapshot: &ZonesSnapshot) -> proto::ZonesReport {
    proto::ZonesReport {
        report: Some(proto::zones_report::Report::Snapshot(proto::ZonesSnapshotReport {
            version: snapshot.version,
            json: serde_json::to_string(snapshot).unwrap_or_default(),
        })),
    }
}

/// The report an event travels as.
pub fn event_report(event: &ZonesEvent) -> proto::ZonesReport {
    proto::ZonesReport {
        report: Some(proto::zones_report::Report::Event(proto::ZonesEventReport {
            version: ipc::VERSION,
            name: ipc::event_name(event),
            json: serde_json::to_string(event).unwrap_or_default(),
        })),
    }
}

/// The report an answer travels as.
pub fn answer_report(request: u64, answer: &Answer) -> proto::ZonesReport {
    proto::ZonesReport {
        report: Some(proto::zones_report::Report::Answer(proto::ZonesCommandAnswer {
            request_id: request,
            success: answer.ok,
            error: answer.error.clone(),
            events: answer
                .events
                .iter()
                .filter_map(|event| serde_json::to_string(event).ok())
                .collect(),
        })),
    }
}

/// What the window manager sent, read: what is asked, or (where it cannot
/// be read and someone waits) the answer that says why not. Nothing where
/// it cannot be read and nobody waits.
pub fn read_request(request: proto::ZonesRequest) -> Result<ZonesAsk, Option<proto::ZonesReport>> {
    let number = (request.request_id != 0).then_some(request.request_id);
    let read = if request.version > ipc::VERSION {
        Err(format!(
            "this shell reads zones commands of version {}, and this one is of version {}",
            ipc::VERSION,
            request.version
        ))
    } else {
        ipc::Request::from_json(&request.json)
    };
    match read {
        Ok(ask) => Ok(ZonesAsk { request: number, ask }),
        Err(why) => {
            log::warn!("something asked of the zones was not read: {why}");
            Err(number.map(|number| answer_report(number, &Answer::refused(why))))
        }
    }
}

/// What a key asked of the zones, from the event the window manager tells
/// of it (`shell.command` with the action `zones`). None for every other
/// event, and for one whose command cannot be read.
pub fn read_event(kind: &str, fields: &HashMap<String, String>) -> Option<ZonesAsk> {
    if kind != "shell.command"
        || fields.get("action").map(String::as_str) != Some(gevrek_model::command::SHELL_ZONES)
    {
        return None;
    }
    match ipc::Request::from_json(fields.get("json")?) {
        Ok(ask) => Some(ZonesAsk { request: None, ask }),
        Err(why) => {
            log::warn!("a key's zones command was not read: {why}");
            None
        }
    }
}

impl ZonesSide {
    pub fn new(tells: UnboundedReceiver<ZonesTell>, teller: ZonesTeller) -> Self {
        Self {
            tells,
            closed: false,
            teller,
            hosting: false,
            latest: None,
            call: None,
            retry_at: None,
            said: None,
        }
    }

    #[cfg(test)]
    pub fn hosting(&self) -> bool {
        self.hosting
    }

    /// Take what was told while there is no window manager: that this
    /// program runs the zones and the last snapshot are kept, the rest is
    /// let go.
    pub fn keep(&mut self, tell: ZonesTell) {
        match tell {
            ZonesTell::Host => self.hosting = true,
            ZonesTell::Snapshot(snapshot) => self.latest = Some(snapshot_report(&snapshot)),
            ZonesTell::Event(_) | ZonesTell::Answer { .. } => {}
        }
    }

    /// Take what was told while the window manager is there.
    pub async fn take(&mut self, tell: ZonesTell, client: &mut WMClient, features: &[String]) {
        let report = match tell {
            ZonesTell::Host => {
                if !self.hosting {
                    self.hosting = true;
                    self.attach(client, features).await;
                }
                return;
            }
            ZonesTell::Snapshot(snapshot) => {
                let report = snapshot_report(&snapshot);
                self.latest = Some(report.clone());
                report
            }
            ZonesTell::Event(event) => event_report(&event),
            ZonesTell::Answer { request, answer } => answer_report(request, &answer),
        };
        self.send(report);
    }

    fn send(&mut self, report: proto::ZonesReport) {
        use tokio::sync::mpsc::error::TrySendError;
        let Some(call) = &self.call else {
            return;
        };
        match call.reports.try_send(report) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                log::warn!("the window manager is not taking what is told of the zones")
            }
            Err(TrySendError::Closed(_)) => self.lost("the zones' call ended".to_string()),
        }
    }

    /// Make the call, if this program runs the zones. Where it cannot be
    /// made it is tried again in a while, except where the window manager
    /// does not know it at all.
    pub async fn attach(&mut self, client: &mut WMClient, features: &[String]) {
        self.retry_at = None;
        if !self.hosting || self.call.is_some() {
            return;
        }
        if !features.iter().any(|feature| feature == FEATURE) {
            return self.say("this window manager does not know the zones' call (it is older)".to_string());
        }
        let (reports, outgoing) = tokio::sync::mpsc::channel(WAITING);
        if let Some(latest) = &self.latest {
            let _ = reports.try_send(latest.clone());
        }
        let outgoing = tokio_stream::wrappers::ReceiverStream::new(outgoing);
        let made = tokio::time::timeout(Duration::from_secs(5), client.attach_zones(outgoing)).await;
        match made {
            Ok(Ok(requests)) => {
                self.call = Some(Call { reports, requests });
                self.said = None;
                self.teller.tell(ZonesHeard::Attached);
            }
            Ok(Err(error)) => {
                self.retry_at = Some(Instant::now() + crate::RETRY);
                self.say(format!("the zones' call was not made: {error}"));
            }
            Err(_) => {
                self.retry_at = Some(Instant::now() + crate::RETRY);
                self.say("the zones' call was not made: no answer".to_string());
            }
        }
    }

    /// The call is over while the line is still up: said, and tried again.
    pub fn lost(&mut self, why: String) {
        if self.call.take().is_some() {
            self.retry_at = Some(Instant::now() + crate::RETRY);
            self.say(why);
        }
    }

    /// The line itself is down: the call went with it, and is made again
    /// when the line is.
    pub fn line_down(&mut self) {
        self.retry_at = None;
        if self.call.take().is_some() {
            self.say("the window manager went away".to_string());
        }
    }

    fn say(&mut self, why: String) {
        if self.said.as_deref() != Some(&why) {
            log::info!("zones: {why}");
            self.said = Some(why.clone());
            self.teller.tell(ZonesHeard::Detached(why));
        }
    }

    /// Something came over the call.
    pub fn asked(&mut self, request: proto::ZonesRequest) {
        match read_request(request) {
            Ok(ask) => self.teller.tell(ZonesHeard::Asked(ask)),
            Err(Some(refusal)) => self.send(refusal),
            Err(None) => {}
        }
    }

    /// Something happened in the window manager: a key's zones command is
    /// told as what is asked.
    pub fn heard(&self, kind: &str, fields: &HashMap<String, String>) {
        if !self.hosting {
            return;
        }
        if let Some(ask) = read_event(kind, fields) {
            self.teller.tell(ZonesHeard::Asked(ask));
        }
    }
}

/// The next thing the window manager sends over the call; never, while
/// there is none.
pub(crate) async fn next(
    call: &mut Option<Call>,
) -> Option<Result<proto::ZonesRequest, tonic::Status>> {
    use tokio_stream::StreamExt;
    match call {
        Some(call) => call.requests.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::{Command, ZoneId};

    fn request(id: u64, version: u32, json: &str) -> proto::ZonesRequest {
        proto::ZonesRequest {
            request_id: id,
            version,
            json: json.to_string(),
        }
    }

    fn answer_of(report: proto::ZonesReport) -> proto::ZonesCommandAnswer {
        match report.report {
            Some(proto::zones_report::Report::Answer(answer)) => answer,
            other => panic!("not an answer: {other:?}"),
        }
    }

    #[test]
    fn what_is_asked_is_read_with_its_number() {
        let ask = read_request(request(7, 1, r#"{"command":"peek","on":true}"#)).unwrap();
        assert_eq!(ask.request, Some(7));
        assert_eq!(ask.ask, ipc::Request::Command(Command::Peek { on: true }));
        // Without a number nobody waits; and a version that is not said
        // is read as this one.
        let ask = read_request(request(0, 0, r#"{"command":"restore_system_icons"}"#)).unwrap();
        assert_eq!((ask.request, ask.ask), (None, ipc::Request::RestoreSystemIcons));
    }

    #[test]
    fn what_cannot_be_read_is_answered_where_someone_waits() {
        let refusal = read_request(request(3, 1, r#"{"command":"fly"}"#)).unwrap_err();
        let answer = answer_of(refusal.expect("someone waits"));
        assert_eq!(answer.request_id, 3);
        assert!(!answer.success);
        assert!(answer.error.unwrap().starts_with("not a zones command"));
        // Nobody waits: nothing is answered.
        assert_eq!(read_request(request(0, 1, "{")).unwrap_err(), None);
        // A newer shape is not guessed at.
        let newer = read_request(request(4, 99, r#"{"command":"quick_hide"}"#)).unwrap_err();
        assert!(answer_of(newer.unwrap()).error.unwrap().contains("version 99"));
    }

    #[test]
    fn a_keys_command_is_read_from_the_event_that_tells_of_it() {
        let fields = |action: &str, json: &str| -> HashMap<String, String> {
            [("action", action), ("json", json)]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect()
        };
        let hide = r#"{"command":"quick_hide"}"#;
        let ask = read_event("shell.command", &fields("zones", hide)).unwrap();
        assert_eq!(ask.request, None);
        assert_eq!(ask.ask, ipc::Request::Command(Command::QuickHide { hidden: None }));
        // As the window manager writes it for the command of a binding.
        let command: gevrek_model::command::DispatchCommand =
            serde_json::from_str(&format!(r#"{{"command":"zones","action":{hide}}}"#)).unwrap();
        let told: HashMap<String, String> = command.shell_fields().unwrap().into_iter().collect();
        assert!(read_event("shell.command", &told).is_some());
        // Other events and other actions are not for the zones.
        assert!(read_event("window.created", &fields("zones", hide)).is_none());
        assert!(read_event("shell.command", &fields("launcher", hide)).is_none());
        assert!(read_event("shell.command", &fields("zones", "nonsense")).is_none());
    }

    #[test]
    fn reports_carry_the_zones_json() {
        let snapshot = ZonesSnapshot {
            enabled: true,
            ..ZonesSnapshot::default()
        };
        match snapshot_report(&snapshot).report {
            Some(proto::zones_report::Report::Snapshot(report)) => {
                assert_eq!(report.version, ipc::VERSION);
                assert_eq!(ZonesSnapshot::from_json(&report.json).unwrap(), snapshot);
            }
            other => panic!("not a snapshot: {other:?}"),
        }
        let created = ZonesEvent::ZoneCreated { zone: ZoneId::new("work") };
        match event_report(&created).report {
            Some(proto::zones_report::Report::Event(report)) => {
                assert_eq!(report.name, "zone_created");
                assert_eq!(serde_json::from_str::<ZonesEvent>(&report.json).unwrap(), created);
            }
            other => panic!("not an event: {other:?}"),
        }
        let refused = answer_of(answer_report(9, &Answer::refused("the zone is locked")));
        assert_eq!((refused.request_id, refused.success), (9, false));
        assert_eq!(refused.error.as_deref(), Some("the zone is locked"));
        let done = Answer {
            ok: true,
            error: None,
            events: vec![created.clone()],
        };
        let done = answer_of(answer_report(10, &done));
        assert!(done.success && done.error.is_none());
        assert_eq!(serde_json::from_str::<ZonesEvent>(&done.events[0]).unwrap(), created);
    }

    #[test]
    fn what_is_told_without_a_window_manager_is_kept_or_let_go() {
        let (_tells, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (heard, _heard) = std::sync::mpsc::channel();
        let mut side = ZonesSide::new(
            receiver,
            ZonesTeller {
                heard,
                wake: std::sync::Arc::new(|| {}),
            },
        );
        assert!(!side.hosting());
        side.keep(ZonesTell::Event(ZonesEvent::LayoutChanged));
        side.keep(ZonesTell::Answer {
            request: 1,
            answer: Answer::done(),
        });
        assert!(!side.hosting() && side.latest.is_none());
        side.keep(ZonesTell::Host);
        side.keep(ZonesTell::Snapshot(Box::default()));
        let later = ZonesSnapshot {
            peeking: true,
            ..ZonesSnapshot::default()
        };
        side.keep(ZonesTell::Snapshot(Box::new(later.clone())));
        assert!(side.hosting());
        assert_eq!(side.latest, Some(snapshot_report(&later)));
    }
}
