# gevrek Architecture

## Overview

gevrek is a window manager for Windows and Linux that is growing into a
desktop environment for Windows, Linux and, eventually, macOS. Two decisions
carry that aim:

1. **A portable engine behind a contract.** All window-management logic
   (workspaces, layouts, rules, focus, bindings, gestures, animations, the
   automation graph) lives in `gevrek-engine` and `gevrek-model`, which use no
   system API, no UI toolkit and no async runtime (CI enforces this). The
   engine drives the system through the `Desktop` trait
   (`crates/engine/src/contract.rs`). A backend implements that trait for one
   system.
2. **Separate processes over gRPC.** The window-management runtime is a
   server; the editor, the CLI, the bar and external tools are clients that
   talk to it over **gRPC** with **Protobuf** serialization. The same clients
   work against every backend.

## Architecture Diagram

```
 gevrek-studio     gevrek (CLI)     gevrek-shell       external tools
       │                │                 │                  │
       └────────────────┴────────┬────────┴──────────────────┘
                                 │  gRPC/Protobuf over a per-user local endpoint
                    ┌────────────▼────────────┐
                    │  gevrek-server          │  IPC surface, the same on
                    │  gevrek-host            │  every system: config store,
                    │                         │  file watcher, started programs
                    ├─────────────────────────┤
                    │  gevrek-engine          │  window-management logic,
                    │  gevrek-model           │  configuration, node graph
                    ├──────── Desktop ────────┤  the contract a backend implements
          ┌─────────┴──────┬──────────────────┴──┬─────────────────┐
          │ gevrek-win32   │ gevrek-smithay      │ gevrek-macos    │
          │ gevrek-wm      │ gevrek-wl           │ gevrek-wm       │
          │ manages the    │ is the Wayland      │ manages the     │
          │ windows of the │ compositor; X11     │ windows through │
          │ Windows desktop│ through Xwayland    │ Accessibility   │
          └────────────────┴─────────────────────┴─────────────────┘
```

The two existing backends differ in kind. On Windows gevrek is a guest: the
system's compositor draws the windows and gevrek moves them. On Linux gevrek
is the compositor: it owns the outputs, the input devices and every frame.
A macOS backend would be a guest again. The contract is written so the engine
does not need to know which it is; what only one system can do is reported
through capabilities (`gevrek capabilities`) and configured under
`desktop.platform`.

The parts of a desktop environment that are still to come (launcher,
notifications, panel and tray, session and lock screen, settings) are meant to
follow the same pattern: clients of the same gRPC surface, written once, with
a thin system-specific part where one is needed.

## Communication Protocol

### Services (defined in `crates/protocol/proto/gevrek.proto`)

#### WindowManager Service

1. **PushGraph** - Send updated node graph to WM for execution
   - Request: `NodeGraph`
   - Response: `PushGraphResponse` (success, error, node count, connection count)

2. **PauseExecution** - Temporarily halt node graph execution
   - Request: `Empty`
   - Response: `StatusResponse`

3. **ResumeExecution** - Resume paused execution
   - Request: `Empty`
   - Response: `StatusResponse`

4. **TriggerManualEvent** - Fire a ManualTrigger node
   - Request: `TriggerRequest` (node_id)
   - Response: `StatusResponse`

5. **Dispatch** - Execute an action from the shared typed dispatcher
   - Request: `DispatchRequest`
   - Response: `StatusResponse`

6. **GetConfiguration / UpdateConfiguration** - Read or atomically validate,
   persist, and hot-reload the complete versioned configuration
   - Request/response: `ConfigurationDocument`

7. **QueryState** - Get current WM state (workspaces, windows, monitors)
   - Request: `Empty`
   - Response: `WMState`

8. **SubscribeToEvents** - Stream execution events in real-time (streaming RPC)
   - Request: `Empty`
   - Response: Stream of `ExecutionEvent`

9. **HealthCheck** - Check if WM is running
   - Request: `Empty`
   - Response: `HealthStatus` (healthy, version, uptime, message)

10. **DispatchBatch** - Execute a prevalidated, uninterrupted batch of typed
    dispatchers with per-command results (continue or stop-on-error)

11. **GetCapabilities** - Versioned discovery: protocol and config-schema
    versions, feature flags, and every dispatcher with argument metadata

12. **GetNodeDefinitions / GetCurrentGraph / GetWorkingGraph /
    UpdateWorkingGraph** - Automation node registry and working-graph
    collaboration between editor and WM

13. **GetOverlayData** - Consolidated snapshot for the overlay status bar

14. **Shutdown** - Graceful server shutdown

15. **AttachZones / GetZonesState / SendZonesCommand** - The zones on the
    desktop live in the shell. Over `AttachZones` (a call that lasts while
    the zones run) the shell tells how the zones are, what happens to them
    and what it answers, and is sent what is asked of them; the WM serves
    the last snapshot (`GetZonesState`, marked stale once the shell is
    gone) and passes commands on and waits for the answer
    (`SendZonesCommand`). The zones' own data travels as versioned JSON.
    See `docs/zones/milestone-2-ipc.md`.

### Execution Events (Live Feedback)

The editor receives real-time updates about node execution:

- `NodeExecutionStarted` - When a node begins executing
- `NodeExecutionCompleted` - When a node finishes (includes outputs and duration)
- `NodeExecutionFailed` - When a node encounters an error
- `GraphExecutionStarted` - When an event chain begins
- `GraphExecutionCompleted` - When an event chain finishes
- `LogEvent` - Log messages from the WM
- `RuntimeEvent` - What happens in the WM, by kind (`window.created`,
  `shell.command`, ...); what happens to the zones is among them as
  `zones.<name>`, passed on from the shell

### Data Format

All messages use **Protocol Buffers (protobuf)** for:
- Compact binary serialization
- Strong typing and schema validation
- Schema-v20 declarative monitor, appearance, animation-channel, workspace
  lifecycle, Dwindle, snap-zone, scratchpad, startup-supervision,
  advanced-binding, Precision Touchpad gesture, effect, theme, work-area
  offset, and typed geometry/decoration rule profiles with validated defaults
- Efficient network transfer
- Cross-language compatibility

## Project Structure

```
gevrek/
├── Cargo.toml                 # Workspace root and the `gevrek` package
├── src/bin/
│   ├── gevrek.rs              # Unified CLI and component manager
│   ├── wm.rs                  # gevrek-wm: the WM runtime and its server
│   │                          # (Windows and macOS)
│   └── elevated.rs            # gevrek-elevated: the helper for the windows
│                              # of programs that run as an administrator
├── crates/
│   ├── model/                 # Configuration, node graph, commands, keys,
│   │                          # rules, themes; `platform/` holds the
│   │                          # per-system settings. No system API.
│   ├── schema/                # The configuration's JSON Schema, derived
│   │                          # from the model
│   ├── engine/                # Window-management logic and the `Desktop`
│   │                          # contract; layouts, executor, animations,
│   │                          # a fake desktop for tests. No system API.
│   ├── zones/                 # The zones of the desktop (boxes that hold
│   │                          # its icons): what they are and do. No
│   │                          # system API.
│   ├── host/                  # What every host process shares: config store
│   │                          # and watcher, started programs, heartbeat;
│   │                          # and the WM host itself (`wm`): commands,
│   │                          # reload, bindings, published state, written
│   │                          # once for every backend
│   ├── protocol/              # gevrek.proto, generated code, client wrapper
│   ├── server/                # The gRPC server, independent of the system
│   ├── link/                  # The line from a program (the studio, the
│   │                          # shell) to the window manager, on a thread
│   │                          # of its own
│   ├── backend-win32/         # Windows backend: Win32/DWM, hooks, drawn
│   │                          # effects
│   ├── backend-smithay/       # Linux backend and gevrek-wl: a Smithay
│   │                          # Wayland compositor (nested window or
│   │                          # DRM/libinput on the hardware), Xwayland
│   ├── backend-macos/         # macOS backend (gevrek-wm there): windows
│   │                          # through the Accessibility API, keys from
│   │                          # an event tap, hidden windows parked
│   ├── ui/                    # The look every program of gevrek's shares
│   │                          # (egui): palette, fonts, widgets, icons
│   ├── shellkit/              # A surface on the screen (a panel, a
│   │                          # launcher) that egui draws into, on each
│   │                          # system's own terms
│   ├── system/                # What the shell asks of the system: sound,
│   │                          # networks, battery, clipboard, icons, the
│   │                          # desktop's files
│   ├── shell/                 # gevrek-shell: panel, launcher, control
│   │                          # centre, notifications, zones, the
│   │                          # tutorial's card
│   ├── studio/                # gevrek-studio (egui): every setting, the graph,
│   │                          # inspectors, the welcome; builds on every
│   │                          # system
│   ├── tutorial/              # The tutorial without a window: the lessons
│   │                          # as data, what a step waits for, a lesson
│   │                          # while it is taken, what is remembered
│   ├── lock/                  # gevrek-lock: the lock screen and the
│   │                          # greeter (Linux)
│   ├── dm/                    # gevrek-dm: the display manager (Linux)
│   ├── update/                # Updates on Windows: look, download, check,
│   │                          # put in place, take back
│   ├── logging/               # How every program of gevrek's logs
│   ├── build/                 # Which commit a program was built from
│   └── testclient/            # A Wayland client for the compositor's tests
├── tests/                     # The gRPC contract against a fake runtime;
│                              # every lesson of the tutorial played against
│                              # the real core (`tutorial_played.rs`)
├── xtask/                     # `cargo xtask`: check, install, package, dist
├── packaging/                 # Windows setup, .deb, .rpm, Arch, macOS app
├── resources/                 # The Linux session's units, the display
│                              # manager's files, the boot splash
├── scripts/                   # run-nested.sh, the other system's
│                              # type-check, WSL
├── site/                      # The web page (GitHub Pages)
└── docs/                      # GUIDE.md, LINUX.md, capability matrix,
                               # zones, roadmap
```

A new system means a new `crates/backend-*` crate that implements `Desktop`
and `gevrek_host::wm::WmHost`, plus its section under
`crates/model/src/platform/`. `Desktop` is what the engine asks of the
windows; `WmHost` lends the shared host the backend's configuration, core,
desktop and host state, and answers the few things only that system knows
(what follows a command, how keys are taken, what a reload changes of the
system's own settings). Everything else the host does (the server's
configuration requests and commands, the reload as a diff, the binding
ids, the published state) is the same code on every system, and is tested
on the engine's fake desktop (`crates/host/src/wm/tests.rs`).

## Building and Running

See [README.md](README.md) for Windows and [docs/LINUX.md](docs/LINUX.md) for
Linux. In short:

```bash
# Windows
cargo build --bins
cargo run --bin gevrek-wm          # WM on the local endpoint
cargo run --bin gevrek-studio      # settings, connects to the WM

# Linux
scripts/run-nested.sh              # build, then gevrek-wl as a window
cargo run --bin gevrek-studio      # from a terminal inside it
```

## Data Flow

### 1. Initial Setup
```
Editor → Creates/loads node graph → Displays in UI
```

### 2. Push Graph to WM
```
Editor → Converts NodeGraph (core) → NodeGraph (proto)
       → gRPC PushGraph → WM receives and stores
```

### 3. Apply Desktop Configuration
```
Editor → Edits typed profiles/rules/bindings → validates locally
       → gRPC UpdateConfiguration → validates and persists on WM
       → full Config hot-reloads focus, rules, bindings, workspaces, and graph
```

Workspace profiles carry Dwindle split policy and preserve-split behavior into
the runtime. The workspace owns concrete split-depth history and per-window
pseudotile sizes; the shared dispatcher mutates that state from CLI, bindings,
or the editor, and `QueryState` reports the effective active-workspace state
back to those frontends.

Ordered monitor profiles resolve primary/index/name/all selectors after Windows
monitor discovery. Their per-edge reservations are applied after the native
taskbar work area and before layouts, snap zones, rules, or scratchpads consume the
usable rectangle. Master-family workspace profiles persist the master edge and
new-window insertion policy. Full hot reload preserves tiled container and
group ordering so applying a placement policy does not reorder existing windows.
State and overlay IPC report the effective usable area and active policy.

Scratchpad profiles are first-class special workspaces rather than graph-local
state. The executor owns normalized profile IDs, safely launches configured
program/argument arrays without a shell, adopts matching windows on creation,
positions them against active/primary/indexed/named monitors, optionally hides
them on focus loss, and preserves tracked windows across full hot reloads.
Bindings, graph nodes, gRPC dispatch, the CLI, editor Command Center, state
queries, and the overlay all use or report the same runtime state.

Snap zone settings configure the existing drag-snap engine instead of constructing
a hard-coded manager. A default normalized layout and monitor-specific
primary/index/name/all overrides are resolved after monitor discovery. Direct
snap-zone dispatch converts tiled windows to floating placement, records the zone
assignment, and supports stable next/previous cycling through runtime
bindings, gRPC, CLI, and the editor Command Center.

Advanced bindings use one low-level Windows keyboard/mouse hook engine instead
of `RegisterHotKey`. The engine resolves press, release, native repeat, and
timed long-press events, identifies mouse buttons and wheel directions, and
decides consume versus pass-through before Windows forwards the input. The
runtime registers only the active named submap, while automation-graph key
events remain global. Submap transitions use the typed dispatcher and the
effective submap is reported through state and overlay IPC to the editor, CLI,
and status bar.

Precision Touchpad gestures use a hidden message-only window registered for
global digitizer/touchpad Raw Input. The backend discovers compatible HID
collections, caches their preparsed descriptors, reads contact IDs/counts and
logical X/Y ranges, then sends normalized contact-centroid frames to a
device-neutral recognizer. Exact two- through five-contact profiles lock only
after a dominant axis emerges, commit once at their configured travel
threshold, and cancel on release, finger-count changes, or idle timeout. The
runtime dispatches completed gestures through the same typed command path as
bindings, CLI, gRPC, and the editor. Started, updated, completed, and cancelled
events feed the live editor, overlay, CLI state, and external event subscribers.

Window discovery callbacks only transport operating-system facts. The runtime
owns rule evaluation and maintains separate base tags (manual and static) and
derived dynamic tags. Title, minimize/restore, drag/workspace, tag, and named
rule changes re-evaluate dynamic rules. The executor records each window's
pre-dynamic workspace, rectangle, floating, topmost, opacity, maximize,
fullscreen, border/caption color, backdrop, and corner state so effects are
reversed when predicates stop matching. `QueryState` publishes effective
tags, matched rules, resolved effects, and live named-rule state to the CLI
and editor.

The editor starter library is source-defined and produces complete `Config`
values through the same public settings and graph APIs used by ordinary
files. Every starter is validated in tests before it can ship, and loading one
copies it into the editor's mutable working configuration.

### 4. Execution
```
WM → Detects window event → Executes matching nodes
   → Sends ExecutionEvent stream → Editor displays in real-time
```

### 5. Control Flow
```
Editor → Pause/Resume buttons → gRPC commands → WM pauses/resumes execution
```

### 6. State Query
```
Editor → Request state → WM sends workspace/window info → Editor displays
```

## Key Features

### Bidirectional Communication
- Editor can push graphs and send control commands
- WM streams execution events back to editor
- Real-time feedback for debugging

### Health Checks
- Health reports reflect the actual runtime initialization state
- The editor reconnects after its event stream is interrupted
- The overlay retries after startup or polling failures
- Component startup waits for a healthy WM before launching dependants

### Error Handling
- Status responses for all commands
- Detailed error messages in ExecutionEvent
- Reconnect and retry handling in the editor and overlay clients

### Async Execution
- WM uses tokio async runtime
- Non-blocking event streaming
- Concurrent request handling

## Implementation Status

- Editor connection, declarative configuration editing, graph synchronization,
  Push, pause/resume, manual triggers, runtime state, logs, and node execution
  events are implemented.
- Full configuration updates and graph pushes hot-reload the runtime and
  persist the active configuration.
- A polling config-file watcher (`runtime/config_watch.rs`) hot-reloads
  external edits to the active configuration, debouncing non-atomic saves.
- A single event-loop animation scheduler owns layout, workspace, border, and
  explicit automation transitions. Re-targeting cancels the previous
  transition without spawning worker threads, and runtime status is reported
  through `QueryState`.
- CLI component lifecycle and remote window/workspace actions are implemented.
- Overlay state includes real measured event-loop rate and reconnects after a
  dropped WM connection.
- The default endpoint is local to the user (see "Default Address"). A TCP
  address is unauthenticated; authentication and TLS remain deployment
  concerns if the service is deliberately exposed that way.

## Protocol Evolution

The protocol and configuration carry explicit version information:

- Version field in NodeGraph messages
- Schema version in the declarative desktop configuration
- Server version in HealthStatus

gevrek is pre-1.0 and does not promise backward compatibility yet. Protocol,
configuration, and node definitions may be simplified when a cleaner model is
available; version fields make those changes diagnosable instead of silently
misinterpreting data.

## Default Address

`local`: a Unix socket in the runtime folder on Linux
(`$XDG_RUNTIME_DIR/gevrek/<instance>.sock`, mode 0600), a named pipe on
Windows (`\\.\pipe\gevrek-<user>-<instance>`). Only the user's own programs can
open it, which matters because the WM can start programs and replace the
configuration. `<instance>` is `main`, or `GEVREK_INSTANCE`, so several
gevreks run side by side (`crates/protocol/src/endpoint.rs`).

All executables accept `--address`: `local`, `local:<instance>`, or `ip:port`
for TCP (unauthenticated; for tests across machines). The unified CLI
forwards its address to components it starts.
