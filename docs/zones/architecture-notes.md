# Zones: notes on the codebase (Phase 0)

What gevrek is today, as far as it bears on a desktop icon-organization
feature. Written 2026-11-19 at 0.6.0 (commit `6122353`). Paths are relative
to the repository root. Nothing here was checked on a running desktop; it is
read off the source.

## Language, build, split

- Rust, edition 2021, one cargo workspace (`members = ["crates/*", "xtask"]`),
  version 0.6.0. No `rust-version`, no toolchain file.
- **Portable core.** `crates/model` (serde data: configuration, commands,
  keys, themes) and `crates/engine` (window-management logic). Neither may
  use a system API, egui, eframe or tokio: CI greps for it
  (`.github/workflows/ci.yml:20`, `scripts/check-linux.ps1:24-30`).
- **The contract.** The engine drives a system through the `Desktop` and
  `WindowQueries` traits (`crates/engine/src/contract.rs`). A fake desktop
  for tests implements them (`crates/engine/src/testing/fake_desktop.rs`,
  behind the `test-support` feature).
- **Backends.** `crates/backend-win32` (Win32/DWM; gevrek is a guest beside
  Windows' own shell) and `crates/backend-smithay` (gevrek is the Wayland
  compositor, `gevrek-wl`; X11 programs through Xwayland).
- **Around them.** `crates/host` (config store, watcher, started programs),
  `crates/server` + `crates/protocol` (gRPC), `crates/link` (the line from
  shell or studio to the WM), `crates/system` (what the system knows:
  installed programs, icons, sound, session), `crates/schema` (JSON Schema
  of the configuration).
- **UI programs.** `crates/shell` (panel, launcher, notifications, control
  centre, switcher, tray, and more), `crates/studio` (settings), `crates/ui`
  (the design system), `crates/shellkit` (the surfaces egui draws into).

## UI and rendering

- egui 0.37 drawn with egui-wgpu 0.37. The shell uses no winit and no
  eframe; the studio uses eframe.
- `crates/shellkit` gives both systems one surface model: an `Edge` (the
  four sides, the corners, `Fill`, `Centre`) and a `Level` (`Background`,
  `Below`, `Above`, `Overlay`, `Marks`) (`crates/shellkit/src/lib.rs:1-60`).
  - **Windows** (`shellkit/src/win/mod.rs`): each surface is a borderless
    tool window of class `Gevrek_Shell_<name>`. `Background` and `Below`
    are the same: placed at `HWND_BOTTOM`. `Above`/`Overlay` are topmost.
  - **Linux** (`shellkit/src/layer.rs`): a wlr-layer-shell client through
    smithay-client-toolkit 0.19.
- No drag and drop exists in the shell beyond drags inside one egui surface:
  no `IDropTarget`/`RegisterDragDrop`, no `wl_data_device` client.

## Linux: who owns the desktop

- gevrek is a **Wayland compositor** (smithay 0.7), not an X11 window
  manager and not a client of another compositor. Three ways to run:
  nested in a window (`winit.rs`), on the hardware (`udev.rs`, whose header
  says it has not run on a physical machine), and headless for tests.
- The compositor draws the wallpaper itself
  (`backend-smithay/src/wallpaper.rs`, drawn last in `render.rs:1106`).
- It implements wlr-layer-shell for clients. Order, top to bottom: Overlay,
  Top, the windows, Bottom, Background, the wallpaper (`render.rs`).
- Blur behind a layer surface exists as a layer rule (`soft.rs`); rounded
  content is a GLES shader (`clip.rs`). Fractional scale is implemented.
- Drag and drop: `wl_data_device` for clients; `xdnd.rs` bridges Wayland
  drags to X11. Whether a drag from an X11 program reaches a Wayland
  surface was not checked.
- **Nothing draws desktop icons.** Nothing reads the Desktop folder;
  `XDG_DESKTOP_DIR` is used nowhere.

## Windows: who owns the desktop

- gevrek **runs beside `explorer.exe`** and does not replace it: no
  `SetShellWindow`, no Winlogon `Shell` entry. It hides the taskbar's
  windows while its own panel is shown (`backend-win32/src/taskbar.rs`) and
  runs a decoy `Shell_TrayWnd` for tray icons (`system/src/tray_host.rs`).
- **Explorer owns the desktop window and its icons.** gevrek has no window
  parented to or placed against `Progman`/`WorkerW`; the names
  `SHELLDLL_DefView` and `SysListView32` appear nowhere. gevrek only
  ignores those windows in its rules and detects a press on the bare
  desktop (`windows_key.rs:231-244`, the `DesktopPressed` event).
- Windows draws the wallpaper; gevrek hands it the picture through
  `IDesktopWallpaper` and restores the original
  (`backend-win32/src/wallpaper.rs`).
- **DWM.** Used on other programs' windows only: frame bounds, cloak,
  border and caption colour, `DWMWA_SYSTEMBACKDROP_TYPE`, corner
  preference, thumbnails. shellkit's own surfaces use no DWM attribute.
- **DPI.** Per-monitor v2 in `gevrek-wm` (manifest and call) and in
  shellkit (`win/mod.rs:303`).
- **No injection.** Hooks are low-level keyboard/mouse hooks and
  out-of-context WinEvent hooks. `gevrek-elevated` only moves, shows and
  hides windows of elevated programs.
- No integration with Windows' virtual desktops (`IVirtualDesktop` is used
  nowhere). gevrek's workspaces are its own.

## Configuration

- One JSON file, `config.json` (`%APPDATA%\gevrek`, `$XDG_CONFIG_HOME/gevrek`).
  State is in `%LOCALAPPDATA%\gevrek\state` / `$XDG_STATE_HOME/gevrek`.
- Schema version 26 (`model/src/settings.rs:12`), migrated step by step on
  JSON values (`model/src/migrate.rs`). (It was 25 when these notes were
  first written; 26 is the renaming below.)
- Saved by writing the whole file anew (`serde_json::to_string_pretty`):
  atomic (temp file, fsync, rename), after a backup into
  `state/backups/` (100 kept) (`model/src/files.rs`). **JSON has no
  comments**; unknown keys are kept (`extra` on every section).
- Hot reload by polling the file's time once a second
  (`host/src/config_watch.rs`).
- **`desktop.zones` was taken** by the snap-zone feature (below). Since
  schema 26 that is `desktop.snap_zones`, and `desktop.zones` is the
  section of this feature (`model/src/zones.rs`, `enabled` false by
  default). A file of schema 26 with snap zones under `desktop.zones` is
  refused, not guessed at.
- There is no feature-flag mechanism: only `enabled` fields per section,
  environment variables, and the server's capability strings.

## The name "zones" is in use

(As it was at commit `6122353`. Since then the feature described here is
"snap zones" throughout: `desktop.snap_zones`, `SnapZone*`,
`model/src/snap_zones.rs`, `engine/src/snap_zones.rs`,
`studio/src/snap_zones_edit.rs`.)

`desktop.zones`, `Zone`, `ZoneLayout`, `ZoneConfig`, `ZoneSettings`,
`ZoneManager`, the commands `SnapActiveToZone` / `CycleActiveZoneNext` /
`CycleActiveZonePrevious`, the CLI's `zone`, `zone-next`, `zone-previous`,
and the studio page "Snap zones" are a FancyZones-style feature: rectangles
on a display that a dragged window snaps into (`model/src/zones.rs`,
`engine/src/zones.rs`, `studio/src/zones_edit.rs`). See design.md, open
question 1.

## IPC and CLI

- gRPC over a per-user endpoint: a named pipe on Windows, a Unix socket on
  Linux (`protocol/src/endpoint.rs`). The server is in the WM's process.
- Commands are `DispatchCommand` variants (`model/src/command.rs`). An RPC
  takes a command as configuration JSON, so a new command needs no change
  of the protocol; typed entries in the proto and the CLI are added by hand.
- What the shell does is asked through `DispatchCommand::Shell { action }`,
  which reaches the shell as a `ShellCommand` event. There is no general
  path for the shell to answer a query.
- Events: `SubscribeToEvents` streams `ExecutionEvent`s, among them
  `runtime_event` with a `RuntimeEventKind` (`engine/src/executor/mod.rs`).

## Input, theming, accessibility

- Bindings are `BindingProfile`s with a `DispatchCommand`, matched in the
  engine (`engine/src/input.rs`); submaps, hot corners and gestures exist.
  Defaults follow niri with Super: Super+arrows or H/J/K/L to focus, with
  Ctrl to move, Super+O overview, Super+D launcher.
- `crates/ui/src/theme.rs`: a palette of roles, a radius, density, fonts,
  `reduce_motion`. `desktop.appearance.high_contrast` and
  `desktop.animations.reduce_motion` follow the system's settings.
- Accessibility is AccessKit under egui: UI Automation on Windows, AT-SPI
  on Linux. No code talks to either directly.

## Displays and workspaces

- A display is known by its `device_path` (`engine/src/outputs.rs`): on
  Windows the `\\?\DISPLAY#…` path, stable for a display on a port; on
  Linux the connector's name (`HDMI-A-1`). EDID was read nowhere when
  these notes were first written; it is now read on both systems into
  `MonitorInfo::stable_id` (`model/src/edid.rs`), which is the
  same for a display wherever it is plugged in.
- Each display has its own ring of workspaces. A display that goes has its
  workspaces kept and given back when it returns
  (`engine/src/outputs_reconcile.rs`).
- Room kept free: `work_area_offset` and per-monitor edges; on Windows the
  WM finds the panel by its class and keeps its room; on Linux layer-shell
  exclusive zones.

## What exists to build on, and what does not

| Need | Today |
|---|---|
| Icons of files and programs | `system/src/icons.rs`: icon-theme lookup on Linux, `IShellItemImageFactory` on Windows, a cache off the draw thread |
| `.desktop` entries | parsed by hand in `system/src/apps.rs` (for programs, not for trust rules) |
| Starting things | `ShellExecuteExW` (`system/src/start_menu.rs`), `LaunchProfile`, `Exec` |
| Atomic files, backups | `model/src/files.rs` |
| A file picker | `ui/src/browser.rs` |
| Watching folders | nothing (no `ReadDirectoryChangesW`, inotify, or `notify`) |
| Shell namespace, context menus, OLE drag and drop | nothing |
| Thumbnails of files, trash | nothing |
| MIME types, default programs | nothing found |

Dependencies come from crates.io only, with a licence allow-list and
RustSec checks (`deny.toml`). No written rule beyond that.

## Tests

- Unit tests beside the code; the fake desktop; seeded long runs that hold
  invariants (`engine/src/random_runs.rs`), which is the house form of a
  property test (no proptest).
- `backend-smithay/tests/` drives the compositor headless with real
  Wayland clients. `model/tests/` holds fixtures of old configurations.
- CI: portable tests on Linux, the workspace on Linux without the Windows
  backend, the workspace and clippy on Windows, cargo-deny.
- The checks that drive a real desktop are not part of the public
  repository (`docs/ROADMAP.md:55-57`).
