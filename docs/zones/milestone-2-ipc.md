# Zones: milestone 2, the path between scripts and the zones

Written 2026-11-19. The part of milestone 2 that `design.md` calls "the
publish RPCs of decision C": how a script, the command line or a key
reaches the zones, which live in the shell, through the window manager,
which stays the only endpoint.

**No shell runs zones yet.** This is the path and both of its ends as
libraries: the window manager's side (`crates/server/src/zones.rs`), the
shell's side (`crates/link/src/zones.rs`), the shapes
(`crates/zones/src/ipc.rs`) and the command line. The shell's zones
component calls the link's functions; until it does, every answer is
"zones are not running".

## The contract in one picture

```
gevrek zones send ─┐                                   ┌─ poll_zones() -> Asked{request, ask}
any gRPC client   ─┼─ SendZonesCommand ──► WM ── AttachZones (one call, ──► shell (gevrek-link)
{"command":"zones"}┘   (waits, 5 s)         │     both directions)     ◄── answer_zones(request, answer)
                                            │                          ◄── publish_zones(snapshot)
gevrek zones list ──── GetZonesState ──────►│ keeps the last snapshot  ◄── publish_zones_event(event)
gevrek zones events ── SubscribeToEvents ◄──┘ passes events on as `zones.<name>`

a key (binding) ── the WM's loop ── `shell.command` {action: zones, json} ──► shell: Asked{request: None}
```

## What was chosen, and why

- **One call that lasts, for the shell** (`AttachZones`, a stream each
  way), not a handful of single calls. The shell must be known to be
  there: "no shell" and "the shell is gone, this is old" are answers a
  script needs, and the end of a lasting call is the one sign of it that
  needs no timer and no heartbeat. Commands go to the one shell that runs
  the zones and to nobody else, and its answers come back the same way.
- **Events on the existing stream as `runtime_event`s**, kind
  `zones.<name>`, not a new arm of `ExecutionEvent`. A new arm breaks
  every client that reads the stream exhaustively (the command line's
  `log`, extension hosts); a new kind of an event they already read does
  not. This is the envelope `extension_events_v1` already promises.
- **The zones' data is JSON inside the messages, under a version**
  (`gevrek_zones::ipc::VERSION`, 1), as the command call already carries
  commands. The shapes are the core's serde types and still move; the
  window manager never reads them (beyond seeing that a command is an
  object with a `command`), so it needs no change when they do.
- **The command stays in `gevrek-zones`; the model carries it unread.**
  `Command` names `ZoneId`, `ItemId`, `DisplayId`, `Cell`, `Edges`,
  `Scope`, which are the zones crate's, and the zones crate is built on
  the model, so the model cannot name `Command`. `DispatchCommand::Zones
  { action: serde_json::Value }` holds it as it was written, and the
  shell's line reads it. A binding holds a `DispatchCommand`, so
  everything a pointer does can be bound to a key.
- **Asked and answered by a number.** The window manager numbers each
  command it sends (`request_id`), the shell echoes the number in its
  answer, and the window manager waits for it: 5 seconds, or what the
  asker says (never more than 30). No answer in time is itself an answer
  ("The shell did not answer within ... ms; what was asked may still be
  carried out"). An answer that comes later is dropped.
- **A key does not wait.** A binding's command goes through the window
  manager's loop as every binding does and is told as the event
  `shell.command` with the action `zones` and the command as `json`,
  exactly as `Shell`, `Notify` and `Osd` are. Nobody waits for a key, so
  there is no number and no answer. The shell's line turns both ways into
  the same thing for the shell (`ZonesHeard::Asked`).
- **`restore-explorer` is not a command of the core.** The core refuses
  every command while the zones are switched off, and putting Explorer's
  icons back must work then above all. So it is a word of its own beside
  the core's (`{"command": "restore_system_icons"}`,
  `ipc::Request::RestoreSystemIcons`) that the shell carries out on the
  backend directly (`ZonesBackend::restore_system_icons`).
- **One shell at a time.** A second `AttachZones` is refused
  (`ALREADY_EXISTS`) while the first is there; its line tries again every
  3 seconds. Letting the later one in would have two shells take the line
  from each other for ever.

## The protocol (`crates/protocol/proto/gevrek.proto`, all appended)

```proto
rpc AttachZones(stream ZonesReport) returns (stream ZonesRequest);
rpc GetZonesState(Empty) returns (ZonesStateResponse);
rpc SendZonesCommand(ZonesCommandDocument) returns (ZonesCommandReply);

message ZonesReport { oneof report {
  ZonesSnapshotReport snapshot = 1; ZonesEventReport event = 2; ZonesCommandAnswer answer = 3; } }
message ZonesSnapshotReport { uint32 version = 1; string json = 2; }
message ZonesEventReport    { uint32 version = 1; string name = 2; string json = 3; }
message ZonesCommandAnswer  { uint64 request_id = 1; bool success = 2;
                              optional string error = 3; repeated string events = 4; }
message ZonesRequest        { uint64 request_id = 1; uint32 version = 2; string json = 3; }
message ZonesStateResponse  { bool running = 1; bool stale = 2; uint32 version = 3;
                              uint64 sequence = 4; string json = 5;
                              int64 published_ms = 6; string message = 7; }
message ZonesCommandDocument { string json = 1; uint32 version = 2; uint32 timeout_ms = 3; }
message ZonesCommandReply   { bool success = 1; optional string error = 2;
                              repeated string events = 3; bool delivered = 4; }
```

Capability: `zones_ipc_v1` (beside `zones_v1`). The command `zones` has no
number among the typed dispatchers, as `shell`, `notify` and `osd` have
none: it is sent as JSON (`DispatchCommand`, the call) or by
`SendZonesCommand`.

### The state

`GetZonesState` answers, always (it never fails for want of a shell):

| | `running` | `stale` | `json` | `message` |
|---|---|---|---|---|
| no shell, and never one | false | false | empty | `Zones are not running: no gevrek-shell that runs them is attached to this window manager` |
| a shell, nothing told yet | true | false | empty | `The shell has not reported the zones yet` |
| a shell that has told | true | false | the snapshot | empty |
| the shell is gone | false | true | its last snapshot | the first sentence, and `This is what the shell that ran them last reported` |
| a new shell, nothing told yet | true | true | the old shell's last | `The shell has not reported the zones yet. This is what the shell before it last reported` |

`sequence` counts the snapshots this window manager was given, from 1,
over every shell; it only grows while the window manager runs.

`json` is a `gevrek_zones::ipc::ZonesSnapshot`:

```json
{
  "version": 1,
  "enabled": true,
  "quick_hidden": false,
  "peeking": false,
  "profile": { "displays": [ { "id": "DEL-A1B2-77", "width": 1920, "height": 1080, "scale": 1000 } ] },
  "displays": [ { "id": "DEL-A1B2-77", "rect": { "x": 0, "y": 0, "width": 1920, "height": 1080 },
                  "scale": 1.0, "primary": true } ],
  "zones": [ {
    "id": "work", "title": "Work", "kind": { "type": "manual" },
    "display": "DEL-A1B2-77",
    "rect": { "x": 40, "y": 40, "width": 320, "height": 240 },
    "geometry": { "display": "DEL-A1B2-77", "anchor": "top_left",
                  "offset_x": 40, "offset_y": 40, "width": 320, "height": 240 },
    "visible": true, "rolled_up": false, "always_visible": false,
    "locked": { "geometry": false, "members": false },
    "front": true, "view": "grid", "icon_size": 48,
    "item_count": 1,
    "items": [ { "id": "/home/ada/Desktop/report.pdf", "name": "report.pdf",
                 "kind": "file", "path": "/home/ada/Desktop/report.pdf" } ]
  } ],
  "loose": [ { "id": "/home/ada/Desktop/notes.txt", "name": "notes.txt",
               "kind": "file", "path": "/home/ada/Desktop/notes.txt" } ]
}
```

- `profile` is the key of the arrangement in use: the set of displays by
  their stable ids, sizes and scales (thousandths), and the `workspace`
  where each has its own. Absent until the displays are known.
- A zone's `display` is the stable id. `rect` is where it is drawn on the
  desktop in logical units (the title alone when rolled up); `geometry` is
  the same as it is remembered, from a corner or side of that display.
  Tabs share the place of the first of their group (`tab_group`, `front`).
- `visible` is false for a zone hidden on these displays, and for every
  zone that is not `always_visible` while `quick_hidden` (unless
  `peeking`).
- Switched off (`enabled` false), the zones are listed as configured,
  with no `display`, `rect` or `geometry`, and none is `visible`.
- A reader refuses a `version` it does not know
  (`ZonesSnapshot::from_json`).

### Commands

`SendZonesCommand { json, version, timeout_ms }`; `json` is a
`gevrek_zones::Command` or one of the words beside it:

```json
{"command": "create_zone", "title": "Play"}
{"command": "quick_hide"}
{"command": "peek", "on": true}
{"command": "add_item", "zone": "work", "item": "/home/ada/Desktop/report.pdf"}
{"command": "restore_system_icons"}
```

The reply:

| | `success` | `delivered` | `error` | `events` |
|---|---|---|---|---|
| carried out | true | true | | what happened, `ZonesEvent`s as JSON |
| refused by the zones | false | true | the core's reason (`no zone is called nowhere`, `zones are switched off (desktop.zones.enabled)`) | what the core said, the refusal among it |
| cannot be read by the shell | false | true | `not a zones command: unknown variant ...` | |
| no shell | false | false | `Zones are not running: ...` | |
| no answer in time | false | true | `The shell did not answer within 5000 ms; what was asked may still be carried out` | |
| the shell went while asked | false | true | `The shell that runs the zones went away before it answered` | |

Only what is no JSON object with a `command` fails the call itself
(`INVALID_ARGUMENT`), and a locked session refuses it
(`FAILED_PRECONDITION`), as it refuses every command.

The same through the call every client has, for a script that knows only
that one, and in a binding:

```json
{"command": "zones", "action": {"command": "quick_hide"}}
```

Sent by `DispatchCommand` (the call; `gevrek dispatch '...'`), the window
manager hands it to the shell and answers `success` or the refusal reason
in the `StatusResponse` (without the events). In a binding it is told to
the shell as the event `shell.command` with `action: zones` and the
command as `json`, and nothing is answered.

### Events

Each `ZonesEvent` the shell publishes reaches `SubscribeToEvents` as a
`runtime_event`:

```json
{"event": "zones.zone_created", "timestamp_ms": 1795099609753, "window_handle": null,
 "fields": {"json": "{\"event\":\"zone_created\",\"zone\":\"zone-1\"}", "version": "1", "sequence": "3"}}
```

(as `gevrek log --json` prints it). `kind` is `zones.` and the event's own
name; `fields.json` is the `ZonesEvent`; `fields.sequence` is the number
of the snapshot there was when it happened, so the snapshot that shows it
has a higher one. The window manager adds two of its own: `zones.attached`
and `zones.detached` (field `session`), when a shell begins and ends
running the zones.

## The shell's side (`gevrek-link`)

On the `Link` the shell already has. Nothing here blocks; all of it can
be called from the thread that draws.

```rust
impl Link {
    /// This program runs the zones. Once, at any time; kept over every
    /// loss of the window manager.
    pub fn host_zones(&self);
    /// How the zones are now. After everything that changes them and
    /// once at the start; the last one is told again by the line itself
    /// whenever the window manager comes (back).
    pub fn publish_zones(&self, snapshot: gevrek_zones::ipc::ZonesSnapshot);
    /// Something that happened (each `Effect::Notify`).
    pub fn publish_zones_event(&self, event: gevrek_zones::ZonesEvent);
    /// The answer to what was asked with a number.
    pub fn answer_zones(&self, request: u64, answer: gevrek_zones::ipc::Answer);
    /// What was asked, and whether the window manager listens.
    pub fn poll_zones(&self) -> Vec<gevrek_link::ZonesHeard>;
    /// The first four from another thread.
    pub fn zones_teller(&self) -> impl Fn(gevrek_link::ZonesTell) + Send + 'static;
}

pub enum ZonesHeard { Attached, Detached(String), Asked(ZonesAsk) }
pub struct ZonesAsk { pub request: Option<u64>, pub ask: gevrek_zones::ipc::Request }
pub enum ZonesTell { Host, Snapshot(Box<ZonesSnapshot>), Event(ZonesEvent),
                     Answer { request: u64, answer: Answer } }
```

and in `gevrek_zones::ipc`:

```rust
pub const VERSION: u32 = 1;
pub enum Request { Command(Command), RestoreSystemIcons }
pub struct Answer { pub ok: bool, pub error: Option<String>, pub events: Vec<ZonesEvent> }
impl Answer { pub fn done() -> Self; pub fn refused(reason: impl Into<String>) -> Self;
              pub fn of(effects: &[Effect]) -> Self; }
pub fn events_of(effects: &[Effect]) -> Vec<ZonesEvent>;
impl ZonesSnapshot { pub fn of(core: &ZonesCore, now: u64) -> Self;
                     pub fn from_json(text: &str) -> Result<Self, String>; }
```

What the zones component does with them:

```rust
link.host_zones();                                    // at the start
link.publish_zones(ZonesSnapshot::of(&core, now));    // at the start, and after every change

for heard in link.poll_zones() {                      // beside link.poll()
    let ZonesHeard::Asked(ZonesAsk { request, ask }) = heard else { continue };
    let answer = match ask {
        Request::Command(command) => {
            let effects = host.command(&mut core, &mut backend, command, now);
            for event in ipc::events_of(&effects) { link.publish_zones_event(event); }
            // ... SaveConfig, SaveState, redraw, as for a command of the pointer ...
            Answer::of(&effects)
        }
        Request::RestoreSystemIcons => match backend.restore_system_icons() {
            Ok(()) => Answer::done(),
            Err(error) => Answer::refused(error.to_string()),
        },
    };
    if let Some(request) = request { link.answer_zones(request, answer); }
    link.publish_zones(ZonesSnapshot::of(&core, now));
}
```

- Publish events before the snapshot that shows them.
- Not a snapshot for every movement of a drag: at its end, or a few times
  a second. A snapshot holds every item's name.
- A command that cannot be read never reaches the shell: the line
  answers it itself where someone waits, and writes it to the log.
- `Detached` says why the window manager does not listen (it is gone, it
  is older and does not know the call, another shell runs the zones). The
  line tries again by itself; there is nothing to do but perhaps say it.
- A program that does not call `host_zones` (the settings program) is
  never told anything here and never attaches.

## The command line

`gevrek zones` is new and plural; `gevrek zone 2`, `zone-next` and
`zone-previous` are still the snap zones of a window (hidden names of
`snap-zone...`).

```
$ gevrek zones list
Zones: 1 · profile: DEL-A1B2-77 1920x1080@1.00 · snapshot 1
  ID    TITLE  KIND    DISPLAY      RECT           ITEMS  FLAGS
  work  Work   manual  DEL-A1B2-77  40,40 320x240      1
Loose items: 1

$ gevrek zones show work
work  "Work"  manual
  display    DEL-A1B2-77
  rect       40,40 320x240
  anchored   top_left +40,+40 320x240
  visible    yes
  rolled up  no
  locked     geometry: no, members: no
  items (1):
    file      report.pdf  /home/ada/Desktop/report.pdf

$ gevrek zones send '{"command":"create_zone","title":"Play"}'
Done
{"event":"zone_created","zone":"zone-1"}

$ gevrek zones send '{"command":"delete_zone","zone":"nowhere"}'
Error: no zone is called nowhere                      (exit code 1)

$ gevrek zones events
[zones] a shell runs the zones
[zones] {"event":"zone_created","zone":"zone-1"}

$ gevrek zones restore-explorer
The system's own desktop icons were put back

$ gevrek zones list                                   (no shell)
Error: Zones are not running: no gevrek-shell that runs them is attached to this window manager
```

- `FLAGS`: `hidden`, `rolled-up`, `locked`, `locked(geometry)`,
  `locked(members)`, `always-visible`, `tab-behind`. Before the list, a
  line each where it holds: `Stale: ...`, `Zones are switched off
  (desktop.zones.enabled)`, `Zones are hidden (quick hide)`, `Zones are
  shown above the windows (peek)`.
- `list --json` prints `{running, stale, sequence, version, published_ms,
  message, state}` with the snapshot as `state` (null where there is
  none) and ends well also where no shell runs: the JSON says it.
- `show <id> --json` prints `{running, stale, sequence, zone}`.
- `send '<json>' [--json] [--timeout-ms N]`: with `--json` the reply as
  `{success, delivered, error, events}`; the exit code is 1 where
  `success` is false, in both forms.
- `events [--json]`: with `--json` one object a line, `{event,
  timestamp_ms, sequence, data}`, `data` being the `ZonesEvent`.

The full command line (a word for each command, completions) is
milestone 5.

## What changed outside the path

- `gevrek_model::command::DispatchCommand::Zones { action:
  serde_json::Value }` and `SHELL_ZONES`, with its `shell_fields`. It is
  listed where every command of its kind is: the two lists of the
  engine's dispatcher (`executor/dispatch.rs`, `commands_other.rs`: "the
  runtime's"), the list of commands without a typed request
  (`protocol/src/conversions.rs`), and one line each in the two window
  managers' arm that tells the shell (`backend-win32/src/runtime/mod.rs`,
  `backend-smithay/src/host.rs`), beside `Notify`, `Osd` and
  `AppShortcut`.
- `crates/zones`: the new module `ipc` only. `core.rs`, `host.rs` and the
  backend trait are as they were.
- `gevrek-link` depends on `gevrek-zones` and `tonic`; the root package on
  `gevrek-zones` (and on `gevrek-link` for its tests).

## What was verified

By tests, on Windows and on Linux (Fedora in WSL), where the workspace
was also checked whole (`cargo check --workspace --exclude gevrek-win32
--all-targets`).

- `gevrek-zones` (`ipc`, 5 tests): a request is a core command or the
  other word and says why when it is neither; an answer is the core's
  refusal or what happened; a snapshot of a core (zones, items, loose
  items, rolled up, locked, quick-hidden, switched off) survives its
  text, and another version is refused.
- `gevrek-server` (`zones`, 8 tests): no shell; what is no command; a
  snapshot kept, numbered, stale after its shell, fresh after the next
  one's first; one shell at a time; a command to the shell and its answer
  back, done and refused; the wait that ends; the asker told when the
  shell goes; events passed on under their names.
- `gevrek-link` (`zones`, 5 tests): what is asked read with its number;
  what cannot be read answered where someone waits; a key's command read
  from its event, as the model writes it; the reports; what is kept
  while there is no window manager.
- `gevrek-model`: the command read, written and told.
- `tests/grpc_integration_tests.rs` (4 tests, a real server on a port of
  its own, the real client, the real line, the built `gevrek` program):
  no shell, by all three ways of asking; publish, query, a second shell
  refused, the shell's connection dropped, stale, the next shell let in;
  a command through the shell's `Link` and back (done with its events,
  refused by both ways of sending, unreadable, not answered in 150 ms),
  an event relayed to a subscriber, a key's command, the `Link` dropped
  and the state stale; and the command line itself: `zones list`, `list
  --json`, `show`, `show --json`, `send` (done and refused, with the
  core's own reason and exit code), `restore-explorer`, and each of them
  with no shell.
- The command line's reading of `zones ...` beside `zone ...`, and its
  text (`src/zones_cli.rs`, 5 tests).

## What was not verified, and what is not done

- **No shell runs zones.** Nothing was tried against a running gevrek:
  the shell's component is being written, and the gevrek on this machine
  is an older build that was left alone. The path was only run between a
  test server and the real line.
- The path was run over TCP (as the integration tests are), not over the
  named pipe or the Unix socket. They carry the same HTTP/2; that a
  lasting call in both directions ends promptly when the shell's process
  is killed was seen on TCP only.
- On Linux the tests ran in WSL, over TCP as well; no compositor was
  started there.
- A key bound to a zones command was not pressed. The parts were tested
  (the command read from a binding's JSON, its event fields, the line
  reading that event); the window managers' own arm that emits the event
  is one line in each and was compiled, not run.
- `restore-explorer` reaches the shell as `Request::RestoreSystemIcons`;
  what the backend then does is milestone 2's other half.
- The studio and the guide do not mention any of this.
- Whether a session that is locked should let `peek` through was not
  asked: every zones command is refused while locked.
