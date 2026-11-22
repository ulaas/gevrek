# Capability matrix

This document defines the feature surface gevrek aims for. It is a living
completion matrix, not a loose wish list. Each row names a user-facing
workflow; a row is complete when that workflow works where the platform
exposes the necessary capability. Concerns the platform owns are marked as
such instead of being counted as missing.

gevrek targets Windows and Linux, and macOS is new, and aims to become a
complete desktop environment (see the [README](../README.md)). The statuses
below were established on Windows, the first backend; what the Linux backend
does not do yet is listed in [LINUX.md](LINUX.md). Desktop-environment parts
beyond window management are not in this matrix yet.

Status meanings:

- **Implemented**: usable from the WM and at least one supported frontend.
- **Partial**: foundations exist, but a documented user workflow is incomplete.
- **Missing**: no complete implementation exists.
- **Windows-owned**: Windows or the display driver owns this concern; gevrek
  should expose relevant state but must not pretend to be the compositor.

## Verified milestone

The current dispatcher/configuration/input milestone was verified on Windows
on 2026-08-01:

- `cargo check --workspace --all-targets`
- 307 passing tests from `cargo test --workspace --all-targets` (258 in
  `gevrek-core`, 21 gRPC integration tests against a fake runtime channel,
  28 in CLI/WM/lib bins)
- `cargo clippy --workspace --all-targets` (warnings remain; warnings are not
  denied repository-wide yet)
- optimized CLI, editor, overlay, and WM entry-point builds
- release CLI parsing for workspace selectors, layouts, groups, snap zones, and
  binding submaps
- live health/state queries against a running two-monitor WM
- live creation and departure from a named workspace, with the resulting
  inactive workspace still present in the all-workspace IPC snapshot
- visual inspection of the connected release editor and its Configure and
  Command Center surfaces

The normal optimized binaries all linked successfully. Automated tests
intentionally do not install global hooks or take over the live desktop, so
live-desktop scenario verification remains manual. Low-level input delivery,
multi-monitor focus, window lifecycle, fullscreen restoration, group
visibility, hot reload, and animation timing still require repeatable
Windows scenario tests before gevrek can be called production-ready.

## Capability matrix

| Area | Requirement | gevrek status | Completion evidence |
|---|---|---:|---|
| Dispatch | One typed action vocabulary shared by IPC, CLI, binds, and UI | Implemented | `DispatchCommand`, gRPC `Dispatch`, CLI commands, binding profiles, editor Command Center |
| Focus | Cycle, history, spatial navigation, and targeted selection | Implemented | next/previous and left/right/up/down geometric selection, focus-last history, plus active/floating/tiled/handle/address/PID/live-title/initial-title/live-class/initial-class/executable/path/tag selectors shared by runtime, IPC, CLI, bindings, gestures, Command Center, and the live window inspector; text selectors offer approachable case-insensitive substring matching plus validated `re:` regular-expression mode |
| Window movement | Spatial tile swap; absolute/relative floating move and resize | Implemented | directional swaps plus active-window deltas and selector-targeted absolute/relative geometry shared by runtime, IPC, CLI, bindings, gestures, and Command Center |
| Window state | floating, maximize, minimize, fullscreen, pin/topmost, close | Implemented | typed active and selector-targeted close, fullscreen, toggle/idempotent float/tile, and toggle/idempotent pin/unpin dispatchers; fullscreen restores style/bounds/maximized state |
| Window placement | center and explicit top/bottom Z-order | Implemented | active and selector-targeted center/raise/lower actions; center reveals the target workspace, promotes to floating, and uses the reserved monitor work area; the live inspector exposes direct focus/float/tile/fullscreen/pseudotile/center/pin/tag-clear/close controls |
| Workspaces | switch, silent move, move-and-follow, rename; move/swap between monitors | Implemented | active and selector-targeted silent/follow moves plus typed runtime, IPC, CLI, bindings, gestures, editor Command Center, lifecycle events, rule re-evaluation, and multi-monitor starter coverage |
| Workspace selectors | relative, named, previous, empty, monitor-local | Implemented | typed selectors shared by runtime, gRPC, CLI, bindings, and editor |
| Special workspaces | multiple named scratchpads with show/hide/toggle | Implemented | schema-v8 named profiles, structured launches, matching/adoption, monitor-aware placement, focus-loss hiding, hot-reload preservation, graph/bind/IPC/CLI/editor controls, and live state |
| Workspace rules | monitor binding, defaults, persistence, created-empty actions, per-workspace gaps/layout | Implemented | schema-v13 eagerly materializes persistent/default profiles, lazily creates other named profiles, prunes inactive empty workspaces, supports structured `on_created_empty` launches without firing for move-created workspaces, preserves occupied/active names across hot reload, publishes lifecycle events, and exposes validation/editor/starter coverage |
| Monitors | discovery, per-monitor work areas and workspace rings; logical focus | Implemented | typed `focus-monitor`, `move-workspace-to-monitor`, and `swap-active-workspaces` actions share primary/index/name/next/previous selectors across runtime, IPC, CLI, bindings, gestures, Command Center, and starter coverage |
| Monitor rules | stable name matching, reserved areas, workspace binding | Implemented | schema-v8 ordered primary/index/stable-name/all monitor profiles, per-edge reservations after the Windows work area, workspace binding, hot reload, editor forms, starter coverage, and effective IPC/CLI/overlay state |
| Display mode | resolution, refresh, scale, transform, VRR, mirror | Windows-owned | expose read-only state where Windows APIs permit |
| Layouts | dwindle/BSP and master-family layouts | Implemented | BSP, Dwindle, Master Stack, Centered Master |
| Extra layouts | columns, rows, grid, spiral, monocle | Implemented | layout engine implementations |
| Floating windows' look | a border and glow of their own | Implemented | `appearance.floating`: width, offset, colours and the glow's reach and strength for floating windows, apart from the tiled ones (by default one pixel thinner and two thirds of the glow); a window is dressed anew when it starts or stops floating; the glow's part is drawn on Wayland only |
| Floating layout | a workspace whose windows all float | Implemented | `Floating` among the layouts: the tiled windows float where they are, a window that opens or is moved there is put in the middle of the display at its own size, another layout tiles them again; by command, the panel's menu and the configuration |
| Layout controls | select, ratio, master count, gaps, rotate, cycle, promote | Implemented | functional rotation plus typed runtime/gRPC/CLI/binding/editor controls |
| Dwindle controls | preserve split, forced split, pseudotile, split dispatcher | Implemented | split-depth history, alternating/horizontal/vertical policies, active and selector-targeted preferred-size pseudotile, typed runtime/gRPC/CLI/binding/editor controls, and live state reporting |
| Master controls | orientation, promote, multiple masters, placement policy | Implemented | ratio/count/rotation/promote plus schema-v8 persistent left/right/top/bottom master position and stack-end/master/after-focused insertion policy, with hot-reload order preservation and editor/starter/live-state support |
| Groups | tabbed groups, lock, cycle, indexed focus, reorder, merge, remove | Implemented | container-backed groups expose active- and selector-targeted toggle/lock/cycle/index/reorder operations and preserve inactive-workspace visibility; a tab strip is opt-in via the GDI stackbar (`StackbarProfile`, `toggle-stackbar` dispatcher) with tested tab geometry and hit-testing; available through runtime, gRPC, CLI, bindings, gestures, Command Center, and starters; merge/remove remain useful gevrek extensions |
| Rules | ordered multi-predicate matching | Implemented | all/any predicates, negation, case handling, regex validation, ordered overrides |
| Rule predicates | title, class, executable, path, state, workspace, tags | Implemented | title/class/exe/path, Windows state, workspace name/index, and cascading normalized tag predicates |
| Rule effects | float/tile/workspace/size/position/state/decorations/opacity/focus | Implemented | schema-v19 preserves floating aspect ratios along the user's dominant resize axis while reconciling dynamic pixel/percent min/max bounds; real resize events, focus policies, durable persistent sizes, move/resize/center, state, routing, tags, and native decorations remain editable and reversible |
| Dynamic rules | re-evaluate title/state/workspace/tag changes; named enable/disable | Implemented | runtime-owned metadata events, reversible workspace/geometry/state/decoration restoration, named session toggles, gRPC/CLI/bind/editor controls, and live resolved-effect state |
| Tags | static/dynamic window tags usable by rules and dispatch | Implemented | base/derived tag metadata, cascading add/remove effects, active and selector-targeted add/remove/toggle/clear dispatch, query state, CLI, bindings, Command Center, and inspector |
| Keyboard binds | modifiers and press actions | Implemented | typed declarative bindings and graph key events share the runtime dispatcher |
| Advanced binds | release/repeat/long-press, mouse, submaps, inhibit/pass | Implemented | schema-v8 low-level keyboard/mouse engine, press/release/native-repeat/timed-long-press triggers, button/wheel input, modal submaps, consume/pass-through policy, recorder/editor/IPC/CLI/overlay controls, and validated starter |
| Gestures | multi-finger directional actions with live progress | Implemented | schema-v12 profiles map exact 2–5 finger left/right/up/down swipes to any typed action; the global Windows Precision Touchpad Raw Input backend normalizes HID logical ranges and feeds a deterministic axis-lock, threshold, single-commit, and idle-cancel recognizer; profiles hot-reload, while live backend/progress state and structured events reach IPC, CLI, editor, overlay, and extensions; Advanced Binding Lab provides validated starters |
| Appearance | active/inactive borders, colors, alpha, rounded corners | Implemented | GDI border renderer draws the four supported styles (Square/Rounded/Double/Dashed) with per-state colors and alpha; unsupported styles fail config validation loudly instead of silently degrading; native DWM border/caption color, backdrop, and corner overrides complement the custom border |
| Decorations | backdrop, caption color, corner preference | Implemented | Windows 11 DWM actions |
| Effects | per-window opacity, blur/backdrop, shadow, dim, overrides | Implemented | schema-v9 composes global/rule opacity with focus-relative inactive dimming, fullscreen policy, reversible per-rule dim exemptions, Windows DWM shadow enable/disable/default overrides, backdrop/corner/border/caption effects, editor controls, starter coverage, and effective IPC/CLI/overlay/inspector state |
| Animations | non-blocking layout, workspace, window-effect, and border transitions | Implemented | schema-v11 gives layout, workspace, border, and window/graph channels independent enable switches, durations, and curves on one cancellable 60 Hz scheduler; graph automation transitions render the five graph styles (Fade/Slide/Scale/Expand/Contract); configurable None/Fade/Slide/Scale entrance and WM-requested exit styles preserve rule opacity, defer close safely, survive hot-reload disabling, publish IPC/CLI/overlay state, and have editor previews, starter coverage, and deterministic timing/completion tests |
| Overview | expose-style window overview | Implemented | runtime action and editor/CLI dispatcher |
| Snap zones | snap layouts and zone assignment | Implemented | schema-v8 behavior, nine presets, custom visual geometry editor, monitor overrides, activation modifiers, drag snapping, direct/cycling runtime actions, bindings, gRPC, CLI, Command Center, and starter |
| IPC | health, complete desktop state, events, config collaboration, typed actions | Implemented | protocol-v2 publishes server/config-schema versions, feature flags, all 138 typed dispatchers with argument metadata, automation node types, and every materialized workspace/window including inactive workspace contents and active-workspace identity; prevalidated uninterrupted batches support continue/stop-on-error with per-command results, alongside health, event streams, graph collaboration, and atomic full-config get/update |
| CLI | lifecycle, state, logs, graph/config apply, dispatchers, and persistent rules | Implemented | typed lifecycle/state/actions, versioned capability discovery, prevalidated uninterrupted batch files, full live-config export and offline validation, plus atomic persistent rule list/upsert/remove through the shared configuration validator |
| Autostart | run programs and event-driven commands | Implemented | schema-v10 provides structured program/argument/environment/working-directory launches, delay and enable controls, restart policies and limits, reload reconciliation, optional shutdown termination, binding/IPC/CLI/editor controls, live status in the editor/CLI/overlays, and an event-driven Launch Process graph node demonstrated by a built-in starter |
| Extensions | scripts/plugins and external event hooks | Implemented | the versioned `extension_events_v1` capability publishes structured runtime/configuration/binding/dispatch/window events over the broadcast gRPC stream with lag reporting; CLI NDJSON supports script and satellite consumers, which can respond through typed single or prevalidated batch dispatch |
| Session utilities | lock, idle, wallpaper, notifications | Windows-owned | integrate suitable Windows satellites instead of compositor protocols |

## Editor rewrite

The editor exposes two complementary levels:

1. **Configure** — task-oriented pages for monitors, workspaces, layouts,
   bindings, gestures, rules, appearance, animations, and startup.
2. **Automate** — the node graph for conditional/event-driven workflows.

The shell uses this information architecture:

- **Home**: connection, health, diagnostics, unsaved changes, quick actions.
- **Command Center**: searchable live actions and parameter forms.
- **Monitors & workspaces**: visual monitor map and workspace profiles.
- **Bindings**: key/mouse recorder, gesture builder with live progress,
  conflict detection, and the shared action picker.
- **Rules**: ordered rule builder with live window picker and match preview.
- **Appearance**: borders, corners, opacity, backdrop, animation preview.
- **Automation**: existing node canvas, upgraded to consume the same typed
  action and condition schemas as the form editors.
- **Inspector**: live windows, workspaces, execution trace, errors, and logs.

Every form must support validation before apply, undo/redo, search, reset to
default, and a clear distinction between draft, applied runtime state, and
persisted configuration.

The Home workspace is implemented as the default landing surface. It combines
connection and pause state, live desktop counts, schema and full-config
diagnostics, cached unsaved-change detection, save/apply controls, direct
navigation, capability details, and fourteen loadable starter configurations.
Automate remains a separate graph workspace so first-time users are not dropped
directly onto the canvas. Local-file and confirmed runtime revisions are tracked
independently; asynchronous apply progress, rejection, transport failure, and
success remain visible instead of being inferred from a button click.
The Windows and Workspace inspectors use the WM's authoritative all-workspace
snapshot instead of a local monitor scan, so inactive workspaces and their
windows remain inspectable with live focused-monitor identity, layout, mode,
rule, tag, decoration, and opacity state. Standalone editor connection state now owns inspector
availability; disconnects clear cached windows and snapshots instead of showing
stale desktop state.

## Delivery order

1. Typed dispatcher and Command Center.
2. Versioned declarative configuration alongside graph migration.
3. Named workspaces/selectors, workspace and monitor profiles.
4. Ordered multi-match window rules and effects.
5. Unified binding engine and binding editor.
6. Group integration and advanced layout controls.
7. Appearance/animation profiles and preview.
8. Gesture backend, extension hooks, richer IPC and diagnostics.
9. Requirement-by-requirement audit with Windows runtime scenarios.
