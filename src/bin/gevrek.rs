//! Gevrek - Unified CLI tool for managing Gevrek window manager components
//!
//! This tool provides a unified interface for:
//! - Starting/stopping/restarting components (wm, editor, shell)
//! - Sending RPC commands to the window manager
//! - Querying state and subscribing to events

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand, ValueEnum};
use gevrek_model::command::{MonitorTarget, WindowSelector, WorkspaceSelector};
use gevrek_model::layout::LayoutType;
use gevrek_protocol::{proto, WMClient, DEFAULT_WM_ENDPOINT};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

/// Default gRPC server address
const DEFAULT_ADDRESS: &str = DEFAULT_WM_ENDPOINT;

/// PID file directory for tracking running components
#[derive(Subcommand, Debug)]
enum InputStep {
    /// Press and release a chord: `gevrek input key Super+T`
    Key { chord: String },
    /// Press and release a key by its place (XKB keycode, evdev + 8):
    /// `gevrek input keycode 31 --hold super`
    Keycode {
        code: u32,
        #[arg(long, value_delimiter = ',')]
        hold: Vec<String>,
    },
    /// Hold a key or modifier down: `gevrek input down Super`
    Down { key: String },
    /// Let it go again
    Up { key: String },
    /// Move the pointer
    Move { x: i32, y: i32 },
    /// Click at a place
    Click {
        x: i32,
        y: i32,
        #[arg(long, default_value = "left")]
        button: String,
    },
    /// Drag from one place to another, with modifiers held:
    /// `gevrek input drag 400 300 800 500 --hold alt`
    Drag {
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        #[arg(long, default_value = "left")]
        button: String,
        /// Modifiers held meanwhile, with commas: `super,shift`
        #[arg(long, value_delimiter = ',')]
        hold: Vec<String>,
    },
    /// One step of the wheel: `gevrek input wheel up --hold super`
    Wheel {
        direction: String,
        #[arg(long, value_delimiter = ',')]
        hold: Vec<String>,
    },
    /// Plug in another output (a window manager without a display only):
    /// `gevrek input plug 1920x1080`
    Plug { size: String },
    /// Unplug an output by name: `gevrek input unplug HEADLESS-2`
    Unplug { name: String },
    /// Steps as JSON: `gevrek input steps '[{"input":"chord","chord":"Super+1"}]'`
    Steps { json: String },
}

fn pid_dir() -> PathBuf {
    gevrek_model::paths::runtime_dir()
}

/// Files of this instance: several gevreks share the runtime folder.
fn log_file(component: &str) -> PathBuf {
    pid_dir().join(format!(
        "{}.{}.log",
        gevrek_model::paths::instance(),
        component
    ))
}

fn pid_file(component: &str) -> PathBuf {
    pid_dir().join(format!(
        "{}.{}.pid",
        gevrek_model::paths::instance(),
        component
    ))
}

/// Gevrek - a tiling window manager for Windows and Linux
#[derive(Parser)]
#[command(name = "gevrek")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Server address for RPC commands
    #[arg(short, long, default_value = DEFAULT_ADDRESS, global = true)]
    address: String,

    #[command(subcommand)]
    command: Commands,
}

/// What `gevrek wallpaper` changes.
#[derive(Subcommand, Debug, PartialEq)]
enum WallpaperAction {
    /// Show a picture: on every display, or with --output on one
    Set {
        /// The picture's file (PNG, JPEG or WebP)
        file: PathBuf,
        /// Only on this display (its connector: DP-1); without it the
        /// picture is every display's and none keeps one of its own
        #[arg(long)]
        output: Option<String>,
        /// fill, fit, stretch, centre or tile (default: as it was)
        #[arg(long)]
        fit: Option<String>,
    },
    /// Take the picture away (the colour stays), or with --output take
    /// away what one display has of its own
    Clear {
        #[arg(long)]
        output: Option<String>,
    },
    /// Replace the whole wallpaper section with the JSON in a file (`-`:
    /// read from standard input): several displays changed at once
    Apply { file: PathBuf },
}

/// A fit as it is written in the configuration (`center` is read too).
fn wallpaper_fit(said: &str) -> Result<gevrek_model::wallpaper::WallpaperFit> {
    let said = said.trim().to_ascii_lowercase();
    let said = if said == "center" {
        "centre"
    } else {
        said.as_str()
    };
    serde_json::from_value(serde_json::Value::String(said.to_string()))
        .map_err(|_| anyhow!("{said}: the fit is fill, fit, stretch, centre or tile"))
}

/// Put a picture into the wallpaper section: every display's (and none
/// keeps one of its own), or one display's.
fn wallpaper_set(
    section: &mut gevrek_model::wallpaper::WallpaperConfig,
    image: String,
    output: Option<&str>,
    fit: Option<gevrek_model::wallpaper::WallpaperFit>,
) {
    let Some(output) = output else {
        section.image = image;
        section.fit = fit.unwrap_or(section.fit);
        section.outputs.clear();
        return;
    };
    match section
        .outputs
        .iter_mut()
        .find(|entry| entry.name.eq_ignore_ascii_case(output))
    {
        Some(entry) => {
            entry.image = image;
            entry.fit = fit.unwrap_or(entry.fit);
        }
        None => section
            .outputs
            .push(gevrek_model::wallpaper::OutputWallpaper {
                name: output.to_string(),
                image,
                // A display that had the common picture keeps how it was put.
                fit: fit.unwrap_or(section.fit),
                ..Default::default()
            }),
    }
}

/// Take the picture out of the wallpaper section, or what one display
/// has of its own (it shows the common one again). Whether anything was
/// there to take.
fn wallpaper_clear(
    section: &mut gevrek_model::wallpaper::WallpaperConfig,
    output: Option<&str>,
) -> bool {
    match output {
        None => {
            let had = !section.image.is_empty() || !section.outputs.is_empty();
            section.image.clear();
            section.outputs.clear();
            had
        }
        Some(output) => {
            let before = section.outputs.len();
            section
                .outputs
                .retain(|entry| !entry.name.eq_ignore_ascii_case(output));
            section.outputs.len() != before
        }
    }
}

/// A wallpaper section from JSON; what it leaves out is as by default.
fn wallpaper_from_json(text: &str) -> Result<gevrek_model::wallpaper::WallpaperConfig> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| anyhow!("not JSON: {error}"))?;
    if !value.is_object() {
        return Err(anyhow!(
            "a wallpaper is an object: {{\"image\": ..., \"fit\": ..., \"outputs\": [...]}}"
        ));
    }
    serde_json::from_value(value).map_err(|error| anyhow!("not a wallpaper: {error}"))
}

/// The displays out of the window manager's state, as `gevrek wallpaper
/// --json` lists them: each with its connector, its place and its size.
fn wallpaper_displays(state: &serde_json::Value) -> Vec<serde_json::Value> {
    let number = |value: &serde_json::Value| value.as_f64().unwrap_or(0.0).round() as i64;
    state["monitors"]
        .as_array()
        .map(|monitors| {
            monitors
                .iter()
                .map(|monitor| {
                    let name = monitor["connector"]
                        .as_str()
                        .filter(|connector| !connector.is_empty())
                        .or(monitor["name"].as_str())
                        .unwrap_or_default();
                    serde_json::json!({
                        "name": name,
                        "x": number(&monitor["position"]["x"]),
                        "y": number(&monitor["position"]["y"]),
                        "width": number(&monitor["size"]["width"]),
                        "height": number(&monitor["size"]["height"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Subcommand)]
enum Commands {
    /// Start Gevrek components
    Start {
        /// Component to start: wm, shell, editor (the settings program,
        /// gevrek studio), or all (the window manager and the shell; the
        /// settings program is started by its own name)
        #[arg(default_value = "all")]
        component: String,

        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
    },

    /// Stop Gevrek components
    Stop {
        /// Component to stop (wm, editor, shell, or all)
        #[arg(default_value = "all")]
        component: String,

        /// Force kill without graceful shutdown
        #[arg(short, long)]
        force: bool,
    },

    /// Restart Gevrek components
    Restart {
        /// Component to restart (wm, editor, shell, or all)
        #[arg(default_value = "all")]
        component: String,
    },

    /// Show status of Gevrek components
    Status,

    /// Query window manager state
    State {
        /// Everything the window manager publishes (outputs, workspaces,
        /// windows with their details) as one JSON object, for scripts.
        #[arg(long)]
        json: bool,
    },

    /// Show health status of the window manager
    Health,

    /// Show the running WM's versioned features and dispatcher surface
    Capabilities,

    /// Print the configuration file gevrek uses when none is named
    ConfigPath,

    /// The helper for the windows of programs that run as an
    /// administrator (Windows): `start` it for now (Windows asks first),
    /// `install` the task that starts it without a question from then on
    /// (asked once), `remove` that task, or show its `status`
    Elevated {
        /// start, install, remove or status
        #[arg(default_value = "status")]
        action: String,
    },

    /// Print the JSON Schema of a configuration file (for an editor to
    /// complete and check `config.json` with), or with `--settings` the
    /// list of settings with their place and description
    Schema {
        #[arg(long)]
        settings: bool,
    },

    /// End the session: the window manager stops
    Quit,

    /// Lock the screen (as `gevrek shell lock` does)
    Lock,

    /// Switch the displays off until the next key or movement (as
    /// `gevrek shell displays_off` does)
    DisplaysOff,

    /// Answer the trial of new display settings: `keep` them, or `revert`
    /// to the ones before (which also happens by itself after 15 seconds)
    OutputTrial { answer: String },

    /// Ask the shell to do something: `gevrek shell launcher`, `lock`,
    /// `volume_up`, `screenshot_region`, ... (`gevrek shell list`); and
    /// `gevrek shell osd "<words>" [0-100]` to show words, with a level,
    /// on the on-screen display
    Shell {
        action: String,
        /// What `osd` shows: the words, then a level if there is one
        rest: Vec<String>,
    },

    /// Learn gevrek by doing: begin the tutorial, or go on where it was
    /// left; `gevrek tutorial leave` ends it, `gevrek tutorial welcome`
    /// opens the window of the first start again
    Tutorial {
        /// `leave`, `next`, `back` or `welcome`; nothing: begin
        what: Option<String>,
    },

    /// Show a notification of gevrek's own: `gevrek notify "<title>"
    /// "<text>"` (the shell that is running shows it)
    Notify {
        title: String,
        #[arg(default_value = "")]
        text: String,
    },

    /// Switch the keyboard layout: `next`, `previous`, or a number from 0
    KeyboardLayout { target: String },

    /// For automated checks: send made-up input to a window manager started
    /// with --test-input. It goes through the WM's own input path, so
    /// bindings and drags behave as with a real keyboard and mouse.
    Input {
        #[command(subcommand)]
        step: InputStep,
    },

    /// Run one command given as the JSON a binding holds for it:
    /// `gevrek dispatch '{"command":"exec","program":"kitty"}'`.
    /// Every command of the configuration can be sent this way.
    Dispatch {
        /// The command as JSON
        json: String,
    },

    /// Execute a JSON array of typed DispatchCommand values as one runtime batch
    Batch {
        /// JSON file containing an array of tagged DispatchCommand objects
        file: PathBuf,

        /// Skip remaining commands after the first runtime failure
        #[arg(long)]
        stop_on_error: bool,
    },

    /// Pause execution of the node graph
    Pause,

    /// Resume execution of the node graph
    Resume,

    /// Push a complete Config or a raw NodeGraph to the window manager
    Push {
        /// Path to the graph JSON file
        file: PathBuf,
    },

    /// Export the complete live configuration to a JSON file
    ConfigPull { file: PathBuf },

    /// Validate a complete configuration file without applying it
    ConfigValidate { file: PathBuf },

    /// List the built-in example configurations, or use one: write it to a
    /// file (--out) or make it the running configuration (--apply)
    Example {
        /// The example's id, as the list shows it (aurora, paper, ...)
        id: Option<String>,
        /// Replace the running configuration with the example
        #[arg(long)]
        apply: bool,
        /// Write the example to this file
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// List persistent window rules from the live configuration
    RuleList {
        /// Print the complete rules as JSON
        #[arg(long)]
        json: bool,
    },

    /// Insert or replace one persistent WindowRuleProfile from JSON
    RuleUpsert { file: PathBuf },

    /// Remove one persistent window rule by exact name
    RuleRemove { name: String },

    /// The wallpaper: show it (`--json`: with the displays, for scripts),
    /// or change it: `gevrek wallpaper set <file> [--output DP-1] [--fit
    /// fill]`, `gevrek wallpaper clear [--output DP-1]`, `gevrek wallpaper
    /// apply <file|->`
    Wallpaper {
        /// The wallpaper section and the displays as one JSON object
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        action: Option<WallpaperAction>,
    },

    /// Subscribe to window manager events and logs
    Log {
        /// Show only errors
        #[arg(long)]
        errors_only: bool,

        /// Emit newline-delimited JSON for scripts and extension hosts
        #[arg(long)]
        json: bool,
    },

    /// Focus the next window in the current workspace
    FocusNext,

    /// Focus the previous window in the current workspace
    FocusPrevious,

    /// Focus the previously active managed window
    FocusLast,

    /// Focus a managed window selected by handle or metadata
    FocusWindow {
        /// Selector: active, floating, tiled, handle/address:<id>, pid:<id>, title/class/exe/path:<text>, or tag:<tag>
        selector: String,
    },

    /// Focus a window even when a no-focus rule would prevent it (one-shot override)
    ForceFocus {
        /// Selector: active, floating, tiled, handle/address:<id>, pid:<id>, title/class/exe/path:<text>, or tag:<tag>
        selector: String,
    },

    /// Close a selected managed window
    CloseWindow { selector: String },

    /// Toggle borderless fullscreen for a selected managed window
    ToggleFullscreenWindow { selector: String },

    /// Toggle preferred-size pseudotiling for a selected tiled window
    TogglePseudotileWindow { selector: String },

    /// Add a tag to a selected managed window
    TagAddWindow { selector: String, tag: String },

    /// Remove a tag from a selected managed window
    TagRemoveWindow { selector: String, tag: String },

    /// Toggle a tag on a selected managed window
    TagToggleWindow { selector: String, tag: String },

    /// Clear all manually assigned tags from a selected managed window
    TagClearWindow { selector: String },

    /// Move a selected window to a workspace without changing the view
    MoveWindowTo { window: String, workspace: String },

    /// Move a selected window to a workspace and follow it
    MoveWindowToAndFollow { window: String, workspace: String },

    /// Toggle floating mode for a selected managed window
    ToggleFloatWindow { selector: String },

    /// Make a selected managed window floating
    FloatWindow { selector: String },

    /// Make a selected managed window tiled
    TileWindow { selector: String },

    /// Toggle always-on-top for a selected managed window
    TogglePinWindow { selector: String },

    /// Make a selected managed window always-on-top
    PinWindow { selector: String },

    /// Remove always-on-top from a selected managed window
    UnpinWindow { selector: String },

    /// Float and center a selected managed window
    CenterWindow { selector: String },

    /// Raise a selected managed window
    RaiseWindow { selector: String },

    /// Lower a selected managed window
    LowerWindow { selector: String },

    /// Move a selected managed window to or by pixel coordinates
    MoveWindow {
        selector: String,
        #[arg(allow_hyphen_values = true)]
        x: i32,
        #[arg(allow_hyphen_values = true)]
        y: i32,
        /// Treat x/y as deltas instead of absolute screen coordinates
        #[arg(long)]
        relative: bool,
    },

    /// Resize a selected managed window to or by pixel dimensions
    ResizeWindow {
        selector: String,
        #[arg(allow_hyphen_values = true)]
        width: i32,
        #[arg(allow_hyphen_values = true)]
        height: i32,
        /// Treat width/height as deltas instead of absolute dimensions
        #[arg(long)]
        relative: bool,
    },

    /// Move current window to next workspace without changing the view
    MoveToNext,

    /// Move current window to previous workspace without changing the view
    MoveToPrevious,

    /// Switch workspace by index, name, relative offset, history, or availability
    Workspace {
        /// Selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
    },

    /// Focus a monitor without changing its active workspace
    FocusMonitor {
        /// Selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Focus the monitor containing the mouse cursor
    FocusMonitorAtCursor,

    /// Focus a workspace on a specific monitor
    FocusMonitorWorkspace {
        /// Monitor selector: primary, index, name:<name>, next, or previous
        monitor: String,
        /// Workspace selector on that monitor: index, name:<name>, next, previous, back, empty, +N, or -N
        workspace: String,
    },

    /// Move the active workspace to another monitor
    MoveWorkspaceToMonitor {
        /// Selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Swap active workspaces with another monitor
    SwapActiveWorkspaces {
        /// Selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Move the active window to another monitor's active workspace and follow it
    MoveToMonitor {
        /// Selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Move the active window to another monitor's active workspace without changing the view
    SendToMonitor {
        /// Selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Move a selected window to another monitor's active workspace without changing the view
    MoveWindowToMonitor {
        /// Window selector: active, handle:<id>, pid:<id>, title:<text>, class:<text>, exe:<text>, path:<text>, or tag:<tag>
        window: String,
        /// Monitor selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Move a selected window to another monitor's active workspace and follow it
    MoveWindowToMonitorAndFollow {
        /// Window selector: active, handle:<id>, pid:<id>, title:<text>, class:<text>, exe:<text>, path:<text>, or tag:<tag>
        window: String,
        /// Monitor selector: primary, index, name:<name>, next, or previous
        selector: String,
    },

    /// Force the layout to re-apply on every monitor's active workspace
    Retile,

    /// Suspend or resume tiling globally (windows stay managed but are not repositioned)
    ToggleTiling,

    /// Toggle tiling for a workspace on the focused monitor
    ToggleWorkspaceTiling {
        /// Selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
    },

    /// Toggle the float override for a workspace on the focused monitor
    ToggleWorkspaceFloatOverride {
        /// Selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
    },

    /// Materialize a new workspace on the focused monitor (optionally named)
    NewWorkspace {
        /// Optional workspace name; defaults to the next "Workspace N" index
        name: Option<String>,
    },

    /// Close the focused workspace when it is empty
    CloseWorkspace,

    /// Create workspaces on a monitor until it has at least the given count
    EnsureWorkspaces {
        /// Monitor selector: primary, index, name:<name>, next, or previous
        monitor: String,
        /// Workspace count to ensure (1-128)
        count: usize,
    },

    /// Create a named workspace on a monitor when it does not exist yet
    EnsureNamedWorkspace {
        /// Monitor selector: primary, index, name:<name>, next, or previous
        monitor: String,
        /// Workspace name to ensure
        name: String,
    },

    /// Toggle the focused workspace between column and append container behaviour
    ToggleWindowContainerBehaviour,

    /// Toggle container behaviour for a workspace on the focused monitor
    ToggleWorkspaceWindowContainerBehaviour {
        /// Selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
    },

    /// Toggle the title bar of the active window
    ToggleTitleBar,

    /// Toggle the title bar of a selected window
    ToggleWindowTitleBar {
        /// Window selector: active, handle:<id>, pid:<id>, title:<text>, class:<text>, exe:<text>, path:<text>, or tag:<tag>
        selector: String,
    },

    /// Adopt a window into tiling (reverses unmanage or an ignore rule)
    Manage {
        /// Window selector: active, handle:<id>, pid:<id>, title:<text>, class:<text>, exe:<text>, path:<text>, or tag:<tag>
        selector: String,
    },

    /// Release a window from tiling for this session (until re-managed or restart)
    Unmanage {
        /// Window selector: active, handle:<id>, pid:<id>, title:<text>, class:<text>, exe:<text>, path:<text>, or tag:<tag>
        selector: String,
    },

    /// Restore all windows hidden by workspace switches, groups, or scratchpads
    RestoreWindows,

    /// Toggle warping the cursor to the focused window on dispatch focus changes
    ToggleMouseFollowsFocus,

    /// Toggle floating mode for the focused window
    ToggleFloat,

    /// Make the focused window floating
    Float,

    /// Make the focused window tiled
    Tile,

    /// Float and center the focused window in its monitor work area
    Center,

    /// Put the shown workspace's floating windows out of the way, or bring them back
    FloatingAside,

    /// Show the desktop (every display's windows out of sight), or bring the windows back
    ShowDesktop {
        /// on: show the desktop; off: bring the windows back; toggle (or nothing): whichever is not the case now
        #[arg(value_enum, default_value_t = CliSwitch::Toggle)]
        switch: CliSwitch,
    },

    /// Raise the focused window to the top of its Z-order band
    Raise,

    /// Lower the focused window to the bottom of its Z-order band
    Lower,

    /// Add a normalized tag to the focused window
    TagAdd { tag: String },

    /// Remove a tag from the focused window
    TagRemove { tag: String },

    /// Toggle a tag on the focused window
    TagToggle { tag: String },

    /// Clear all manually assigned tags from the focused window
    TagClear,

    /// Enable a named rule for this WM session
    RuleEnable { name: String },

    /// Disable a named rule for this WM session
    RuleDisable { name: String },

    /// Toggle a named rule for this WM session
    RuleToggle { name: String },

    /// Focus the nearest window in a spatial direction
    Focus {
        #[arg(value_enum)]
        direction: CliDirection,
    },

    /// Swap the active tiled window with its nearest neighbor
    Swap {
        #[arg(value_enum)]
        direction: CliDirection,
    },

    /// Move the active window by a pixel delta
    MoveActive {
        #[arg(allow_hyphen_values = true)]
        x: i32,
        #[arg(allow_hyphen_values = true)]
        y: i32,
    },

    /// Resize the active window by a pixel delta
    ResizeActive {
        #[arg(allow_hyphen_values = true)]
        width: i32,
        #[arg(allow_hyphen_values = true)]
        height: i32,
    },

    /// Close the active window
    Close,

    /// Minimize the active window
    Minimize,

    /// Toggle the active window's maximized state
    ToggleMaximize,

    /// Toggle borderless fullscreen on the active window
    ToggleFullscreen,

    /// Toggle always-on-top for the active window
    TogglePin,

    /// Make the active window always-on-top
    Pin,

    /// Remove always-on-top from the active window
    Unpin,

    /// Move the active window to a selected workspace without changing the view
    MoveTo {
        /// Selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
    },

    /// Move the active window to a workspace and follow it
    MoveToAndFollow {
        /// Selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
    },

    /// Rename an existing workspace on the focused monitor
    WorkspaceRename {
        /// Existing selector: index, name:<name>, next, previous, back, empty, +N, or -N
        selector: String,
        /// New non-empty workspace name
        name: String,
    },

    /// Cycle to the next layout
    LayoutNext,

    /// Cycle to the previous layout
    LayoutPrevious,

    /// Select a layout by name
    LayoutSet {
        /// bsp, grid, columns, rows, monocle, ultrawide, master, centered, spiral, or dwindle
        layout: String,
    },

    /// Set the master-area ratio
    LayoutRatio {
        /// Ratio from 0.1 through 0.9
        ratio: f32,
    },

    /// Adjust the master-area ratio
    LayoutRatioAdjust {
        /// Signed ratio delta, for example 0.05 or -0.05
        delta: f32,
    },

    /// Set the number of master windows
    LayoutMasterCount {
        /// Master count from 1 through 8
        count: usize,
    },

    /// Rotate the active layout clockwise
    LayoutRotate,

    /// Set the inner layout gap in pixels
    LayoutGap { pixels: i32 },

    /// Adjust the inner layout gap in pixels
    LayoutGapAdjust { delta: i32 },

    /// Promote the active tiled window to the master position
    LayoutPromote,

    /// Grow or shrink one edge of the active tiled window by the resize delta
    ResizeEdge {
        #[arg(value_enum)]
        edge: CliDirection,
        #[arg(value_enum)]
        sizing: CliSizing,
    },

    /// Grow or shrink the active tiled window along an axis by the resize delta
    ResizeAxis {
        #[arg(value_enum)]
        axis: CliAxis,
        #[arg(value_enum)]
        sizing: CliSizing,
    },

    /// Set the pixel delta used by resize-edge and resize-axis (session-only)
    ResizeDelta { pixels: i32 },

    /// Mirror the workspace layout along an axis
    FlipLayout {
        #[arg(value_enum)]
        axis: CliAxis,
    },

    /// Temporarily switch the workspace to monocle, restoring the layout on a second toggle
    ToggleMonocle,

    /// Lock or unlock the workspace layout against ratio changes
    ToggleLayoutLock,

    /// Stack every window on the focused workspace into one group
    StackAll,

    /// Dissolve the focused group so every window gets its own container
    UnstackAll,

    /// Toggle the stackbar tab strip shown above grouped containers
    ToggleStackbar,

    /// Save all workspaces' resize dimensions (split ratios, layout, master settings) to a JSON file
    SaveResize {
        /// Destination JSON file
        path: PathBuf,
    },

    /// Restore resize dimensions saved by save-resize
    LoadResize {
        /// JSON file previously written by save-resize
        path: PathBuf,
    },

    /// Save resize dimensions to the standard temp file (gevrek.resize.json)
    QuickSaveResize,

    /// Restore resize dimensions from the standard temp file (gevrek.resize.json)
    QuickLoadResize,

    /// Watch the configuration file and hot-reload it on external changes
    WatchConfiguration {
        /// true to enable watching, false to disable
        #[arg(action = clap::ArgAction::Set)]
        enabled: bool,
    },

    /// Toggle the transparency system: when off, all windows render fully opaque
    ToggleTransparency,

    /// Set the inactive-window transparency alpha at runtime (20-255)
    TransparencyAlpha {
        /// Alpha value for unfocused windows
        alpha: i32,
    },

    /// Set the global work-area offset applied to every monitor
    GlobalWorkAreaOffset {
        #[arg(allow_hyphen_values = true)]
        left: i32,
        #[arg(allow_hyphen_values = true)]
        top: i32,
        #[arg(allow_hyphen_values = true)]
        right: i32,
        #[arg(allow_hyphen_values = true)]
        bottom: i32,
    },

    /// Set the focused workspace's work-area offset
    WorkspaceWorkAreaOffset {
        #[arg(allow_hyphen_values = true)]
        left: i32,
        #[arg(allow_hyphen_values = true)]
        top: i32,
        #[arg(allow_hyphen_values = true)]
        right: i32,
        #[arg(allow_hyphen_values = true)]
        bottom: i32,
    },

    /// Set the focused workspace's outer padding (gaps_out)
    WorkspacePadding {
        /// Padding in pixels (0-500)
        padding: i32,
    },

    /// Adjust the focused workspace's outer padding by a delta
    AdjustWorkspacePadding {
        /// Delta in pixels (may be negative)
        #[arg(allow_hyphen_values = true)]
        delta: i32,
    },

    /// Set the focused workspace's container padding (gaps_in)
    ContainerPadding {
        /// Padding in pixels (0-500)
        padding: i32,
    },

    /// Adjust the focused workspace's container padding by a delta
    AdjustContainerPadding {
        /// Delta in pixels (may be negative)
        #[arg(allow_hyphen_values = true)]
        delta: i32,
    },

    /// Start gevrek when you sign in (a scheduled task; where tasks are
    /// refused, a script in the Startup folder)
    EnableAutostart,

    /// Remove the logon autostart registration
    DisableAutostart,

    /// Say whether, and how, gevrek is started when you sign in
    AutostartStatus,

    /// Look for a newer gevrek and install it (Windows, where gevrek was
    /// installed from its package; elsewhere it only says what there is).
    /// A gevrek that is running is stopped for it and started again
    Update {
        /// Only say whether there is a newer release
        #[arg(long)]
        check: bool,
        /// `on` or `off`: whether gevrek looks by itself, once a day when
        /// it starts, and installs what it found the next time it starts
        #[arg(long, value_name = "on|off")]
        automatic: Option<String>,
        /// What `gevrek start` leaves running behind itself: look if it is
        /// time to, and get a newer release ready
        #[arg(long, hide = true)]
        background: bool,
    },

    /// Move the active tiled window to the front of the layout, preserving other positions
    Promote,

    /// Focus the front (master) container without reordering windows
    PromoteFocus,

    /// Focus the window in a direction and promote it to the front of the layout
    PromoteWindow {
        #[arg(value_enum)]
        direction: CliDirection,
    },

    /// Set how Dwindle chooses split directions
    DwindleDirection {
        #[arg(value_enum)]
        direction: CliDwindleDirection,
    },

    /// Cycle Dwindle between alternating, horizontal, and vertical splits
    DwindleCycleSplit,

    /// Toggle preservation of existing Dwindle split directions
    DwindlePreserveSplit,

    /// Toggle preferred-size pseudotiling for the active tiled window
    DwindlePseudotile,

    /// Create a group with the next container, or remove the active tab
    GroupToggle,

    /// Create a group for a selected window, or remove it from its group
    GroupToggleWindow { selector: String },

    /// Lock or unlock the active group against membership changes
    GroupLock,

    /// Lock or unlock the group containing a selected window
    GroupLockWindow { selector: String },

    /// Focus the next tab in the active group
    GroupNext,

    /// Focus the previous tab in the active group
    GroupPrevious,

    /// Focus the next tab relative to a selected window
    GroupNextWindow { selector: String },

    /// Focus the previous tab relative to a selected window
    GroupPreviousWindow { selector: String },

    /// Focus a zero-based tab index in the active group
    GroupFocus { index: usize },

    /// Focus a zero-based tab index in a selected window's group
    GroupFocusWindow { selector: String, index: usize },

    /// Move the active tab forward in its group order
    GroupMoveForward,

    /// Move the active tab backward in its group order
    GroupMoveBackward,

    /// Move a selected window forward in its group order
    GroupMoveWindowForward { selector: String },

    /// Move a selected window backward in its group order
    GroupMoveWindowBackward { selector: String },

    /// Merge the active container into the next container
    GroupMergeNext,

    /// Merge the active container into the previous container
    GroupMergePrevious,

    /// Remove the active window from its group
    GroupRemove,

    /// Toggle the window overview
    Expose,

    /// Open (or close) the window that lists the current shortcuts
    Shortcuts,

    /// Toggle a named scratchpad profile from the active configuration
    Scratchpad { id: String },

    /// Start a program in the window manager's session: `gevrek exec kitty`,
    /// `gevrek exec -- firefox --new-window`
    Exec {
        program: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Launch a configured startup application now
    StartupLaunch { id: String },

    /// Stop a configured startup application
    StartupStop { id: String },

    /// Restart a configured startup application
    StartupRestart { id: String },

    /// Snap the active window to a zero-based snap zone index
    #[command(alias = "zone")]
    SnapZone { index: usize },

    /// Snap the active window to the next snap zone
    #[command(alias = "zone-next")]
    SnapZoneNext,

    /// Snap the active window to the previous snap zone
    #[command(alias = "zone-previous")]
    SnapZonePrevious,

    /// Enter a named advanced-binding submap
    Submap { name: String },

    /// Return to the default advanced-binding submap
    SubmapReset,

    /// The zones on the desktop (the containers that hold its icons):
    /// list, create and change them, put items into them, hide and peek,
    /// run the rules, make zones by type (`auto-create`), draw the icons
    /// grey (`grey`), keep and restore layouts, follow what happens.
    /// `gevrek zones <word> --help` says what each word takes. Every word
    /// prints JSON with --json. Ends with 0 when done, 1 when the zones
    /// refused or what was named is not there, 3 when no shell runs the
    /// zones, 4 when the shell did not answer. (Snap zones for windows
    /// are `snap-zone`.)
    Zones {
        #[command(subcommand)]
        action: gevrek::zones_cli::ask::ZonesAction,
        /// Print JSON: what is listed as one object, and for everything
        /// that is asked of the zones their answer as
        /// `{success, delivered, error, events}`
        #[arg(long, global = true)]
        json: bool,
        /// How long to wait for the zones' answer (the window manager
        /// waits 5 seconds where none is said, and never more than 30)
        #[arg(long, global = true, value_name = "MS")]
        timeout_ms: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliDwindleDirection {
    Alternate,
    Horizontal,
    Vertical,
}

/// On, off, or whichever is not the case now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliSwitch {
    On,
    Off,
    Toggle,
}

impl CliSwitch {
    /// As a dispatch request's argument says it.
    fn argument(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Toggle => "",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSizing {
    Increase,
    Decrease,
}

impl CliSizing {
    fn config_name(self) -> &'static str {
        match self {
            Self::Increase => "increase",
            Self::Decrease => "decrease",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliAxis {
    Horizontal,
    Vertical,
    HorizontalAndVertical,
}

impl CliAxis {
    fn config_name(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::HorizontalAndVertical => "horizontal-and-vertical",
        }
    }
}

impl CliDwindleDirection {
    fn config_name(self) -> &'static str {
        match self {
            Self::Alternate => "alternate",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

impl From<CliDirection> for proto::Direction {
    fn from(direction: CliDirection) -> Self {
        match direction {
            CliDirection::Left => Self::Left,
            CliDirection::Right => Self::Right,
            CliDirection::Up => Self::Up,
            CliDirection::Down => Self::Down,
        }
    }
}

/// Whether a file's JSON is a node graph on its own (`nodes` at the top)
/// and not a configuration (which has the graph under `node_graph`).
fn is_bare_graph(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value.as_object().map(|keys| {
                keys.contains_key("nodes")
                    && !keys.contains_key("node_graph")
                    && !keys.contains_key("desktop")
            })
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Component {
    Wm,
    Editor,
    /// `gevrek-shell`: the panel (on Linux the rest of the shell too).
    Shell,
}

/// Whether `start all` starts the shell: on Windows, where its program is
/// beside this one. On Linux the session starts the shell itself (a unit
/// of its own).
fn shell_is_started(shell_is_there: bool, on_windows: bool) -> bool {
    on_windows && shell_is_there
}

/// Whether a component's program is in the folder of this one.
fn program_is_beside(component: &Component) -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .is_some_and(|folder| {
            folder
                .join(format!(
                    "{}{}",
                    component.binary_name(),
                    std::env::consts::EXE_SUFFIX
                ))
                .exists()
        })
}

impl Component {
    fn name(&self) -> &'static str {
        match self {
            Component::Wm => "wm",
            Component::Editor => "editor",
            Component::Shell => "shell",
        }
    }

    fn binary_name(&self) -> &'static str {
        match self {
            // The window manager is a different program on each system.
            Component::Wm if cfg!(any(windows, target_os = "macos")) => "gevrek-wm",
            Component::Wm => "gevrek-wl",
            // The settings program; `editor` is the name it had before.
            Component::Editor => "gevrek-studio",
            Component::Shell => "gevrek-shell",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "wm" | "window-manager" | "windowmanager" => Some(Component::Wm),
            "editor" | "studio" | "settings" | "node-editor" | "nodeeditor" => {
                Some(Component::Editor)
            }
            "shell" | "panel" => Some(Component::Shell),
            _ => None,
        }
    }

    /// Every component: what `status` lists and `stop all` stops. The
    /// shell is among them on Windows only: on Linux the session starts
    /// and stops it (a unit of its own), and this program knows nothing
    /// of it that it could honestly list.
    fn all() -> Vec<Component> {
        let mut all = vec![Component::Wm, Component::Editor];
        if cfg!(windows) {
            all.push(Component::Shell);
        }
        all
    }

    /// What `start all` starts: what the desktop is made of: the window
    /// manager, and the shell where this program starts it (see
    /// [`shell_is_started`]). Not the settings program: that is opened
    /// by whoever wants to change something (`gevrek start editor`, the
    /// launcher's button), and a desktop that starts is not asked to
    /// show it.
    fn all_to_start() -> Vec<Component> {
        let mut all = vec![Component::Wm];
        if shell_is_started(program_is_beside(&Component::Shell), cfg!(windows)) {
            all.push(Component::Shell);
        }
        all
    }
}

/// The components a word on the command line means; `starting` says
/// whether they are to be started (then `all` has one bar, not both).
fn parse_components(component: &str, starting: bool) -> Vec<Component> {
    if component == "all" && starting {
        Component::all_to_start()
    } else if component == "all" {
        Component::all()
    } else {
        Component::from_str(component)
            .map(|c| vec![c])
            .unwrap_or_default()
    }
}

/// Check if a process is running by PID
fn is_process_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
            if let Ok(h) = handle {
                let mut exit_code = 0;
                let running = GetExitCodeProcess(h, &mut exit_code).is_ok()
                    && exit_code == STILL_ACTIVE.0 as u32;
                let _ = CloseHandle(h);
                return running;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        // On Unix, send signal 0 to check if process exists
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

fn process_matches_component(pid: u32, component: &Component) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };

        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut buffer = vec![0u16; 32_768];
            let mut size = buffer.len() as u32;
            let result = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(handle);
            if result.is_err() {
                return false;
            }

            let path = String::from_utf16_lossy(&buffer[..size as usize]);
            PathBuf::from(path)
                .file_stem()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case(component.binary_name()))
                .unwrap_or(false)
        }
    }

    #[cfg(not(windows))]
    {
        let _ = component;
        is_process_running(pid)
    }
}

#[cfg(windows)]
fn discover_component_pids(component: &Component) -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut pids = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return pids;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|character| *character == 0)
                    .unwrap_or(entry.szExeFile.len());
                let executable = String::from_utf16_lossy(&entry.szExeFile[..length]);
                let matches = PathBuf::from(executable)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(component.binary_name()));
                // (Only of this session: another person signed in on the
                // same machine has a gevrek of their own, which is neither
                // "already running" for this one nor this one's to stop.)
                if matches && in_this_session(entry.th32ProcessID) {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Whether a process runs in the session this one runs in. A process
/// Windows does not tell the session of counts as one of it, as before.
#[cfg(windows)]
fn in_this_session(pid: u32) -> bool {
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    let (mut own, mut other) = (0u32, 0u32);
    // SAFETY: plain questions with valid places for the answers.
    unsafe {
        if ProcessIdToSessionId(std::process::id(), &mut own).is_err()
            || ProcessIdToSessionId(pid, &mut other).is_err()
        {
            return true;
        }
    }
    own == other
}

#[cfg(not(windows))]
fn discover_component_pids(_component: &Component) -> Vec<u32> {
    Vec::new()
}

/// Which of the processes found by their program's name belong to this
/// instance. `claims` is what the programs said about themselves (see
/// `gevrek_model::paths::claim_process`), with whether each is still
/// running as that program.
///
/// A process that claims to be this instance's is. Of the others, the
/// main instance takes those nobody else claims (a program of an older
/// build says nothing about itself, and was always the main one's);
/// another instance takes none, since it cannot tell them from the main
/// instance's.
fn pids_of_instance(
    instance: &str,
    discovered: &[u32],
    claims: &[(String, u32, bool)],
) -> Vec<u32> {
    let mut pids: Vec<u32> = claims
        .iter()
        .filter(|(owner, _, running)| *running && owner == instance)
        .map(|(_, pid, _)| *pid)
        .collect();
    if instance == gevrek_model::paths::MAIN_INSTANCE {
        pids.extend(discovered.iter().copied().filter(|pid| {
            !claims
                .iter()
                .any(|(owner, claimed, running)| *running && claimed == pid && owner != instance)
        }));
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn component_pids(component: &Component) -> Vec<u32> {
    let claims: Vec<(String, u32, bool)> = gevrek_model::paths::claimed_processes(component.name())
        .into_iter()
        .map(|(instance, pid, file)| {
            let running = is_process_running(pid) && process_matches_component(pid, component);
            if !running {
                // Left behind by a process that ended.
                log::debug!(
                    "{} said that process {pid} is {instance}'s, which has ended: the note is removed",
                    file.display()
                );
                if let Err(error) = std::fs::remove_file(&file) {
                    log::debug!("{} was not removed: {error}", file.display());
                }
            }
            (instance, pid, running)
        })
        .collect();
    let mut pids = pids_of_instance(
        &gevrek_model::paths::instance(),
        &discover_component_pids(component),
        &claims,
    );
    if let Some(pid) = read_pid(component) {
        if is_process_running(pid) && process_matches_component(pid, component) {
            pids.push(pid);
        } else {
            log::debug!(
                "the pid file of {} names process {pid}, which is not it any more",
                component.name()
            );
            remove_pid(component);
        }
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Read PID from file
fn read_pid(component: &Component) -> Option<u32> {
    let path = pid_file(component.name());
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Write PID to file
fn write_pid(component: &Component, pid: u32) -> Result<()> {
    let dir = pid_dir();
    std::fs::create_dir_all(&dir)?;
    let file = pid_file(component.name());
    std::fs::write(&file, pid.to_string())?;
    log::debug!("process {pid} was written to {}", file.display());
    Ok(())
}

/// Remove PID file
fn remove_pid(component: &Component) {
    let file = pid_file(component.name());
    match std::fs::remove_file(&file) {
        Ok(()) => log::debug!("{} was removed", file.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::debug!("{} was not removed: {error}", file.display()),
    }
}

/// File name of the autostart script inside the user's Startup folder.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
const AUTOSTART_SCRIPT_NAME: &str = "gevrek-autostart.cmd";

/// The user's Startup folder (`%APPDATA%\Microsoft\Windows\Start
/// Menu\Programs\Startup` on Windows; `dirs::config_dir` is Roaming AppData).
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn startup_folder() -> Result<PathBuf> {
    let roaming = dirs::config_dir()
        .ok_or_else(|| anyhow!("Could not determine the Roaming AppData folder"))?;
    Ok(roaming
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup"))
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn autostart_script_path_in(startup_folder: &std::path::Path) -> PathBuf {
    startup_folder.join(AUTOSTART_SCRIPT_NAME)
}

/// Batch script run by Explorer at logon: the way gevrek is started where a
/// scheduled task cannot be made. The working directory is the program's
/// folder: the configuration has its standard place now
/// (`gevrek_model::paths::config_file`), but a `config.json` beside the
/// program from before that is still used while the standard file is
/// missing, and it is found by the working directory.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn autostart_script_contents(executable: &std::path::Path) -> String {
    let exe_dir = executable
        .parent()
        .map(|dir| dir.to_path_buf())
        .unwrap_or_default();
    format!(
        "@echo off\r\ncd /d \"{}\"\r\nstart \"\" \"{}\" start all\r\n",
        exe_dir.display(),
        executable.display()
    )
}

/// The scheduled task that starts this instance at sign-in. One folder
/// for gevrek in the task library, one task per instance.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn autostart_task_name(instance: &str) -> String {
    if instance == "main" {
        r"\gevrek\autostart".to_string()
    } else {
        format!(r"\gevrek\autostart-{instance}")
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The task as Task Scheduler takes it (`schtasks /Create /XML`): at the
/// sign-in of `user`, as that user and without raised rights, run
/// `gevrek start all` from the folder of the program. Unlike a script in
/// the Startup folder it is not held back by Explorer's delay for startup
/// programs, runs on battery too, shows no console window, and can be seen
/// and switched off in Task Scheduler.
///
/// Another instance than the session's own is named to the program through
/// its environment, which a task cannot set: `cmd` sets it first.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn autostart_task_xml(executable: &std::path::Path, user: &str, instance: &str) -> String {
    let folder = executable
        .parent()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let program = executable.display().to_string();
    let (command, arguments) = if instance == "main" {
        (program, "start all".to_string())
    } else {
        (
            "cmd.exe".to_string(),
            format!("/c \"set GEVREK_INSTANCE={instance}&& \"{program}\" start all\""),
        )
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts gevrek ({instance}) when {user} signs in. Made by "gevrek enable-autostart"; "gevrek disable-autostart" removes it.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>false</StartWhenAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{folder}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        instance = xml_text(instance),
        user = xml_text(user),
        command = xml_text(&command),
        arguments = xml_text(&arguments),
        folder = xml_text(&folder),
    )
}

/// The person signed in, as Task Scheduler names them (`MACHINE\name`).
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn signed_in_user() -> Result<String> {
    let name = std::env::var("USERNAME")
        .map_err(|_| anyhow!("USERNAME is not set: whose sign-in should start gevrek?"))?;
    Ok(match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{name}"),
        _ => name,
    })
}

/// Run `schtasks` and give back what it said when it failed.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn schtasks(arguments: &[&str]) -> Result<String> {
    let output = Command::new("schtasks")
        .args(arguments)
        .output()
        .map_err(|error| anyhow!("schtasks could not be run: {error}"))?;
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(said)
    } else {
        Err(anyhow!("{}", said.trim()))
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn autostart_task_exists(task: &str) -> bool {
    schtasks(&["/Query", "/TN", task]).is_ok()
}

/// How gevrek is started at sign-in after `enable_autostart`.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
enum Autostart {
    /// By the scheduled task of this name.
    Task(String),
    /// By the script in the Startup folder, because the task could not be
    /// made (why is the second value).
    Script(PathBuf, String),
}

/// Register gevrek to start at sign-in: by a scheduled task, and where
/// that is refused (a policy can forbid tasks) by the script in the
/// Startup folder as before. Only one of the two is left in place, or
/// gevrek would be started twice.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn enable_autostart() -> Result<Autostart> {
    let executable = std::env::current_exe()?;
    let instance = gevrek_model::paths::instance();
    let task = autostart_task_name(&instance);
    let script = autostart_script_path_in(&startup_folder()?);

    let made = signed_in_user().and_then(|user| {
        // Task Scheduler reads the file as UTF-16 with its mark.
        let xml = autostart_task_xml(&executable, &user, &instance);
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        let file =
            std::env::temp_dir().join(format!("gevrek-autostart-{}.xml", std::process::id()));
        std::fs::write(&file, bytes)?;
        let result = schtasks(&[
            "/Create",
            "/TN",
            &task,
            "/XML",
            &file.to_string_lossy(),
            "/F",
        ]);
        let _ = std::fs::remove_file(&file);
        result
    });
    match made {
        Ok(_) => {
            // The script of an earlier version goes: the task replaces it.
            if instance == "main" && script.exists() {
                std::fs::remove_file(&script)?;
            }
            Ok(Autostart::Task(task))
        }
        Err(error) if instance == "main" => {
            if let Some(folder) = script.parent() {
                std::fs::create_dir_all(folder)?;
            }
            std::fs::write(&script, autostart_script_contents(&executable))?;
            Ok(Autostart::Script(script, error.to_string()))
        }
        // The script knows only the session's own instance.
        Err(error) => Err(anyhow!("The scheduled task could not be made: {error}")),
    }
}

/// Remove the autostart registration, whichever kind it is. Succeeds even
/// if it was never enabled. Returns what was removed.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn disable_autostart() -> Result<Vec<String>> {
    let instance = gevrek_model::paths::instance();
    let task = autostart_task_name(&instance);
    let mut removed = Vec::new();
    if autostart_task_exists(&task) {
        schtasks(&["/Delete", "/TN", &task, "/F"])
            .map_err(|error| anyhow!("The scheduled task {task} could not be removed: {error}"))?;
        removed.push(format!("the scheduled task {task}"));
    }
    if instance == "main" {
        let script = autostart_script_path_in(&startup_folder()?);
        if script.exists() {
            std::fs::remove_file(&script)?;
            removed.push(script.display().to_string());
        }
    }
    Ok(removed)
}

/// Say how this instance is started at sign-in, if it is.
#[cfg_attr(target_os = "macos", allow(dead_code))] // (Windows' autostart; macOS has its own: `macos_app`)
fn autostart_status() -> Result<String> {
    let instance = gevrek_model::paths::instance();
    let task = autostart_task_name(&instance);
    if autostart_task_exists(&task) {
        return Ok(format!("Autostart: the scheduled task {task}"));
    }
    let script = autostart_script_path_in(&startup_folder()?);
    if instance == "main" && script.exists() {
        return Ok(format!("Autostart: the script {}", script.display()));
    }
    Ok("Autostart: off".to_string())
}
/// Get component status
fn get_component_status(component: &Component) -> (bool, Option<u32>) {
    let pids = component_pids(component);
    (!pids.is_empty(), pids.first().copied())
}

/// A component started to stay does not get this program's own input and
/// output. On Windows a child inherits every handle that may be inherited,
/// whatever is named as its own output; with ours among them, whoever
/// reads what `gevrek start` says (a script, a check) would go on waiting
/// until the component ends, because the pipe stays open in it.
#[cfg(windows)]
fn keep_own_output_from_children() {
    use windows::Win32::Foundation::{SetHandleInformation, HANDLE_FLAGS, HANDLE_FLAG_INHERIT};
    use windows::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: a query and a flag on this process's own handles; one
        // that is not there, or is a console, is simply left as it is.
        unsafe {
            if let Ok(handle) = GetStdHandle(which) {
                if !handle.is_invalid() {
                    let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
                }
            }
        }
    }
}

/// Start a component
fn start_component(
    component: &Component,
    foreground: bool,
    address: &str,
) -> Result<Option<Child>> {
    let (running, pid) = get_component_status(component);
    if running {
        log::debug!(
            "{} is not started: process {} is it already",
            component.name(),
            pid.unwrap_or_default()
        );
        println!(
            "{} is already running (PID: {})",
            component.name(),
            pid.unwrap()
        );
        return Ok(None);
    }

    let binary = component.binary_name();
    println!("Starting {}...", component.name());

    if *component == Component::Wm {
        ensure_wm_endpoint_available(address)?;
    }

    // Find the binary - check same directory as this executable first
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    let binary_path = if let Some(dir) = exe_dir {
        let path = dir.join(format!("{binary}{}", std::env::consts::EXE_SUFFIX));
        if path.exists() {
            path
        } else {
            PathBuf::from(binary)
        }
    } else {
        PathBuf::from(binary)
    };

    let configure_command = |cmd: &mut Command| {
        let component_address = if *component == Component::Wm {
            address
                .strip_prefix("http://")
                .or_else(|| address.strip_prefix("https://"))
                .unwrap_or(address)
        } else {
            address
        };
        cmd.arg("--address").arg(component_address);
    };

    if foreground {
        // Run in foreground - inherit stdio
        let mut cmd = Command::new(&binary_path);
        configure_command(&mut cmd);
        let child = cmd
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .inspect_err(|error| {
                log::debug!("{} could not be started: {error}", binary_path.display())
            })?;

        log::debug!(
            "{} was started in the foreground: {} as process {}",
            component.name(),
            binary_path.display(),
            child.id()
        );
        write_pid(component, child.id())?;
        println!("{} started (PID: {})", component.name(), child.id());
        Ok(Some(child))
    } else {
        // On macOS the window manager is started as its application, so
        // that macOS holds the bundle (not the terminal that ran this)
        // responsible for it: the Accessibility permission is the bundle's.
        #[cfg(target_os = "macos")]
        if *component == Component::Wm {
            if let Some(bundle) = macos_app::bundle() {
                let log_path = log_file(component.name());
                std::fs::create_dir_all(pid_dir())?;
                let component_address = address
                    .strip_prefix("http://")
                    .or_else(|| address.strip_prefix("https://"))
                    .unwrap_or(address);
                macos_app::start_wm(&bundle, component_address, &log_path)?;
                // (`open` gives no process id: it is found once it runs.)
                let pid = (0..50).find_map(|_| {
                    std::thread::sleep(Duration::from_millis(100));
                    component_pids(component).first().copied()
                });
                let Some(pid) = pid else {
                    // What it said before it ended, where it said it.
                    let said = std::fs::read_to_string(&log_path)
                        .ok()
                        .and_then(|log| {
                            log.lines()
                                .rev()
                                .find(|line| line.starts_with("Error:"))
                                .map(|line| line.trim_start_matches("Error:").trim().to_string())
                        })
                        .unwrap_or_else(|| "it said nothing".to_string());
                    return Err(anyhow!(
                        "the window manager did not start: {said} (its log: {})",
                        log_path.display()
                    ));
                };
                write_pid(component, pid)?;
                println!(
                    "{} started from {} (PID: {pid}, log: {})",
                    component.name(),
                    bundle.display(),
                    log_path.display()
                );
                return Ok(None);
            }
        }
        // On macOS the other programs are started on their own: see
        // `macos_app::spawn_on_its_own`.
        #[cfg(target_os = "macos")]
        {
            let log_path = log_file(component.name());
            std::fs::create_dir_all(pid_dir())?;
            // (The window manager takes its address without the scheme.)
            let component_address = if *component == Component::Wm {
                address
                    .strip_prefix("http://")
                    .or_else(|| address.strip_prefix("https://"))
                    .unwrap_or(address)
            } else {
                address
            };
            let pid = macos_app::spawn_on_its_own(&binary_path, &["--address", component_address], &log_path)?;
            log::debug!(
                "{} was started on its own: {} as process {pid}, its log in {}",
                component.name(),
                binary_path.display(),
                log_path.display()
            );
            write_pid(component, pid)?;
            println!(
                "{} started (PID: {pid}, log: {})",
                component.name(),
                log_path.display()
            );
            return Ok(None);
        }
        // Run as daemon - detach from terminal
        #[allow(unreachable_code)]
        let mut cmd = Command::new(&binary_path);
        configure_command(&mut cmd);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // A console that is not shown. (With DETACHED_PROCESS there is
            // none at all, and a console program the component starts then
            // opens a visible one of its own.)
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
            keep_own_output_from_children();
        }

        // What the component logs goes to a file next to the pid files,
        // started anew each time. How much: as each program decides
        // (gevrek-logging), or as `RUST_LOG` says where it is set.
        let log_path = log_file(component.name());
        let log = std::fs::create_dir_all(pid_dir())
            .and_then(|_| std::fs::File::create(&log_path))
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());

        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .inspect_err(|error| {
                log::debug!("{} could not be started: {error}", binary_path.display())
            })?;

        log::debug!(
            "{} was started: {} as process {}, its log in {}",
            component.name(),
            binary_path.display(),
            child.id(),
            log_path.display()
        );
        write_pid(component, child.id())?;
        println!(
            "{} started (PID: {}, log: {})",
            component.name(),
            child.id(),
            log_path.display()
        );
        Ok(None)
    }
}

/// gevrek as a macOS application (`packaging/macos`): the bundle the window
/// manager runs from, and its start at sign-in.
#[cfg(target_os = "macos")]
mod macos_app {
    use anyhow::{anyhow, Result};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// The bundle's id (`packaging/macos/Info.plist`).
    pub const BUNDLE_ID: &str = "io.github.ulaas.gevrek";

    /// What of this program's surroundings the window manager is given
    /// when it is started through `open` (which gives it none of them).
    const PASSED_ON: &[&str] = &[
        "GEVREK_INSTANCE",
        "GEVREK_MACOS_ONLY",
        "GEVREK_RUNTIME_DIR",
        "RUST_LOG",
    ];

    /// The bundle gevrek runs from: the one this program is in, or else
    /// one installed in ~/Applications or /Applications. `None`: gevrek
    /// runs as bare programs (built, not installed).
    pub fn bundle() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
        if let Some(bundle) = exe
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        {
            return Some(bundle.to_path_buf());
        }
        let home = dirs::home_dir()?;
        [
            home.join("Applications/Gevrek.app"),
            PathBuf::from("/Applications/Gevrek.app"),
        ]
        .into_iter()
        .find(|bundle| bundle.join("Contents/MacOS/gevrek-wm").exists())
    }

    /// Start a program of the bundle in the background so that macOS holds
    /// it, not whoever started it, responsible for what it does: the
    /// questions it asks (the location, recording the screen, bluetooth)
    /// are then Gevrek's, where a terminal that ran `gevrek` would
    /// otherwise answer them, and be named in them. (The same as Chromium
    /// and the terminals do for what they start: the spawn "disclaims"
    /// responsibility.) What it writes to its error stream goes to `log`.
    /// Its process id.
    pub fn spawn_on_its_own(program: &Path, args: &[&str], log: &Path) -> Result<u32> {
        use std::ffi::{CString, OsStr};
        use std::os::unix::ffi::OsStrExt;
        type Disclaim = unsafe extern "C" fn(*mut libc::posix_spawnattr_t, libc::c_int) -> libc::c_int;
        let c = |text: &OsStr| CString::new(text.as_bytes()).map_err(|_| anyhow!("a NUL in {text:?}"));
        let path = c(program.as_os_str())?;
        let mut argv = vec![path.clone()];
        for arg in args {
            argv.push(c(OsStr::new(arg))?);
        }
        let mut argv_ptrs: Vec<*mut libc::c_char> = argv.iter().map(|arg| arg.as_ptr() as *mut _).collect();
        argv_ptrs.push(std::ptr::null_mut());
        let null = c(OsStr::new("/dev/null"))?;
        let log = c(log.as_os_str())?;
        extern "C" {
            static environ: *const *mut libc::c_char;
        }
        // SAFETY: attributes and file actions made, used and destroyed
        // here; the strings outlive the spawn.
        unsafe {
            let mut attributes: libc::posix_spawnattr_t = std::mem::zeroed();
            let mut actions: libc::posix_spawn_file_actions_t = std::mem::zeroed();
            libc::posix_spawnattr_init(&mut attributes);
            libc::posix_spawn_file_actions_init(&mut actions);
            // (Not in the SDK's headers, but in the system since 10.14.)
            let disclaim = libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_spawnattrs_setdisclaim".as_ptr());
            if disclaim.is_null() {
                log::debug!("no way to disclaim responsibility: started as a child");
            } else {
                let disclaim: Disclaim = std::mem::transmute(disclaim);
                disclaim(&mut attributes, 1);
            }
            libc::posix_spawn_file_actions_addopen(&mut actions, 0, null.as_ptr(), libc::O_RDONLY, 0);
            libc::posix_spawn_file_actions_addopen(&mut actions, 1, null.as_ptr(), libc::O_WRONLY, 0);
            libc::posix_spawn_file_actions_addopen(
                &mut actions,
                2,
                log.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o644,
            );
            let mut pid: libc::pid_t = 0;
            let status = libc::posix_spawn(
                &mut pid,
                path.as_ptr(),
                &actions,
                &attributes,
                argv_ptrs.as_ptr(),
                environ,
            );
            libc::posix_spawn_file_actions_destroy(&mut actions);
            libc::posix_spawnattr_destroy(&mut attributes);
            if status != 0 {
                return Err(anyhow!(
                    "{} could not be started: {}",
                    program.display(),
                    std::io::Error::from_raw_os_error(status)
                ));
            }
            Ok(pid as u32)
        }
    }

    /// Start the window manager of `bundle` in the background, its errors
    /// written to `log`.
    pub fn start_wm(bundle: &Path, address: &str, log: &Path) -> Result<()> {
        let mut open = Command::new("/usr/bin/open");
        open.args(["-g", "-n", "-a"]).arg(bundle).arg("--stderr").arg(log);
        for name in PASSED_ON {
            if let Ok(value) = std::env::var(name) {
                open.arg("--env").arg(format!("{name}={value}"));
            }
        }
        open.args(["--args", "--address", address]);
        let status = open.status()?;
        if !status.success() {
            return Err(anyhow!("{} could not be opened ({status})", bundle.display()));
        }
        Ok(())
    }

    fn agent_label() -> String {
        let instance = gevrek_model::paths::instance();
        if instance == gevrek_model::paths::MAIN_INSTANCE {
            BUNDLE_ID.to_string()
        } else {
            format!("{BUNDLE_ID}.{instance}")
        }
    }

    fn agent_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", agent_label())))
    }

    /// The LaunchAgent that starts the window manager when the user signs
    /// in: the bundle's executable, so that macOS holds the bundle
    /// responsible for it.
    pub fn agent_plist(executable: &Path, address: &str, log: &Path, instance: &str) -> String {
        let escape = |text: &str| {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        let environment = if instance == gevrek_model::paths::MAIN_INSTANCE {
            String::new()
        } else {
            format!(
                "\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>GEVREK_INSTANCE</key>\n\t\t<string>{}</string>\n\t</dict>\n",
                escape(instance)
            )
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{executable}</string>
		<string>--address</string>
		<string>{address}</string>
	</array>
{environment}	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>AssociatedBundleIdentifiers</key>
	<string>{bundle_id}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#,
            label = escape(&agent_label()),
            executable = escape(&executable.to_string_lossy()),
            address = escape(address),
            bundle_id = BUNDLE_ID,
            log = escape(&log.to_string_lossy()),
        )
    }

    /// Start gevrek when the user signs in, from the bundle it is
    /// installed as. The agent is written, not loaded: it runs at the next
    /// sign-in (`gevrek start wm` starts it now).
    pub fn enable_autostart(address: &str) -> Result<PathBuf> {
        let bundle = bundle().ok_or_else(|| {
            anyhow!(
                "gevrek is not installed as an application: build and install it first \
                 (packaging/macos/build-app.sh, then install.sh)"
            )
        })?;
        let path = agent_path()?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let instance = gevrek_model::paths::instance();
        std::fs::write(
            &path,
            agent_plist(
                &bundle.join("Contents/MacOS/gevrek-wm"),
                address,
                &super::log_file("wm"),
                &instance,
            ),
        )?;
        Ok(path)
    }

    /// Remove the LaunchAgent; `None` where there was none.
    pub fn disable_autostart() -> Result<Option<PathBuf>> {
        let path = agent_path()?;
        if !path.exists() {
            return Ok(None);
        }
        std::fs::remove_file(&path)?;
        Ok(Some(path))
    }

    pub fn autostart_status() -> String {
        match agent_path() {
            Ok(path) if path.exists() => format!("Autostart: the LaunchAgent {}", path.display()),
            _ => "Autostart: off".to_string(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_agent_starts_the_bundles_window_manager_at_sign_in() {
            let plist = agent_plist(
                Path::new("/Users/someone/Applications/Gevrek.app/Contents/MacOS/gevrek-wm"),
                "local",
                Path::new("/tmp/main.wm.log"),
                gevrek_model::paths::MAIN_INSTANCE,
            );
            assert!(plist.contains("<string>/Users/someone/Applications/Gevrek.app/Contents/MacOS/gevrek-wm</string>"));
            assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
            assert!(plist.contains(BUNDLE_ID));
            assert!(!plist.contains("EnvironmentVariables"));
            let other = agent_plist(Path::new("/a & b/gevrek-wm"), "local:dev", Path::new("/tmp/x"), "dev");
            assert!(other.contains("<string>/a &amp; b/gevrek-wm</string>"));
            assert!(other.contains("<key>GEVREK_INSTANCE</key>\n\t\t<string>dev</string>"));
        }
    }
}

fn ensure_wm_endpoint_available(address: &str) -> Result<()> {
    let address = match gevrek_protocol::Endpoint::parse(address) {
        gevrek_protocol::Endpoint::Local(instance) => {
            // A socket that answers is a running WM; one that does not is
            // left over and the WM replaces it. (A named pipe that exists
            // makes the WM's own start fail with a clear message.)
            #[cfg(unix)]
            {
                let path = gevrek_protocol::endpoint::socket_path(&instance);
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(anyhow!(
                        "Cannot start the WM because one already answers on {}. Stop it, or run another instance with GEVREK_INSTANCE",
                        path.display()
                    ));
                }
            }
            let _ = instance;
            return Ok(());
        }
        gevrek_protocol::Endpoint::Tcp(url) => url,
    };
    let address = address.as_str();
    let socket = address
        .strip_prefix("http://")
        .or_else(|| address.strip_prefix("https://"))
        .unwrap_or(address)
        .trim_end_matches('/');
    let resolved = socket
        .to_socket_addrs()
        .map_err(|error| anyhow!("Invalid WM address '{address}': {error}"))?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(anyhow!("WM address '{address}' did not resolve"));
    }
    if resolved
        .iter()
        .any(|candidate| TcpStream::connect_timeout(candidate, Duration::from_millis(250)).is_ok())
    {
        return Err(anyhow!(
            "Cannot start the WM because {socket} is already in use. Stop the other program or choose another endpoint with --address"
        ));
    }
    Ok(())
}

async fn wait_for_wm(address: &str, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    log::debug!("the window manager at {address} is waited for, up to {timeout:?}");
    // What was the matter the last time, said when it changes and when
    // the wait is given up.
    let mut last = String::new();
    loop {
        let now = match WMClient::connect(address.to_string()).await {
            Ok(mut client) => match client.health_check().await {
                Ok(health) if health.healthy => {
                    log::debug!("the window manager at {address} is healthy: {}", health.version);
                    return Ok(());
                }
                Ok(health) => format!(
                    "it is not healthy yet: {}",
                    health.message.unwrap_or_default()
                ),
                Err(error) => format!("its health was not answered: {error:#}"),
            },
            Err(error) => format!("it is not reached: {error:#}"),
        };
        if now != last {
            log::debug!("the window manager at {address} is waited for: {now}");
            last = now;
        }

        if tokio::time::Instant::now() >= deadline {
            log::debug!("the window manager at {address} was waited for in vain: {last}");
            return Err(anyhow!(
                "Window manager did not become healthy at {} within {:?}",
                address,
                timeout
            ));
        }
        sleep(Duration::from_millis(200)).await;
    }
}

/// Stop a component
async fn stop_component(component: &Component, force: bool, address: &str) -> Result<()> {
    let pids = component_pids(component);
    if pids.is_empty() {
        // A window manager that was not started by this program (a session,
        // a script) has no pid file here: ask it over its endpoint.
        if *component == Component::Wm {
            if let Ok(Ok(mut client)) = tokio::time::timeout(
                Duration::from_secs(2),
                WMClient::connect(address.to_string()),
            )
            .await
            {
                match tokio::time::timeout(Duration::from_secs(2), client.shutdown(false, 0)).await
                {
                    Ok(Ok(response)) if response.success => {
                        println!("{} was asked to stop", component.name());
                        return Ok(());
                    }
                    Ok(Ok(response)) => log::debug!(
                        "the window manager at {address} did not stop: {}",
                        response.error.unwrap_or_default()
                    ),
                    Ok(Err(error)) => {
                        log::debug!("the window manager at {address} was not asked to stop: {error:#}")
                    }
                    Err(_) => log::debug!(
                        "the window manager at {address} did not answer the request to stop within 2 s"
                    ),
                }
            } else {
                log::debug!("no pid of the window manager, and nothing answers at {address}");
            }
        }
        println!("{} is not running", component.name());
        return Ok(());
    }

    if pids.len() == 1 {
        println!("Stopping {} (PID: {})...", component.name(), pids[0]);
    } else {
        println!(
            "Stopping {} processes (PIDs: {})...",
            component.name(),
            pids.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Try graceful shutdown via RPC for WM. Every network operation has a
    // deadline so `gevrek stop all` always reaches the force-stop fallback.
    if *component == Component::Wm && !force {
        let graceful = match tokio::time::timeout(
            Duration::from_secs(2),
            WMClient::connect(address.to_string()),
        )
        .await
        {
            Ok(Ok(mut client)) => {
                match tokio::time::timeout(Duration::from_secs(2), client.shutdown(false, 0)).await
                {
                    Ok(Ok(response)) if response.success => true,
                    Ok(Ok(response)) => {
                        println!(
                            "{} rejected graceful shutdown: {}",
                            component.name(),
                            response
                                .error
                                .unwrap_or_else(|| "unknown error".to_string())
                        );
                        false
                    }
                    Ok(Err(error)) => {
                        println!("{} shutdown RPC failed: {error}", component.name());
                        false
                    }
                    Err(_) => {
                        println!("{} shutdown RPC timed out after 2s", component.name());
                        false
                    }
                }
            }
            Ok(Err(error)) => {
                println!(
                    "Could not connect to {} for graceful shutdown: {error}",
                    component.name()
                );
                false
            }
            Err(_) => {
                println!("Connecting to {} timed out after 2s", component.name());
                false
            }
        };

        if graceful {
            println!("{} shutting down gracefully...", component.name());
            for _ in 0..50 {
                sleep(Duration::from_millis(100)).await;
                if pids.iter().all(|pid| !is_process_running(*pid)) {
                    remove_pid(component);
                    println!("{} stopped", component.name());
                    return Ok(());
                }
            }
            println!(
                "{} did not exit within 5s; using the force-stop fallback",
                component.name()
            );
        } else if pids.iter().all(|pid| !is_process_running(*pid)) {
            remove_pid(component);
            println!("{} stopped", component.name());
            return Ok(());
        } else {
            println!("Using the force-stop fallback for {}", component.name());
        }
    }

    // Force kill
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

        for pid in &pids {
            let terminated = {
                unsafe {
                    if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, *pid) {
                        let result = TerminateProcess(handle, 1).is_ok();
                        let _ = CloseHandle(handle);
                        result
                    } else {
                        false
                    }
                }
            };
            log::debug!(
                "{} (process {pid}) was {}ended by force",
                component.name(),
                if terminated { "" } else { "not " }
            );
            if !terminated && is_process_running(*pid) {
                return Err(anyhow!(
                    "Failed to terminate {} (PID: {})",
                    component.name(),
                    pid
                ));
            }
        }
    }

    #[cfg(not(windows))]
    {
        for pid in &pids {
            // SAFETY: a signal to a process of this component.
            if unsafe { libc::kill(*pid as i32, libc::SIGTERM) } == 0 {
                log::debug!("{} (process {pid}) was sent SIGTERM", component.name());
            } else {
                log::debug!(
                    "{} (process {pid}) could not be sent SIGTERM: {}",
                    component.name(),
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    for _ in 0..20 {
        if pids.iter().all(|pid| !is_process_running(*pid)) {
            remove_pid(component);
            println!("{} stopped", component.name());
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }

    Err(anyhow!(
        "{} process(es) are still running after the stop request (PIDs: {})",
        component.name(),
        pids.iter()
            .filter(|pid| is_process_running(**pid))
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Show status of all components
fn show_status() {
    println!("gevrek components status:");
    println!("-------------------------");

    for component in Component::all() {
        let (running, pid) = get_component_status(&component);
        if running {
            println!("  {} : running (PID: {})", component.name(), pid.unwrap());
        } else {
            println!("  {} : stopped", component.name());
        }
    }
}

/// Query and display WM state
async fn show_state(address: &str) -> Result<()> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let state = client.query_state().await?;

    println!("Window Manager State:");
    println!("--------------------");
    println!(
        "Execution: {}",
        if state.execution_paused {
            "PAUSED"
        } else {
            "RUNNING"
        }
    );
    if state.desktop_shown {
        println!("Desktop: shown (the windows are out of sight until `gevrek show-desktop off`)");
    }
    println!(
        "Animations: {} · {} active",
        if state.animations_enabled {
            "ENABLED"
        } else {
            "DISABLED"
        },
        state.active_animations,
    );
    for channel in &state.animation_channels {
        println!(
            "  {}: {} · {}ms · {}{}",
            channel.scope,
            if channel.enabled {
                "enabled"
            } else {
                "disabled"
            },
            channel.duration_ms,
            channel.curve,
            if channel.enter_style.is_empty() {
                String::new()
            } else {
                format!(
                    " · enter={} exit={}",
                    channel.enter_style, channel.exit_style
                )
            }
        );
    }
    println!(
        "Focus effects: inactive dim={} (alpha {}, fullscreen={}) · shadow={}",
        state.dim_inactive,
        state.inactive_window_opacity,
        state.dim_fullscreen,
        state.shadow_policy
    );
    println!();

    println!("Monitors: {}", state.monitors.len());
    for monitor in &state.monitors {
        println!("  {} ({})", monitor.name, monitor.output_id);
        if let (Some(pos), Some(size)) = (&monitor.position, &monitor.size) {
            println!("    Position: ({}, {})", pos.x, pos.y);
            println!("    Size: {}x{}", size.width, size.height);
        }
        if let (Some(pos), Some(size)) = (&monitor.work_position, &monitor.work_size) {
            println!(
                "    Usable: {}x{} at ({}, {})",
                size.width, size.height, pos.x, pos.y
            );
            println!(
                "    Reserved: left={} top={} right={} bottom={}",
                monitor.reserved_left,
                monitor.reserved_top,
                monitor.reserved_right,
                monitor.reserved_bottom
            );
        }
        println!(
            "    Primary: {} · Focused: {}",
            monitor.is_primary, monitor.is_focused
        );
        // Who the display is wherever it is plugged in (of its EDID).
        if !monitor.stable_id.is_empty() {
            println!("    Stable id: {}", monitor.stable_id);
        }
    }
    println!();

    println!("Workspaces: {}", state.workspaces.len());
    for ws in &state.workspaces {
        println!(
            "  {} ({}) - {} windows",
            ws.name,
            ws.id,
            ws.window_ids.len()
        );
        println!("    Monitor: {}", ws.monitor_id);
        println!("    Layout: {}", ws.layout_type);
        if !ws.master_position.is_empty() {
            println!(
                "    Master: {} · new windows={}",
                ws.master_position, ws.new_window_placement
            );
        }
        if ws.layout_type.eq_ignore_ascii_case("dwindle") {
            println!(
                "    Dwindle: {} · preserve={} · {} pseudotiled",
                ws.dwindle_direction, ws.dwindle_preserve_split, ws.pseudotiled_windows
            );
        }
    }
    println!();

    println!("Windows: {}", state.windows.len());
    for window in &state.windows {
        println!("  [{}] {}", window.window_id, window.title);
        if let (Some(pos), Some(size)) = (&window.position, &window.size) {
            println!("    Position: ({}, {})", pos.x, pos.y);
            println!("    Size: {}x{}", size.width, size.height);
        }
        if !window.tags.is_empty() {
            println!("    Tags: {}", window.tags.join(", "));
        }
        if !window.matched_rules.is_empty() {
            println!("    Rules: {}", window.matched_rules.join(", "));
        }
        println!(
            "    Effects: opacity={}{} · shadow={}",
            window.effective_opacity,
            if window.is_dimmed { " (dimmed)" } else { "" },
            window.shadow_policy
        );
    }
    println!();

    println!("Rules: {}", state.rules.len());
    for rule in &state.rules {
        println!(
            "  [{}] {} ({})",
            if rule.enabled { "on" } else { "off" },
            rule.name,
            if rule.dynamic { "dynamic" } else { "static" }
        );
    }
    println!();

    println!("Scratchpads: {}", state.scratchpads.len());
    for scratchpad in &state.scratchpads {
        let status = if !scratchpad.enabled {
            "disabled"
        } else if scratchpad.pending_spawn {
            "launching"
        } else if scratchpad.visible {
            "visible"
        } else if scratchpad.window_id.is_some() {
            "hidden"
        } else {
            "not running"
        };
        if let Some(handle) = scratchpad.window_id {
            println!("  {}: {} (window {})", scratchpad.id, status, handle);
        } else {
            println!("  {}: {}", scratchpad.id, status);
        }
    }
    println!();
    println!("Startup applications: {}", state.startup.len());
    for process in &state.startup {
        let pid = process
            .pid
            .map(|pid| format!(" · pid {pid}"))
            .unwrap_or_default();
        println!(
            "  {} ({}) · {}{} · restarts {}",
            process.name, process.id, process.status, pid, process.restart_count
        );
        if let Some(code) = process.last_exit_code {
            println!("    Last exit code: {code}");
        }
        if let Some(error) = &process.last_error {
            println!("    Error: {error}");
        }
    }
    println!("Binding submap: {}", state.active_binding_submap);
    if let Some(gesture) = &state.gesture {
        println!(
            "Gestures: {} · {} device{} · {} configured",
            if gesture.available {
                gesture.backend.as_str()
            } else {
                "hardware unavailable"
            },
            gesture.device_count,
            if gesture.device_count == 1 { "" } else { "s" },
            gesture.configured_count,
        );
        if let Some(active) = &gesture.active {
            println!(
                "  {} finger{} {} · {:.0}%",
                active.fingers,
                if active.fingers == 1 { "" } else { "s" },
                active.direction.as_deref().unwrap_or("pending"),
                active.progress * 100.0,
            );
        }
        if let Some(error) = &gesture.last_error {
            println!("  Backend error: {error}");
        }
    }

    Ok(())
}

/// Show health status
/// `gevrek elevated ...`: the helper for elevated windows.
#[cfg(not(windows))]
fn elevated_helper(_address: &str, _action: &str) -> Result<()> {
    println!("Only Windows keeps a program from moving the windows of one with more rights: there is no helper to run here.");
    Ok(())
}

/// `gevrek elevated ...`: the helper for elevated windows.
#[cfg(windows)]
fn elevated_helper(address: &str, action: &str) -> Result<()> {
    use gevrek_win32::elevated::helper_task_name;
    let instance = match gevrek_protocol::Endpoint::parse(address) {
        gevrek_protocol::Endpoint::Local(instance) => instance,
        gevrek_protocol::Endpoint::Tcp(_) => gevrek_model::paths::instance(),
    };
    let helper = std::env::current_exe()?.with_file_name("gevrek-elevated.exe");
    let task = helper_task_name(&instance);
    let pipe = gevrek_win32::elevated_helper::pipe_name(&instance);
    // Start a program with an administrator's rights: Windows asks first.
    let as_administrator = |program: &std::ffi::OsStr, arguments: &str| -> Result<()> {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
        let (program, arguments) = (HSTRING::from(program), HSTRING::from(arguments));
        // SAFETY: the texts live until the call returns.
        let answer = unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("runas"),
                PCWSTR(program.as_ptr()),
                PCWSTR(arguments.as_ptr()),
                None,
                SW_HIDE,
            )
        };
        // (Above 32: started. 5 and 1223 are a "no" to Windows' question.)
        if answer.0 as isize > 32 {
            Ok(())
        } else {
            anyhow::bail!(
                "it was not started with an administrator's rights (Windows' question was answered with no, or: {})",
                std::io::Error::last_os_error()
            )
        }
    };
    match action {
        "start" => {
            anyhow::ensure!(helper.is_file(), "{} is not there", helper.display());
            // (Who it is for: the helper may run as another person, where
            // an administrator's name is given to Windows' question.)
            let person = gevrek_win32::elevated_helper::server::own_user()?;
            as_administrator(
                helper.as_os_str(),
                &format!("--instance \"{instance}\" --for {person}"),
            )?;
            println!("The helper is started. A window manager that runs finds it within a few seconds; it ends when the window manager does.");
        }
        "install" => {
            anyhow::ensure!(helper.is_file(), "{} is not there", helper.display());
            // Done by the helper itself, with an administrator's rights
            // (Windows asks, this once): it puts a copy of itself where
            // only an administrator can write, and registers the task
            // that starts that copy for this person and this window
            // manager. (A task that started the file that lies here, in a
            // folder this person's programs can write to, would let any
            // of them run as an administrator.)
            let person = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
                (Ok(domain), Ok(name)) => format!("{domain}\\{name}"),
                (_, Ok(name)) => name,
                _ => anyhow::bail!("Windows does not say who is signed in"),
            };
            let window_manager = helper.with_file_name("gevrek-wm.exe");
            as_administrator(
                helper.as_os_str(),
                &format!(
                    "--instance \"{instance}\" --wm \"{}\" --install \"{person}\"",
                    window_manager.display()
                ),
            )?;
            let registered = (0..40).any(|_| {
                std::thread::sleep(Duration::from_millis(250));
                gevrek_win32::elevated::helper_task_is_registered(&instance)
            });
            anyhow::ensure!(
                registered,
                "the task \"{task}\" is not there after ten seconds: it was not registered"
            );
            println!(
                "The task \"{task}\" is registered, and the helper kept at {}. From the next start of the window manager on the helper is started by it, without a question. After gevrek is updated, do this again: the helper that is kept is the one of this version.",
                gevrek_win32::elevated::kept_helper().display()
            );
        }
        "remove" => {
            as_administrator(
                helper.as_os_str(),
                &format!("--instance \"{instance}\" --remove"),
            )?;
            // (Until it is gone: what comes next, an uninstall, takes the
            // helper's file away, which Windows holds while it runs.)
            let gone = (0..40).any(|_| {
                std::thread::sleep(Duration::from_millis(250));
                !gevrek_win32::elevated::helper_task_is_registered(&instance)
            });
            anyhow::ensure!(
                gone,
                "the task \"{task}\" is still there after ten seconds: it was not removed"
            );
            std::thread::sleep(Duration::from_millis(500));
            println!("The task \"{task}\" is removed.");
        }
        "status" => {
            let registered = gevrek_win32::elevated::helper_task_is_registered(&instance);
            // (A pipe that is there is in the list of pipes; opening it
            // would take the window manager's place at it.)
            let runs = std::fs::read_dir(r"\\.\pipe\")
                .map(|pipes| {
                    pipes.flatten().any(|entry| {
                        pipe.ends_with(&*entry.file_name().to_string_lossy())
                    })
                })
                .unwrap_or(false);
            println!("helper program: {}{}", helper.display(), if helper.is_file() { "" } else { " (not there)" });
            println!("task \"{task}\": {}", if registered { "registered" } else { "not registered (`gevrek elevated install`)" });
            println!("helper: {}", if runs { "runs" } else { "does not run" });
            println!("windows of programs that run as an administrator are managed while the helper runs and `desktop.platform.windows.manage_elevated_windows` is on.");
        }
        other => anyhow::bail!("`{other}` is none of start, install, remove, status"),
    }
    Ok(())
}

async fn show_health(address: &str) -> Result<()> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let health = client.health_check().await?;

    println!("Window Manager Health:");
    println!("---------------------");
    println!(
        "Status: {}",
        if health.healthy {
            "HEALTHY"
        } else {
            "UNHEALTHY"
        }
    );
    println!("Version: {}", health.version);
    println!("Uptime: {} seconds", health.uptime_seconds);
    if let Some(msg) = health.message {
        println!("Message: {}", msg);
    }
    // What the window manager has to tell (another program on the same
    // keys, ...). An older one says nothing here.
    if let Ok(data) = client.get_overlay_data().await {
        // How often the loop passed in its last second: a frame's rate
        // while something moves, a few times a second when idle.
        if data.event_loop_fps > 0.0 {
            println!("Loop: {:.0} passes a second", data.event_loop_fps);
        }
        // Whether a window that covers a display (a game) reaches it as
        // it draws, with nothing drawn again in between.
        for frames in &data.frames {
            println!(
                "Frames on {}: {} a window's own picture, {} drawn by gevrek; the last one {}",
                frames.output,
                frames.direct,
                frames.composed,
                if frames.direct_now {
                    "a window's own"
                } else {
                    "drawn by gevrek"
                }
            );
        }
        for notice in &data.notices {
            println!("Notice: {}", notice.title);
            println!("  {}", notice.text);
        }
    }
    // On Windows the times after which the screen goes dark and the
    // machine sleeps are the system's, whatever `desktop.session` says:
    // they are read from the power plan in use (and only read).
    #[cfg(windows)]
    if let Some(times) = gevrek_system::idle::read() {
        println!("Idle times (the system's: the Windows power plan in use):");
        for line in times.lines() {
            println!("  {line}");
        }
    }

    Ok(())
}

async fn show_capabilities(address: &str) -> Result<()> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let capabilities = client.get_capabilities().await?;
    println!(
        "gevrek {} · protocol {} · config schema {} · {}",
        capabilities.server_version,
        capabilities.protocol_version,
        capabilities.config_schema_version,
        capabilities.platform
    );
    println!("Features ({}):", capabilities.features.len());
    for feature in &capabilities.features {
        println!("  {feature}");
    }
    println!("Dispatchers ({}):", capabilities.dispatchers.len());
    for dispatcher in &capabilities.dispatchers {
        println!(
            "  {:>2}  {:<32} {}",
            dispatcher.value, dispatcher.id, dispatcher.argument_kind
        );
    }
    println!("Automation nodes: {}", capabilities.node_types.len());
    Ok(())
}

fn validate_complete_config(config: &gevrek_model::Config) -> Result<()> {
    let issues = config.validate();
    for issue in issues
        .iter()
        .filter(|issue| issue.severity == gevrek_model::settings::ConfigIssueSeverity::Warning)
    {
        eprintln!("warning: {}: {}", issue.path, issue.message);
    }
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == gevrek_model::settings::ConfigIssueSeverity::Error)
        .map(|issue| format!("{}: {}", issue.path, issue.message))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "Configuration validation failed:\n{}",
            errors.join("\n")
        ))
    }
}

async fn fetch_live_config(address: &str) -> Result<(WMClient, gevrek_model::Config)> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let document = client.get_configuration().await?;
    let config = gevrek_model::Config::from_json(&document.json)?;
    Ok((client, config))
}

async fn apply_live_config(client: &mut WMClient, config: &gevrek_model::Config) -> Result<()> {
    validate_complete_config(config)?;
    let response = client
        .update_configuration(gevrek_protocol::ConfigurationDocument {
            json: serde_json::to_string_pretty(config)?,
            version: config.version.clone(),
            ..Default::default()
        })
        .await?;
    if response.success {
        Ok(())
    } else {
        Err(anyhow!(
            "Configuration update failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        ))
    }
}

fn upsert_rule(
    config: &mut gevrek_model::Config,
    rule: gevrek_model::settings::WindowRuleProfile,
) -> bool {
    if let Some(existing) = config
        .desktop
        .window_rules
        .iter_mut()
        .find(|existing| existing.name == rule.name)
    {
        *existing = rule;
        true
    } else {
        config.desktop.window_rules.push(rule);
        false
    }
}

fn execution_event_to_json(event: proto::ExecutionEvent) -> Option<(serde_json::Value, bool)> {
    use gevrek_protocol::proto::execution_event::Event;

    match event.event? {
        Event::NodeStarted(event) => Some((
            serde_json::json!({
                "event": "node.started",
                "timestamp_ms": event.timestamp_ms,
                "node_id": event.node_id,
                "node_type": event.node_type().as_str_name(),
            }),
            false,
        )),
        Event::NodeCompleted(event) => Some((
            serde_json::json!({
                "event": "node.completed",
                "timestamp_ms": event.timestamp_ms,
                "node_id": event.node_id,
                "node_type": event.node_type().as_str_name(),
                "duration_ms": event.duration_ms,
                "outputs": event.outputs,
            }),
            false,
        )),
        Event::NodeFailed(event) => Some((
            serde_json::json!({
                "event": "node.failed",
                "timestamp_ms": event.timestamp_ms,
                "node_id": event.node_id,
                "node_type": event.node_type().as_str_name(),
                "error": event.error,
            }),
            true,
        )),
        Event::EventsDropped(event) => Some((
            serde_json::json!({
                "event": "events.dropped",
                "count": event.count,
            }),
            true,
        )),
        Event::ConfigurationChanged(event) => Some((
            serde_json::json!({
                "event": "configuration.changed",
                "timestamp_ms": event.timestamp_ms,
                "revision": event.revision,
                "origin_client_id": event.origin_client_id,
            }),
            true,
        )),
        Event::GraphStarted(event) => Some((
            serde_json::json!({
                "event": "graph.started",
                "timestamp_ms": event.timestamp_ms,
                "trigger": event.trigger_type,
            }),
            false,
        )),
        Event::GraphCompleted(event) => Some((
            serde_json::json!({
                "event": "graph.completed",
                "timestamp_ms": event.timestamp_ms,
                "trigger": event.trigger_type,
                "duration_ms": event.duration_ms,
                "nodes_executed": event.nodes_executed,
            }),
            false,
        )),
        Event::LogEvent(event) => {
            let level = gevrek_protocol::proto::LogLevel::try_from(event.level)
                .unwrap_or(gevrek_protocol::proto::LogLevel::Info);
            let is_error = matches!(
                level,
                gevrek_protocol::proto::LogLevel::Error | gevrek_protocol::proto::LogLevel::Warn
            );
            Some((
                serde_json::json!({
                    "event": "log",
                    "timestamp_ms": event.timestamp_ms,
                    "level": level.as_str_name(),
                    "message": event.message,
                }),
                is_error,
            ))
        }
        Event::WorkingGraphUpdated(event) => Some((
            serde_json::json!({
                "event": "graph.working_updated",
                "timestamp_ms": event.timestamp_ms,
                "editor_id": event.editor_id,
            }),
            false,
        )),
        Event::RuntimeEvent(event) => {
            let is_error = event.kind.ends_with(".failed");
            Some((
                serde_json::json!({
                    "event": event.kind,
                    "timestamp_ms": event.timestamp_ms,
                    "window_handle": event.window_id,
                    "fields": event.fields,
                }),
                is_error,
            ))
        }
    }
}

/// Subscribe to events
async fn subscribe_to_events(address: &str, errors_only: bool, json: bool) -> Result<()> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let mut stream = client.subscribe_to_events().await?;

    if !json {
        println!("Subscribed to window manager events. Press Ctrl+C to stop.");
        println!("-----------------------------------------------------------");
    }

    use gevrek_protocol::proto::execution_event::Event;

    while let Some(event) = stream.message().await? {
        if json {
            if let Some((value, is_error)) = execution_event_to_json(event) {
                if !errors_only || is_error {
                    println!("{}", serde_json::to_string(&value)?);
                }
            }
            continue;
        }
        match event.event {
            Some(Event::NodeStarted(e)) => {
                if !errors_only {
                    println!("[NODE START] {:?} ({})", e.node_type(), e.node_id);
                }
            }
            Some(Event::NodeCompleted(e)) => {
                if !errors_only {
                    println!(
                        "[NODE DONE] {:?} ({}) - {}ms",
                        e.node_type(),
                        e.node_id,
                        e.duration_ms
                    );
                }
            }
            Some(Event::NodeFailed(e)) => {
                println!(
                    "[NODE FAIL] {:?} ({}) - {}",
                    e.node_type(),
                    e.node_id,
                    e.error
                );
            }
            Some(Event::EventsDropped(e)) => {
                println!(
                    "[EVENTS DROPPED] {} events skipped (subscriber fell behind)",
                    e.count
                );
            }
            Some(Event::ConfigurationChanged(e)) => {
                if !errors_only {
                    let origin = if e.origin_client_id.is_empty() {
                        "the WM".to_string()
                    } else {
                        format!("client {}", e.origin_client_id)
                    };
                    println!("[CONFIG] revision {} (changed by {})", e.revision, origin);
                }
            }
            Some(Event::GraphStarted(e)) => {
                if !errors_only {
                    println!("[GRAPH START] {}", e.trigger_type);
                }
            }
            Some(Event::GraphCompleted(e)) => {
                if !errors_only {
                    println!(
                        "[GRAPH DONE] {} - {}ms ({} nodes)",
                        e.trigger_type, e.duration_ms, e.nodes_executed
                    );
                }
            }
            Some(Event::LogEvent(e)) => {
                let level = gevrek_protocol::proto::LogLevel::try_from(e.level)
                    .unwrap_or(gevrek_protocol::proto::LogLevel::Info);
                let is_error = matches!(
                    level,
                    gevrek_protocol::proto::LogLevel::Error
                        | gevrek_protocol::proto::LogLevel::Warn
                );
                if !errors_only || is_error {
                    println!("[{:?}] {}", level, e.message);
                }
            }
            Some(Event::WorkingGraphUpdated(e)) => {
                if !errors_only {
                    println!("[GRAPH UPDATED] by editor {}", e.editor_id);
                }
            }
            Some(Event::RuntimeEvent(e)) => {
                let is_error = e.kind.ends_with(".failed");
                if !errors_only || is_error {
                    let fields =
                        serde_json::to_string(&e.fields).unwrap_or_else(|_| "{}".to_string());
                    if let Some(handle) = e.window_id {
                        println!("[{}] window={} {}", e.kind, handle, fields);
                    } else {
                        println!("[{}] {}", e.kind, fields);
                    }
                }
            }
            None => {}
        }
    }

    Ok(())
}

fn dispatch_request(command: proto::DispatchType) -> proto::DispatchRequest {
    proto::DispatchRequest {
        command: command as i32,
        direction: proto::Direction::Unspecified as i32,
        x: 0,
        y: 0,
        argument: String::new(),
        secondary_argument: String::new(),
    }
}

fn selected_window_request(
    command: proto::DispatchType,
    selector: &str,
) -> Result<proto::DispatchRequest> {
    let selector = selector
        .parse::<WindowSelector>()
        .map_err(anyhow::Error::msg)?;
    let mut request = dispatch_request(command);
    request.argument = selector.expression();
    Ok(request)
}

async fn dispatch_command(
    address: &str,
    request: proto::DispatchRequest,
    success_message: &str,
) -> Result<()> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let response = client.dispatch(request).await?;
    if !response.success {
        return Err(anyhow!(
            "Dispatcher failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    println!("{}", success_message);
    Ok(())
}

/// Send a command as its JSON (every command can go this way).
/// `gevrek shell osd "<words>" [level]` as the command the window manager
/// is given: the words, and a level from 0 to 100 where one is said.
fn osd_command(said: &[String]) -> Result<gevrek_model::command::DispatchCommand> {
    let usage = "gevrek shell osd \"<words>\" [a level from 0 to 100]";
    let (text, level) = match said {
        [text] => (text, None),
        [text, level] => {
            let level: u8 = level
                .parse()
                .ok()
                .filter(|level| *level <= 100)
                .ok_or_else(|| anyhow!("{level} is no level: {usage}"))?;
            (text, Some(level))
        }
        _ => return Err(anyhow!("{usage}")),
    };
    Ok(gevrek_model::command::DispatchCommand::Osd {
        text: text.clone(),
        level,
    })
}

/// `gevrek zones ...`. What is asked and what is printed is made in
/// `gevrek::zones_cli`; this carries it to the window manager and back,
/// and ends with the code that says how it went (`zones_cli::exit`).
async fn zones_command(
    address: &str,
    action: gevrek::zones_cli::ask::ZonesAction,
    json: bool,
    timeout_ms: Option<u32>,
) -> Result<()> {
    if let Err(failure) = zones_run(address, action, json, timeout_ms).await? {
        if json {
            // Said as an answer would say it; the exit code says it too.
            println!("{}", serde_json::to_string(&failure.json())?);
        } else {
            eprintln!("Error: {}", failure.message);
        }
        std::process::exit(failure.code);
    }
    Ok(())
}

/// The section `desktop.zones` of the running configuration, as JSON:
/// the rules have their names there. Null where it cannot be asked for.
async fn zones_section(client: &mut WMClient) -> serde_json::Value {
    let Ok(document) = client.get_configuration().await else {
        return serde_json::Value::Null;
    };
    serde_json::from_str::<serde_json::Value>(&document.json)
        .ok()
        .and_then(|config| config.pointer("/desktop/zones").cloned())
        .unwrap_or_default()
}

/// Send commands to the zones one after the other, until one is not
/// carried out. The answer is one: done if all were, with everything
/// that happened; else the reason of the one that was not, and how many
/// were done before it.
async fn zones_send(
    client: &mut WMClient,
    commands: &[serde_json::Value],
    timeout_ms: u32,
) -> Result<gevrek_protocol::proto::ZonesCommandReply> {
    let mut all = gevrek_protocol::proto::ZonesCommandReply {
        success: true,
        error: None,
        events: Vec::new(),
        delivered: true,
    };
    for (done, command) in commands.iter().enumerate() {
        let reply = client
            .send_zones_command(command.to_string(), gevrek_zones::ipc::VERSION, timeout_ms)
            .await?;
        all.events.extend(reply.events);
        if !reply.success {
            all.success = false;
            all.delivered = reply.delivered;
            all.error = match (reply.error, done) {
                (error, 0) => error,
                (error, done) => Some(format!(
                    "{} ({done} of {} done before it)",
                    error.unwrap_or_else(|| "The zones refused".to_string()),
                    commands.len()
                )),
            };
            break;
        }
    }
    Ok(all)
}

async fn zones_run(
    address: &str,
    action: gevrek::zones_cli::ask::ZonesAction,
    json: bool,
    timeout_ms: Option<u32>,
) -> Result<std::result::Result<(), gevrek::zones_cli::Failure>> {
    use gevrek::zones_cli::ask::{self, ProfileAction, RulesAction, SnapshotAction, ZonesAction};
    use gevrek::zones_cli::{self, exit, Failure};
    use gevrek_protocol::proto::ZonesStateResponse;

    let mut client = WMClient::connect(address.to_string()).await?;
    let wait = timeout_ms.unwrap_or(0);
    // Why there is nothing to read in an answer: no shell (3), or what a
    // shell told that this program cannot read (1).
    let unread = |state: &ZonesStateResponse, why: String| Failure {
        code: if state.running || state.stale {
            exit::REFUSED
        } else {
            exit::NOT_RUNNING
        },
        message: why,
    };
    macro_rules! attempt {
        ($outcome:expr) => {
            match $outcome {
                Ok(value) => value,
                Err(failure) => return Ok(Err(failure)),
            }
        };
    }
    let pretty = |value: &serde_json::Value| serde_json::to_string_pretty(value).unwrap_or_default();

    match &action {
        // ---- what only reads ----
        ZonesAction::List { items } => {
            // The items are left out unless they are asked for: a status
            // bar asks often, and wants the zones.
            let state = client.get_zones_state_of(!(json && *items), "").await?;
            if json {
                println!("{}", pretty(&zones_cli::state_json(&state)));
            } else {
                let text = attempt!(zones_cli::list_text(&state).map_err(|why| unread(&state, why)));
                println!("{text}");
            }
        }
        ZonesAction::Show { zone: Some(text) } => {
            let all = client.get_zones_state_of(true, "").await?;
            let snapshot = attempt!(match zones_cli::snapshot_of(&all) {
                Ok(Some(snapshot)) => Ok(snapshot),
                Ok(None) => Err(unread(&all, all.message.clone())),
                Err(why) => Err(unread(&all, why)),
            });
            let id = attempt!(ask::resolve_zone(&snapshot, text).map_err(Failure::refused))
                .id
                .to_string();
            let one = client.get_zones_state_of(false, &id).await?;
            if json {
                let zone = attempt!(zones_cli::show_json(&one, &id).map_err(|why| unread(&one, why)));
                println!("{}", pretty(&zone));
            } else {
                let text = attempt!(zones_cli::show_text(&one, &id).map_err(|why| unread(&one, why)));
                println!("{text}");
            }
        }
        ZonesAction::Rules {
            action: RulesAction::List,
        } => {
            let section = zones_section(&mut client).await;
            if section.is_null() {
                return Ok(Err(Failure::refused(
                    "The running configuration could not be asked for its rules",
                )));
            }
            if json {
                let mut rules = serde_json::Map::new();
                for key in ["rules", "catch_all", "rules_on_arrival", "timed_rules"] {
                    if let Some(value) = section.get(key) {
                        rules.insert(key.to_string(), value.clone());
                    }
                }
                println!("{}", pretty(&serde_json::Value::Object(rules)));
            } else {
                // (The zones' titles, where a shell tells of them.)
                let state = client.get_zones_state_of(true, "").await?;
                let snapshot = zones_cli::snapshot_of(&state).ok().flatten();
                println!("{}", zones_cli::rules_text(&section, snapshot.as_ref()));
            }
        }
        ZonesAction::Snapshot {
            action: SnapshotAction::List,
        } => {
            let state = client.get_zones_state_of(true, "").await?;
            if json {
                println!("{}", pretty(&zones_cli::snapshots_json(&state)));
            } else {
                let text =
                    attempt!(zones_cli::snapshots_text(&state).map_err(|why| unread(&state, why)));
                println!("{text}");
            }
        }
        ZonesAction::Profile {
            action: action @ (ProfileAction::List | ProfileAction::Show),
        } => {
            let state = client.get_zones_state_of(true, "").await?;
            let all = matches!(action, ProfileAction::List);
            if json {
                println!("{}", pretty(&zones_cli::profiles_json(&state, all)));
            } else {
                let text = attempt!(
                    zones_cli::profiles_text(&state, all).map_err(|why| unread(&state, why))
                );
                println!("{text}");
            }
        }
        ZonesAction::Cancel { request: None } => {
            let state = client.get_zones_state_of(true, "").await?;
            attempt!(zones_cli::zones_for_asking(&state));
            let operations = zones_cli::operations_of(&state);
            match operations.as_slice() {
                // The one there is: called off without being named.
                [only] => {
                    let command = serde_json::json!({
                        "command": "cancel_operation", "request": only["request"],
                    });
                    let reply = zones_send(&mut client, &[command], wait).await?;
                    let failure = zones_cli::failure_of(&reply);
                    if json {
                        println!("{}", serde_json::to_string(&zones_cli::reply_json(&reply))?);
                        if let Some(failure) = failure {
                            std::process::exit(failure.code);
                        }
                    } else {
                        attempt!(failure.map_or(Ok(()), Err));
                        println!("Operation {} was called off", only["request"]);
                    }
                }
                _ if json => println!("{}", pretty(&serde_json::json!({ "operations": operations }))),
                _ => println!("{}", zones_cli::operations_text(&operations)),
            }
        }
        ZonesAction::Events => {
            use gevrek_protocol::proto::execution_event::Event;
            use std::io::Write;
            let mut stream = client.subscribe_to_events().await?;
            let mut out = std::io::stdout();
            // Each line is handed on as it is written: a status bar reads
            // through a pipe, where lines would else wait for a full
            // buffer. A reader that went away ends this quietly.
            let mut say = |line: String| -> bool {
                writeln!(out, "{line}").and_then(|()| out.flush()).is_ok()
            };
            while let Some(event) = stream.message().await? {
                let line = match event.event {
                    Some(Event::RuntimeEvent(event)) => zones_cli::event_line(&event, json),
                    // Some were missed: whoever keeps the zones' state
                    // from the events asks for it anew.
                    Some(Event::EventsDropped(dropped)) => Some(if json {
                        serde_json::json!({"event": "events.dropped", "count": dropped.count})
                            .to_string()
                    } else {
                        format!("[zones] {} events were missed", dropped.count)
                    }),
                    _ => None,
                };
                if line.is_some_and(|line| !say(line)) {
                    break;
                }
            }
        }

        // ---- what is sent as it was written ----
        ZonesAction::Send { command } => {
            let reply = client
                .send_zones_command(command.clone(), gevrek_zones::ipc::VERSION, wait)
                .await?;
            if json {
                println!("{}", serde_json::to_string(&zones_cli::reply_json(&reply))?);
                if let Some(failure) = zones_cli::failure_of(&reply) {
                    // Said in the JSON already; the exit code says it too.
                    std::process::exit(failure.code);
                }
            } else {
                attempt!(zones_cli::failure_of(&reply).map_or(Ok(()), Err));
                println!("{}", zones_cli::reply_text(&reply).unwrap_or_default());
            }
        }

        // ---- every other word: commands made from the zones as they are ----
        _ => {
            // Putting the system's icons back must work when nothing
            // else does, so it asks for no state first.
            let snapshot = if matches!(action, ZonesAction::RestoreExplorer) {
                gevrek_zones::ipc::ZonesSnapshot::default()
            } else {
                // With the items where they are named, or where their
                // names are printed.
                let state = client
                    .get_zones_state_of(!ask::names_items(&action), "")
                    .await?;
                attempt!(zones_cli::zones_for_asking(&state))
            };
            let mut commands = Vec::new();
            let mut then_changes = None;
            let done = match &action {
                ZonesAction::Snapshot {
                    action: SnapshotAction::Import { file, apply },
                } => {
                    let text = attempt!(std::fs::read_to_string(file).map_err(|error| {
                        Failure::refused(format!("{} could not be read: {error}", file.display()))
                    }));
                    commands.push(attempt!(zones_cli::import_command(&text, *apply).map_err(
                        |why| Failure::refused(format!("{} is no layout: {why}", file.display()))
                    )));
                    String::new()
                }
                _ => {
                    let asked = attempt!(ask::asked(&action, &snapshot).map_err(Failure::refused));
                    commands = asked.commands;
                    then_changes = asked.then_changes;
                    asked.done
                }
            };
            let mut reply = zones_send(&mut client, &commands, wait).await?;
            // A zone that was made is given the rest of what was asked
            // for it, now that its id is known.
            if let (true, Some(changes)) = (reply.success, then_changes) {
                if let Some(made) = zones_cli::event_of(&reply, "zone_created") {
                    let change = serde_json::json!({
                        "command": "modify_zone", "zone": made["zone"], "changes": changes,
                    });
                    let changed = zones_send(&mut client, &[change], wait).await?;
                    reply.events.extend(changed.events);
                    if !changed.success {
                        reply.success = false;
                        reply.delivered = changed.delivered;
                        reply.error = Some(format!(
                            "Zone '{}' was made, but not all of it was set: {}",
                            made["zone"].as_str().unwrap_or_default(),
                            changed.error.unwrap_or_default()
                        ));
                    }
                }
            }
            let failure = zones_cli::failure_of(&reply);
            let mut answer = zones_cli::reply_json(&reply);
            let mut said = done;
            if failure.is_none() {
                match &action {
                    ZonesAction::Create { .. } => {
                        if let Some(made) = zones_cli::event_of(&reply, "zone_created") {
                            said = format!("Zone '{}' was made", made["zone"].as_str().unwrap_or_default());
                            answer["zone"] = made["zone"].clone();
                        }
                    }
                    ZonesAction::Rules {
                        action:
                            rules @ (RulesAction::DryRun { .. }
                            | RulesAction::Apply { .. }
                            | RulesAction::Check),
                    } => {
                        let (applied, asked_of) = match rules {
                            RulesAction::DryRun { scope } | RulesAction::Apply { scope } => (
                                !matches!(rules, RulesAction::DryRun { .. }),
                                match scope {
                                    ask::RuleScope::Loose => "the items that are in no zone",
                                    ask::RuleScope::All => "every item of the desktop",
                                },
                            ),
                            _ => (true, "every item, by the rules that go by age"),
                        };
                        let plan = zones_cli::plan_of(&reply);
                        said = zones_cli::plan_text(plan.as_ref(), applied, asked_of);
                        answer["lines"] = plan
                            .map(|plan| plan["lines"].clone())
                            .unwrap_or_else(|| serde_json::json!([]));
                    }
                    ZonesAction::AutoCreate { dry_run: true, scope, .. } => {
                        let planned = zones_cli::event_of(&reply, "auto_planned");
                        let asked_of = match scope {
                            ask::RuleScope::Loose => "the items that are in no zone",
                            ask::RuleScope::All => "every item that is loose or in a zone filled by hand",
                        };
                        said = zones_cli::auto::plan_text(planned.as_ref(), &snapshot, asked_of);
                        answer["plan"] = planned.map_or(serde_json::Value::Null, |event| event["plan"].clone());
                    }
                    ZonesAction::AutoCreate { dry_run: false, .. } => {
                        let made = zones_cli::event_of(&reply, "auto_created");
                        said = zones_cli::auto::created_text(made.as_ref());
                        answer["made"] = made.unwrap_or_default();
                    }
                    ZonesAction::AutoUndo => {
                        let undone = zones_cli::event_of(&reply, "auto_undone");
                        said = zones_cli::auto::undone_text(undone.as_ref());
                        answer["undone"] = undone.unwrap_or_default();
                    }
                    ZonesAction::Search { text } => {
                        let found = zones_cli::event_of(&reply, "found").unwrap_or_else(
                            || serde_json::json!({"typed": text, "hits": []}),
                        );
                        said = zones_cli::search_text(&found, &snapshot);
                        answer["hits"] = found["hits"].clone();
                    }
                    ZonesAction::Snapshot {
                        action: SnapshotAction::Export { name, file },
                    } => {
                        let document = attempt!(zones_cli::exported(&reply).ok_or_else(|| {
                            Failure::refused(format!(
                                "The zones did not hand over the layout '{name}' (this shell may be older)"
                            ))
                        }));
                        attempt!(std::fs::write(file, pretty(&document) + "\n").map_err(|error| {
                            Failure::refused(format!("{} could not be written: {error}", file.display()))
                        }));
                        said = format!("The layout '{name}' was written to {}", file.display());
                        answer["file"] = file.display().to_string().into();
                    }
                    ZonesAction::Snapshot {
                        action: SnapshotAction::Import { .. },
                    } => {
                        let taken = zones_cli::event_of(&reply, "layout_imported").unwrap_or_default();
                        let name = taken["name"].as_str().unwrap_or_default();
                        said = if taken["applied"].as_bool().unwrap_or(false) {
                            match taken["kept_as"].as_str() {
                                Some(kept) => format!(
                                    "The layout '{name}' was taken in and is in force; what there was is kept as '{kept}'"
                                ),
                                None => format!("The layout '{name}' was taken in and is in force"),
                            }
                        } else {
                            format!(
                                "The layout was taken in as '{name}'; `gevrek zones snapshot restore \"{name}\"` puts it in force"
                            )
                        };
                    }
                    _ => {}
                }
            }
            if json {
                println!("{}", serde_json::to_string(&answer)?);
                if let Some(failure) = failure {
                    std::process::exit(failure.code);
                }
            } else {
                attempt!(failure.map_or(Ok(()), Err));
                println!("{said}");
            }
        }
    }
    Ok(Ok(()))
}

async fn run_json_command(
    address: &str,
    command: &gevrek_model::command::DispatchCommand,
) -> Result<()> {
    let mut client = WMClient::connect(address.to_string()).await?;
    let response = client
        .dispatch_command(serde_json::to_string(command)?)
        .await?;
    if !response.success {
        return Err(anyhow!(
            "{} failed: {}",
            command.display_name(),
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    println!("{}", command.display_name());
    Ok(())
}

/// Where the update keeps what it remembers and what it has ready.
fn update_folder() -> PathBuf {
    gevrek_model::paths::state_dir().join("update")
}

/// The folder this program is in, when it is an install the update may
/// replace: on Windows, made from the package.
fn updatable_install() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    gevrek_update::is_packaged(&folder).then_some(folder)
}

/// The note the build that was replaced leaves for the new one: its
/// version, so that the person is told once what happened.
fn updated_from_note() -> PathBuf {
    update_folder().join("updated-from")
}

/// Before `gevrek start` starts anything: if a newer release is ready (the
/// check behind an earlier start got it), it is put in place now, while
/// nothing of gevrek runs, and the new build is started in this one's
/// stead. `None`: there was nothing to do, or it could not be done, and
/// this build starts as always. An update never keeps gevrek from starting.
fn start_updated(components: &[Component], foreground: bool) -> Option<std::process::ExitStatus> {
    if foreground || !components.contains(&Component::Wm) {
        return None;
    }
    let install = updatable_install()?;
    let folder = update_folder();
    if !gevrek_update::Settings::load(&folder).automatic {
        return None;
    }
    let (version, package) = gevrek_update::staged(&folder)?;
    if !gevrek_update::is_newer(&version, gevrek_update::current()) {
        log::debug!(
            "the staged gevrek {version} is not newer than this {}: it is taken away",
            gevrek_update::current()
        );
        gevrek_update::unstage(&folder);
        return None;
    }
    // A desktop in use is not taken away under its windows.
    if Component::all()
        .iter()
        .any(|component| get_component_status(component).0)
    {
        log::debug!("the staged gevrek {version} waits: a part of gevrek runs");
        return None;
    }
    println!(
        "Installing gevrek {version} (this is {})...",
        gevrek_update::current()
    );
    if let Err(error) = gevrek_update::apply(&package, &install) {
        eprintln!("gevrek {version} was not installed: {error:#}");
        return None;
    }
    gevrek_update::unstage(&folder);
    gevrek_update::note_installed_version(&version);
    if let Err(error) = std::fs::write(updated_from_note(), gevrek_update::current()) {
        log::debug!(
            "{} was not written, so the update is not told: {error}",
            updated_from_note().display()
        );
    }
    // This program is the old build still (its file was moved aside): the
    // new one does the starting.
    let new = install.join(format!("gevrek{}", std::env::consts::EXE_SUFFIX));
    log::debug!("gevrek {version} was put in place; {} does the starting", new.display());
    let ended = Command::new(new)
        .args(std::env::args_os().skip(1))
        .status()
        .map_err(|error| eprintln!("The new gevrek could not be started: {error}"))
        .ok();
    if let Some(status) = ended {
        log::debug!("the new gevrek ended: {status}");
    }
    ended
}

/// After `gevrek start` has started the desktop: leave the look for a
/// newer release running behind (`gevrek update --background`), where
/// there is something for it to do.
fn look_for_update_behind(address: &str) {
    if updatable_install().is_none() {
        return;
    }
    let due = gevrek_update::Settings::load(&update_folder()).check_due(gevrek_update::now());
    if !due && !updated_from_note().exists() {
        return;
    }
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            log::debug!("no look for an update: this program is not found: {error}");
            return;
        }
    };
    let mut command = Command::new(program);
    command.args(["--address", address, "update", "--background"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
        keep_own_output_from_children();
    }
    match command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => log::debug!("the look for an update runs behind as process {}", child.id()),
        Err(error) => log::debug!("the look for an update was not started: {error}"),
    }
}

/// A notification of gevrek's own, if a window manager is there to pass it
/// on. Nothing is said here, and nothing is an error.
async fn notify_quietly(address: &str, title: &str, text: &str) {
    let command = gevrek_model::command::DispatchCommand::Notify {
        title: title.to_string(),
        text: text.to_string(),
    };
    match (
        WMClient::connect(address.to_string()).await,
        serde_json::to_string(&command),
    ) {
        (Ok(mut client), Ok(json)) => match client.dispatch_command(json).await {
            Ok(response) if response.success => log::debug!("a notification of gevrek's was shown"),
            Ok(response) => log::debug!(
                "a notification of gevrek's was not shown: {}",
                response.error.unwrap_or_default()
            ),
            Err(error) => log::debug!("a notification of gevrek's was not shown: {error:#}"),
        },
        (Err(error), _) => {
            log::debug!("a notification of gevrek's was not shown: no window manager: {error:#}")
        }
        (_, Err(error)) => log::debug!("a notification of gevrek's was not written: {error}"),
    }
}

/// `gevrek update --background`: say that an update was installed, if one
/// was; and if a day has passed since GitHub was last asked, ask, and get a
/// newer release ready for the next start.
async fn update_behind(address: &str) -> Result<()> {
    let folder = update_folder();
    // The shell that shows the notification has only just been started, and
    // at sign-in the network may not be there yet.
    sleep(Duration::from_secs(20)).await;
    if let Ok(from) = std::fs::read_to_string(updated_from_note()) {
        log::debug!("gevrek was updated from {}: it is told", from.trim());
        if let Err(error) = std::fs::remove_file(updated_from_note()) {
            log::debug!("{} was not removed: {error}", updated_from_note().display());
        }
        notify_quietly(
            address,
            &format!("gevrek {} is installed", gevrek_update::current()),
            &format!(
                "It was {}. That build is kept in the folder \"previous\" beside the programs.",
                from.trim()
            ),
        )
        .await;
    }
    let mut settings = gevrek_update::Settings::load(&folder);
    if updatable_install().is_none() || !settings.check_due(gevrek_update::now()) {
        log::debug!("no look for an update now: it is not due, or this install is not updated");
        return Ok(());
    }
    log::debug!("a newer release is looked for (this is {})", gevrek_update::current());
    // Noted before asking: a check that fails is not made again at every start.
    settings.last_check = Some(gevrek_update::now());
    settings.save(&folder)?;
    let source = gevrek_update::Source::github();
    let release = gevrek_update::latest(&source, &folder)?;
    if !gevrek_update::is_newer(&release.version, gevrek_update::current()) {
        log::debug!("no newer release: the latest is {}", release.version);
        return Ok(());
    }
    if gevrek_update::staged(&folder).is_some_and(|(version, _)| version == release.version) {
        log::debug!("gevrek {} is staged already", release.version);
        return Ok(());
    }
    gevrek_update::stage(&source, &release, &folder)?;
    notify_quietly(
        address,
        &format!("gevrek {} is ready", release.version),
        "It is installed the next time gevrek starts. To install it now: gevrek update",
    )
    .await;
    Ok(())
}

/// `gevrek update` and `gevrek update --check`.
async fn update_now(address: &str, check_only: bool) -> Result<()> {
    let folder = update_folder();
    let source = gevrek_update::Source::github();
    let current = gevrek_update::current();
    let release = gevrek_update::latest(&source, &folder)?;
    let mut settings = gevrek_update::Settings::load(&folder);
    settings.last_check = Some(gevrek_update::now());
    if let Err(error) = settings.save(&folder) {
        log::debug!("when the update was looked for is not kept: {error:#}");
    }
    if !gevrek_update::is_newer(&release.version, current) {
        if release.version == current {
            println!("gevrek {current} is the newest release.");
        } else {
            println!(
                "This is gevrek {current}; the newest release is {}.",
                release.version
            );
        }
        return Ok(());
    }
    println!(
        "gevrek {} is there (this is {current}): {}",
        release.version, release.page
    );
    let Some(install) = updatable_install() else {
        if cfg!(windows) {
            println!("This gevrek was not installed from the package, so this command does not replace it: install the package from that page, or build the newer sources.");
        } else {
            println!("On Linux the package manager updates gevrek: install the newer package from that page.");
        }
        return Ok(());
    };
    if check_only {
        println!("Install it with: gevrek update");
        return Ok(());
    }
    let package = match gevrek_update::staged(&folder) {
        Some((version, package)) if version == release.version => package,
        _ => {
            println!("Downloading and checking it...");
            gevrek_update::stage(&source, &release, &folder)?
        }
    };
    let was_running = get_component_status(&Component::Wm).0;
    if was_running {
        println!("gevrek is running: stopping it for the update.");
        for component in Component::all().iter().rev() {
            stop_component(component, false, address).await?;
        }
        sleep(Duration::from_secs(1)).await;
    }
    let applied = gevrek_update::apply(&package, &install);
    if applied.is_ok() {
        gevrek_update::unstage(&folder);
        gevrek_update::note_installed_version(&release.version);
        println!("Updated: gevrek {current} -> {}", release.version);
        println!(
            "(The build before is in {}.)",
            install.join("previous").display()
        );
    }
    if was_running {
        // By the program that is in place now: the new one, or, if the
        // update could not be put in place, the one that was there.
        println!("Starting gevrek again.");
        match Command::new(install.join(format!("gevrek{}", std::env::consts::EXE_SUFFIX)))
            .args(["--address", address, "start", "all"])
            .status()
        {
            Ok(status) => log::debug!("gevrek start all ended: {status}"),
            Err(error) => log::warn!("gevrek could not be started again after the update: {error}"),
        }
    }
    applied
}

fn main() -> Result<()> {
    // The clap command enum is large enough that building and rendering the
    // parser in a debug build exceeds the default 1 MiB main-thread stack
    // on Windows; run the real entry point on a thread with headroom.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("gevrek main thread panicked")
}

#[tokio::main]
async fn run() -> Result<()> {
    gevrek_logging::Logging::new("gevrek", log::LevelFilter::Warn).only_where_asked().init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start {
            component,
            foreground,
        } => {
            let components = parse_components(&component, true);
            if components.is_empty() {
                return Err(anyhow!(
                    "Unknown component: {}. Use wm, editor, shell, or all",
                    component
                ));
            }

            // A newer release that is ready is installed first, and starts
            // in this build's stead.
            if let Some(status) = start_updated(&components, foreground) {
                std::process::exit(status.code().unwrap_or(1));
            }

            // Start WM first, then others
            let mut wm_started = false;
            for comp in &components {
                if *comp == Component::Wm {
                    if foreground && components.len() == 1 {
                        // If only WM and foreground, wait for it
                        if let Some(mut child) = start_component(comp, true, &cli.address)? {
                            child.wait()?;
                        }
                    } else {
                        start_component(comp, false, &cli.address)?;
                        wm_started = true;
                    }
                }
            }

            // Wait a bit for WM to start before starting other components
            if wm_started {
                wait_for_wm(&cli.address, Duration::from_secs(15)).await?;
            }

            // Start other components
            for comp in &components {
                if *comp != Component::Wm {
                    start_component(comp, foreground && components.len() == 1, &cli.address)?;
                }
            }

            if wm_started {
                look_for_update_behind(&cli.address);
            }
        }

        Commands::Stop { component, force } => {
            let components = parse_components(&component, false);
            if components.is_empty() {
                return Err(anyhow!(
                    "Unknown component: {}. Use wm, editor, shell, or all",
                    component
                ));
            }

            // Stop in reverse order (the shell and the editor first, then the WM)
            let mut reversed = components.clone();
            reversed.reverse();

            for comp in reversed {
                stop_component(&comp, force, &cli.address).await?;
            }
        }

        Commands::Restart { component } => {
            // Both bars are stopped, whichever ran; one is started.
            let stopped = parse_components(&component, false);
            let components = parse_components(&component, true);
            if components.is_empty() {
                return Err(anyhow!(
                    "Unknown component: {}. Use wm, editor, shell, or all",
                    component
                ));
            }

            // Stop all
            let mut reversed = stopped;
            reversed.reverse();
            for comp in &reversed {
                stop_component(comp, false, &cli.address).await?;
            }

            // Wait a bit
            sleep(Duration::from_millis(500)).await;

            // Start all
            for comp in &components {
                start_component(comp, false, &cli.address)?;
                if *comp == Component::Wm {
                    wait_for_wm(&cli.address, Duration::from_secs(15)).await?;
                }
            }
        }

        Commands::Status => {
            show_status();
        }

        Commands::State { json: true } => {
            let mut client = WMClient::connect(cli.address.to_string()).await?;
            let data = client.get_overlay_data().await?;
            println!("{}", serde_json::to_string_pretty(&data)?);
        }
        Commands::State { json: false } => {
            show_state(&cli.address).await?;
        }

        Commands::Health => {
            show_health(&cli.address).await?;
        }

        Commands::Elevated { action } => {
            elevated_helper(&cli.address, &action)?;
        }

        Commands::Capabilities => {
            show_capabilities(&cli.address).await?;
        }

        Commands::Input { step } => {
            use gevrek_host::input_injection::InjectedInput;
            let json = match step {
                InputStep::Steps { json } => json,
                other => {
                    let step = match other {
                        InputStep::Key { chord } => InjectedInput::Chord { chord },
                        InputStep::Keycode { code, hold } => InjectedInput::Keycode { code, hold },
                        InputStep::Down { key } => InjectedInput::Key { key, pressed: true },
                        InputStep::Up { key } => InjectedInput::Key {
                            key,
                            pressed: false,
                        },
                        InputStep::Move { x, y } => InjectedInput::Move { x, y },
                        InputStep::Click { x, y, button } => InjectedInput::Click { x, y, button },
                        InputStep::Drag {
                            from_x,
                            from_y,
                            to_x,
                            to_y,
                            button,
                            hold,
                        } => InjectedInput::Drag {
                            from: (from_x, from_y),
                            to: (to_x, to_y),
                            button,
                            hold,
                        },
                        InputStep::Wheel { direction, hold } => InjectedInput::Wheel {
                            up: match direction.to_ascii_lowercase().as_str() {
                                "up" => true,
                                "down" => false,
                                other => return Err(anyhow!("wheel {other}: up or down")),
                            },
                            hold,
                        },
                        InputStep::Plug { size } => {
                            let (width, height) = size
                                .split_once(['x', 'X'])
                                .and_then(|(width, height)| {
                                    Some((width.parse().ok()?, height.parse().ok()?))
                                })
                                .ok_or_else(|| anyhow!("plug {size}: a size such as 1920x1080"))?;
                            InjectedInput::PlugOutput { width, height }
                        }
                        InputStep::Unplug { name } => InjectedInput::UnplugOutput { name },
                        InputStep::Steps { .. } => unreachable!("handled above"),
                    };
                    serde_json::to_string(&[step])?
                }
            };
            let mut client = WMClient::connect(cli.address.to_string()).await?;
            let response = client.inject_input(json).await?;
            if !response.success {
                return Err(anyhow!(
                    "Input failed: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                ));
            }
        }
        Commands::Quit => {
            run_json_command(&cli.address, &gevrek_model::command::DispatchCommand::Quit).await?;
        }
        // The shell's actions by a name of their own: the same command a
        // key for them gives.
        Commands::Lock => {
            let action = gevrek_model::command::ShellAction::Lock;
            run_json_command(
                &cli.address,
                &gevrek_model::command::DispatchCommand::Shell { action },
            )
            .await?;
        }
        Commands::DisplaysOff => {
            let action = gevrek_model::command::ShellAction::DisplaysOff;
            run_json_command(
                &cli.address,
                &gevrek_model::command::DispatchCommand::Shell { action },
            )
            .await?;
        }
        Commands::OutputTrial { answer } => {
            use gevrek_model::command::TrialAnswer;
            let answer = match answer.as_str() {
                "keep" => TrialAnswer::Keep,
                "revert" => TrialAnswer::Revert,
                other => return Err(anyhow::anyhow!("{other}: keep or revert")),
            };
            run_json_command(
                &cli.address,
                &gevrek_model::command::DispatchCommand::OutputTrial { answer },
            )
            .await?;
        }
        Commands::Notify { title, text } => {
            run_json_command(
                &cli.address,
                &gevrek_model::command::DispatchCommand::Notify { title, text },
            )
            .await?;
        }
        Commands::Tutorial { what } => {
            use gevrek_model::command::ShellAction;
            let action = match what.as_deref() {
                None | Some("begin") => ShellAction::Tutorial,
                Some("next") => ShellAction::TutorialNext,
                Some("back") => ShellAction::TutorialBack,
                Some("leave") => ShellAction::TutorialLeave,
                Some("welcome") => ShellAction::Welcome,
                Some(other) => {
                    return Err(anyhow!(
                        "gevrek tutorial {other}: there are leave, next, back and welcome"
                    ))
                }
            };
            run_json_command(
                &cli.address,
                &gevrek_model::command::DispatchCommand::Shell { action },
            )
            .await?;
        }
        Commands::Shell { action, rest } => {
            use gevrek_model::command::ShellAction;
            if action == "list" {
                for action in ShellAction::ALL {
                    println!("{:<22} {}", action.id(), action.display_name());
                }
                println!(
                    "{:<22} Show words on the screen: osd \"<words>\" [0-100]",
                    "osd"
                );
            } else if action == "osd" {
                run_json_command(&cli.address, &osd_command(&rest)?).await?;
            } else {
                let action = ShellAction::ALL
                    .into_iter()
                    .find(|candidate| candidate.id() == action)
                    .ok_or_else(|| anyhow!("unknown shell action {action} (gevrek shell list)"))?;
                run_json_command(
                    &cli.address,
                    &gevrek_model::command::DispatchCommand::Shell { action },
                )
                .await?;
            }
        }
        Commands::KeyboardLayout { target } => {
            use gevrek_model::command::LayoutTarget;
            let target = match target.as_str() {
                "next" => LayoutTarget::Next,
                "previous" => LayoutTarget::Previous,
                number => LayoutTarget::Index(number.parse().map_err(|_| {
                    anyhow!("keyboard-layout {number}: next, previous or a number")
                })?),
            };
            run_json_command(
                &cli.address,
                &gevrek_model::command::DispatchCommand::KeyboardLayout { target },
            )
            .await?;
        }
        Commands::ConfigPath => {
            let (path, _) = gevrek_model::paths::resolve_config(None);
            println!("{}", path.display());
        }
        Commands::Schema { settings } => {
            if settings {
                for setting in gevrek_schema::catalogue() {
                    let description = setting.description.lines().next().unwrap_or("");
                    println!(
                        "{:<58} {:<8} {}",
                        setting.pointer, setting.kind, description
                    );
                }
            } else {
                let text = serde_json::to_string_pretty(gevrek_schema::schema().document())?;
                println!("{text}");
            }
        }
        Commands::Dispatch { json } => {
            let mut client = WMClient::connect(cli.address.to_string()).await?;
            let response = client.dispatch_command(json).await?;
            if !response.success {
                return Err(anyhow!(
                    "Dispatcher failed: {}",
                    response
                        .error
                        .unwrap_or_else(|| "unknown error".to_string())
                ));
            }
            println!("Done");
        }
        Commands::Batch {
            file,
            stop_on_error,
        } => {
            let json = std::fs::read_to_string(file)?;
            let commands: Vec<gevrek_model::command::DispatchCommand> =
                serde_json::from_str(&json)?;
            if commands.is_empty() {
                return Err(anyhow!("Batch file contains no commands"));
            }
            let requests = commands
                .into_iter()
                .map(gevrek::conversions::dispatch_command_to_proto)
                .collect();
            let mut client = WMClient::connect(cli.address).await?;
            let response = client.dispatch_batch(requests, stop_on_error).await?;
            for result in &response.results {
                println!(
                    "[{}] {}{}",
                    result.index,
                    if result.success {
                        "completed"
                    } else if result.skipped {
                        "skipped"
                    } else {
                        "failed"
                    },
                    result
                        .error
                        .as_ref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                );
            }
            if !response.success {
                return Err(anyhow!("One or more batch commands did not complete"));
            }
        }

        Commands::Pause => {
            let mut client = WMClient::connect(cli.address).await?;
            let response = client.pause_execution().await?;
            if response.success {
                println!("Execution paused");
            } else {
                return Err(anyhow!(
                    "Failed to pause: {}",
                    response.error.unwrap_or_default()
                ));
            }
        }

        Commands::Resume => {
            let mut client = WMClient::connect(cli.address).await?;
            let response = client.resume_execution().await?;
            if response.success {
                println!("Execution resumed");
            } else {
                return Err(anyhow!(
                    "Failed to resume: {}",
                    response.error.unwrap_or_default()
                ));
            }
        }

        Commands::Push { file } => {
            let content = std::fs::read_to_string(&file)?;
            let mut client = WMClient::connect(cli.address).await?;
            // A graph file is told by what is in it, not by whether it
            // reads as a configuration: a configuration keeps the keys it
            // does not know, so a bare graph reads as one too (an empty
            // one, which would then replace the running configuration).
            let as_config = if is_bare_graph(&content) {
                Err(anyhow!("the file is a graph, not a configuration"))
            } else {
                gevrek_model::Config::from_json(&content).map_err(anyhow::Error::from)
            };
            match as_config {
                Ok(config) => {
                    // The same check the WM runs, before sending anything.
                    let cleanup = config.clone().normalize().map_err(|issue| {
                        anyhow!("Invalid configuration at {}: {}", issue.path, issue.message)
                    })?;
                    if !cleanup.is_clean() {
                        return Err(anyhow!(
                            "Configuration contains invalid graph data ({} nodes and {} connections)",
                            cleanup.removed_nodes,
                            cleanup.removed_connections
                        ));
                    }
                    let response = client
                        .update_configuration(gevrek_protocol::ConfigurationDocument {
                            json: content,
                            version: config.version,
                            ..Default::default()
                        })
                        .await?;
                    if !response.success {
                        return Err(anyhow!(
                            "Failed to push configuration: {}",
                            response.error.unwrap_or_default()
                        ));
                    }
                    println!("Configuration validated and applied successfully");
                }
                Err(config_error) => {
                    let graph = serde_json::from_str::<gevrek_model::graph::NodeGraph>(&content)
                        .map_err(|graph_error| {
                            anyhow!(
                                "File is neither a Config nor a NodeGraph (config: {}; graph: {})",
                                config_error,
                                graph_error
                            )
                        })?;
                    let response = client
                        .push_graph(gevrek::conversions::node_graph_to_proto(&graph))
                        .await?;
                    if !response.success {
                        return Err(anyhow!(
                            "Failed to push graph: {}",
                            response.error.unwrap_or_default()
                        ));
                    }
                    println!(
                        "Graph pushed successfully: {} nodes, {} connections",
                        response.node_count, response.connection_count
                    );
                }
            }
        }

        Commands::ConfigPull { file } => {
            let mut client = WMClient::connect(cli.address).await?;
            let document = client.get_configuration().await?;
            std::fs::write(&file, document.json)?;
            println!("Live configuration exported to {}", file.display());
        }

        Commands::ConfigValidate { file } => {
            let json = std::fs::read_to_string(&file)?;
            let config = gevrek_model::Config::from_json(&json)?;
            validate_complete_config(&config)?;
            println!(
                "Configuration is valid (schema {}, {} rules, {} workspaces)",
                config.desktop.schema_version,
                config.desktop.window_rules.len(),
                config.desktop.workspaces.len()
            );
        }

        Commands::Example { id, apply, out } => {
            let examples = gevrek_model::examples::built_in_examples();
            let Some(id) = id else {
                for example in &examples {
                    println!("{:<22} {}", example.id, example.name);
                    println!("{:<22} {}", "", example.description);
                }
                return Ok(());
            };
            let example = examples
                .into_iter()
                .find(|example| example.id.eq_ignore_ascii_case(&id))
                .ok_or_else(|| {
                    anyhow::anyhow!("There is no example '{id}' (see `gevrek example`)")
                })?;
            if let Some(file) = &out {
                std::fs::write(file, serde_json::to_string_pretty(&example.config)?)?;
                println!("Example '{}' written to {}", example.name, file.display());
            }
            if apply {
                let mut client = WMClient::connect(cli.address).await?;
                apply_live_config(&mut client, &example.config).await?;
                println!(
                    "Example '{}' is the running configuration now",
                    example.name
                );
            } else if out.is_none() {
                println!("{}", serde_json::to_string_pretty(&example.config)?);
            }
        }

        Commands::RuleList { json } => {
            let (_client, config) = fetch_live_config(&cli.address).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&config.desktop.window_rules)?
                );
            } else if config.desktop.window_rules.is_empty() {
                println!("No persistent window rules are configured");
            } else {
                for (index, rule) in config.desktop.window_rules.iter().enumerate() {
                    println!(
                        "{:>2}. {} · {} · {} · {} predicates · {} effects",
                        index + 1,
                        rule.name,
                        if rule.enabled { "enabled" } else { "disabled" },
                        if rule.dynamic { "dynamic" } else { "initial" },
                        rule.predicates.len(),
                        rule.effects.len(),
                    );
                }
            }
        }

        Commands::RuleUpsert { file } => {
            let json = std::fs::read_to_string(&file)?;
            let rule = serde_json::from_str::<gevrek_model::settings::WindowRuleProfile>(&json)?;
            let name = rule.name.clone();
            let (mut client, mut config) = fetch_live_config(&cli.address).await?;
            let replaced = upsert_rule(&mut config, rule);
            apply_live_config(&mut client, &config).await?;
            println!(
                "{} persistent rule '{}'",
                if replaced { "Updated" } else { "Added" },
                name
            );
        }

        Commands::Wallpaper { json, action: None } => {
            let (mut client, config) = fetch_live_config(&cli.address).await?;
            let section = &config.desktop.wallpaper;
            let state = serde_json::to_value(client.get_overlay_data().await?)?;
            let displays = wallpaper_displays(&state);
            // On Windows the system draws the wallpaper: what it shows
            // now is told too (the configuration's, once it was given).
            #[cfg(windows)]
            let system = gevrek_win32::wallpaper::shown_now();
            #[cfg(not(windows))]
            let system: Option<serde_json::Value> = None;
            if json {
                let mut told = serde_json::json!({
                    "wallpaper": section,
                    "displays": displays,
                });
                if let Some(system) = &system {
                    told["system"] = system.clone();
                }
                println!("{}", serde_json::to_string_pretty(&told)?);
            } else {
                if let Some(system) = &system {
                    println!(
                        "Windows' background: a {}",
                        system["kind"].as_str().unwrap_or_default()
                    );
                    for display in system["displays"].as_array().into_iter().flatten() {
                        println!(
                            "  {}",
                            display["picture"]
                                .as_str()
                                .filter(|picture| !picture.is_empty())
                                .unwrap_or("no picture")
                        );
                    }
                }
                let picture = |image: &str| {
                    if image.is_empty() {
                        "the colour only".to_string()
                    } else {
                        image.to_string()
                    }
                };
                let fit = |fit: gevrek_model::wallpaper::WallpaperFit| {
                    serde_json::to_value(fit)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_default()
                };
                println!(
                    "Wallpaper: {} ({})",
                    picture(&section.image),
                    fit(section.fit)
                );
                let (red, green, blue) = section.color;
                println!("Colour: {red}, {green}, {blue}");
                for display in &displays {
                    let name = display["name"].as_str().unwrap_or_default();
                    let own = section
                        .outputs
                        .iter()
                        .find(|entry| entry.name.eq_ignore_ascii_case(name));
                    println!(
                        "  {name} ({}x{} at {}, {}): {}",
                        display["width"],
                        display["height"],
                        display["x"],
                        display["y"],
                        match own {
                            Some(entry) =>
                                format!("{} ({})", picture(&entry.image), fit(entry.fit)),
                            None => "the common one".to_string(),
                        }
                    );
                }
                // What is set for a display that is not connected.
                for entry in section.outputs.iter().filter(|entry| {
                    !displays.iter().any(|display| {
                        display["name"]
                            .as_str()
                            .is_some_and(|name| name.eq_ignore_ascii_case(&entry.name))
                    })
                }) {
                    println!(
                        "  {} (not connected): {} ({})",
                        entry.name,
                        picture(&entry.image),
                        fit(entry.fit)
                    );
                }
            }
        }

        Commands::Wallpaper {
            action: Some(action),
            ..
        } => {
            // What is to change is read before anything is asked of the
            // window manager.
            enum Change {
                Set(
                    String,
                    Option<String>,
                    Option<gevrek_model::wallpaper::WallpaperFit>,
                ),
                Clear(Option<String>),
                Apply(gevrek_model::wallpaper::WallpaperConfig),
            }
            let change = match action {
                WallpaperAction::Set { file, output, fit } => {
                    let fit = fit.as_deref().map(wallpaper_fit).transpose()?;
                    if !file.is_file() {
                        return Err(anyhow!("{}: no such file", file.display()));
                    }
                    let file = std::fs::canonicalize(&file)?;
                    Change::Set(file.to_string_lossy().into_owned(), output, fit)
                }
                WallpaperAction::Clear { output } => Change::Clear(output),
                WallpaperAction::Apply { file } => {
                    let text = if file.as_os_str() == "-" {
                        std::io::read_to_string(std::io::stdin())?
                    } else {
                        std::fs::read_to_string(&file)
                            .map_err(|error| anyhow!("{}: {error}", file.display()))?
                    };
                    Change::Apply(wallpaper_from_json(&text)?)
                }
            };
            let (mut client, mut config) = fetch_live_config(&cli.address).await?;
            let section = &mut config.desktop.wallpaper;
            let said = match change {
                Change::Set(image, output, fit) => {
                    wallpaper_set(section, image.clone(), output.as_deref(), fit);
                    match output {
                        Some(output) => format!("Wallpaper of {output}: {image}"),
                        None => format!("Wallpaper: {image}"),
                    }
                }
                Change::Clear(output) => {
                    if !wallpaper_clear(section, output.as_deref()) {
                        match output {
                            Some(output) => {
                                println!("{output} has no wallpaper of its own");
                            }
                            None => println!("No wallpaper is set"),
                        }
                        return Ok(());
                    }
                    match output {
                        Some(output) => format!("{output} shows the common wallpaper again"),
                        None => "Wallpaper cleared".to_string(),
                    }
                }
                Change::Apply(new) => {
                    *section = new;
                    "Wallpaper applied".to_string()
                }
            };
            apply_live_config(&mut client, &config).await?;
            println!("{said}");
        }

        Commands::RuleRemove { name } => {
            let (mut client, mut config) = fetch_live_config(&cli.address).await?;
            let before = config.desktop.window_rules.len();
            config.desktop.window_rules.retain(|rule| rule.name != name);
            if config.desktop.window_rules.len() == before {
                return Err(anyhow!("No persistent rule named '{}' exists", name));
            }
            apply_live_config(&mut client, &config).await?;
            println!("Removed persistent rule '{}'", name);
        }

        Commands::Log { errors_only, json } => {
            subscribe_to_events(&cli.address, errors_only, json).await?;
        }

        Commands::FocusNext => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::FocusNext),
                "Focused next window",
            )
            .await?;
        }

        Commands::FocusPrevious => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::FocusPrevious),
                "Focused previous window",
            )
            .await?;
        }

        Commands::FocusLast => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::FocusLast),
                "Focused last window",
            )
            .await?;
        }

        Commands::FocusWindow { selector } => {
            let request = selected_window_request(proto::DispatchType::FocusWindow, &selector)?;
            dispatch_command(
                &cli.address,
                request,
                &format!("Focused window matching {selector}"),
            )
            .await?;
        }

        Commands::ForceFocus { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ForceFocusWindow, &selector)?;
            dispatch_command(
                &cli.address,
                request,
                &format!("Force-focused window matching {selector} (rule override)"),
            )
            .await?;
        }

        Commands::CloseWindow { selector } => {
            let request = selected_window_request(proto::DispatchType::CloseWindow, &selector)?;
            dispatch_command(&cli.address, request, "Closed selected window").await?;
        }

        Commands::ToggleFullscreenWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowFullscreen, &selector)?;
            dispatch_command(&cli.address, request, "Toggled selected window fullscreen").await?;
        }

        Commands::TogglePseudotileWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowPseudotile, &selector)?;
            dispatch_command(&cli.address, request, "Toggled selected window pseudotile").await?;
        }

        Commands::TagAddWindow { selector, tag } => {
            let mut request =
                selected_window_request(proto::DispatchType::AddSelectedWindowTag, &selector)?;
            request.secondary_argument = tag;
            dispatch_command(&cli.address, request, "Added tag to selected window").await?;
        }

        Commands::TagRemoveWindow { selector, tag } => {
            let mut request =
                selected_window_request(proto::DispatchType::RemoveSelectedWindowTag, &selector)?;
            request.secondary_argument = tag;
            dispatch_command(&cli.address, request, "Removed tag from selected window").await?;
        }

        Commands::TagToggleWindow { selector, tag } => {
            let mut request =
                selected_window_request(proto::DispatchType::ToggleSelectedWindowTag, &selector)?;
            request.secondary_argument = tag;
            dispatch_command(&cli.address, request, "Toggled selected window tag").await?;
        }

        Commands::TagClearWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ClearSelectedWindowTags, &selector)?;
            dispatch_command(&cli.address, request, "Cleared selected window tags").await?;
        }

        Commands::MoveWindowTo { window, workspace } => {
            let mut request =
                selected_window_request(proto::DispatchType::MoveSelectedToWorkspace, &window)?;
            request.secondary_argument = workspace
                .parse::<WorkspaceSelector>()
                .map_err(anyhow::Error::msg)?
                .expression();
            dispatch_command(&cli.address, request, "Moved selected window silently").await?;
        }

        Commands::MoveWindowToAndFollow { window, workspace } => {
            let mut request = selected_window_request(
                proto::DispatchType::MoveSelectedToWorkspaceAndFollow,
                &window,
            )?;
            request.secondary_argument = workspace
                .parse::<WorkspaceSelector>()
                .map_err(anyhow::Error::msg)?
                .expression();
            dispatch_command(
                &cli.address,
                request,
                "Moved selected window and followed it",
            )
            .await?;
        }

        Commands::ToggleFloatWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowFloating, &selector)?;
            dispatch_command(&cli.address, request, "Toggled selected window floating").await?;
        }

        Commands::FloatWindow { selector } => {
            let mut request =
                selected_window_request(proto::DispatchType::SetWindowFloating, &selector)?;
            request.x = 1;
            dispatch_command(&cli.address, request, "Selected window is floating").await?;
        }

        Commands::TileWindow { selector } => {
            let mut request =
                selected_window_request(proto::DispatchType::SetWindowFloating, &selector)?;
            request.x = 0;
            dispatch_command(&cli.address, request, "Selected window is tiled").await?;
        }

        Commands::TogglePinWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowPinned, &selector)?;
            dispatch_command(&cli.address, request, "Toggled selected window pin").await?;
        }

        Commands::PinWindow { selector } => {
            let mut request =
                selected_window_request(proto::DispatchType::SetWindowPinned, &selector)?;
            request.x = 1;
            dispatch_command(&cli.address, request, "Pinned selected window").await?;
        }

        Commands::UnpinWindow { selector } => {
            let mut request =
                selected_window_request(proto::DispatchType::SetWindowPinned, &selector)?;
            request.x = 0;
            dispatch_command(&cli.address, request, "Unpinned selected window").await?;
        }

        Commands::CenterWindow { selector } => {
            let request = selected_window_request(proto::DispatchType::CenterWindow, &selector)?;
            dispatch_command(&cli.address, request, "Centered selected window").await?;
        }

        Commands::RaiseWindow { selector } => {
            let request = selected_window_request(proto::DispatchType::RaiseWindow, &selector)?;
            dispatch_command(&cli.address, request, "Raised selected window").await?;
        }

        Commands::LowerWindow { selector } => {
            let request = selected_window_request(proto::DispatchType::LowerWindow, &selector)?;
            dispatch_command(&cli.address, request, "Lowered selected window").await?;
        }

        Commands::MoveWindow {
            selector,
            x,
            y,
            relative,
        } => {
            let mut request = selected_window_request(proto::DispatchType::MoveWindow, &selector)?;
            request.x = x;
            request.y = y;
            request.secondary_argument = if relative { "relative" } else { "absolute" }.to_string();
            dispatch_command(&cli.address, request, "Moved selected window").await?;
        }

        Commands::ResizeWindow {
            selector,
            width,
            height,
            relative,
        } => {
            let mut request =
                selected_window_request(proto::DispatchType::ResizeWindow, &selector)?;
            request.x = width;
            request.y = height;
            request.secondary_argument = if relative { "relative" } else { "absolute" }.to_string();
            dispatch_command(&cli.address, request, "Resized selected window").await?;
        }

        Commands::MoveToNext => {
            let mut request = dispatch_request(proto::DispatchType::MoveToWorkspace);
            request.argument = WorkspaceSelector::Next.expression();
            dispatch_command(
                &cli.address,
                request,
                "Moved active window to next workspace",
            )
            .await?;
        }

        Commands::MoveToPrevious => {
            let mut request = dispatch_request(proto::DispatchType::MoveToWorkspace);
            request.argument = WorkspaceSelector::Previous.expression();
            dispatch_command(
                &cli.address,
                request,
                "Moved active window to previous workspace",
            )
            .await?;
        }

        Commands::Workspace { selector } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::SwitchWorkspace);
            request.argument = selector.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!("Focused workspace {}", selector.expression()),
            )
            .await?;
        }

        Commands::FocusMonitor { selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::FocusMonitor);
            request.argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!("Focused monitor {}", target.expression()),
            )
            .await?;
        }

        Commands::FocusMonitorAtCursor => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::FocusMonitorAtCursor),
                "Focused the monitor under the cursor",
            )
            .await?;
        }

        Commands::FocusMonitorWorkspace { monitor, workspace } => {
            let target = monitor
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let workspace = workspace
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::FocusMonitorWorkspace);
            request.argument = target.expression();
            request.secondary_argument = workspace.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Focused workspace {} on monitor {}",
                    workspace.expression(),
                    target.expression()
                ),
            )
            .await?;
        }

        Commands::MoveWorkspaceToMonitor { selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::MoveWorkspaceToMonitor);
            request.argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!("Moved active workspace to monitor {}", target.expression()),
            )
            .await?;
        }

        Commands::SwapActiveWorkspaces { selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::SwapActiveWorkspaces);
            request.argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Swapped active workspaces with monitor {}",
                    target.expression()
                ),
            )
            .await?;
        }

        Commands::MoveToMonitor { selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::MoveWindowToMonitorAndFollow);
            request.argument = WindowSelector::Active.expression();
            request.secondary_argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Moved active window to monitor {} and followed",
                    target.expression()
                ),
            )
            .await?;
        }

        Commands::SendToMonitor { selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::MoveWindowToMonitor);
            request.argument = WindowSelector::Active.expression();
            request.secondary_argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Sent active window to monitor {} silently",
                    target.expression()
                ),
            )
            .await?;
        }

        Commands::MoveWindowToMonitor { window, selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request =
                selected_window_request(proto::DispatchType::MoveWindowToMonitor, &window)?;
            request.secondary_argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Sent selected window to monitor {} silently",
                    target.expression()
                ),
            )
            .await?;
        }

        Commands::MoveWindowToMonitorAndFollow { window, selector } => {
            let target = selector
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            let mut request = selected_window_request(
                proto::DispatchType::MoveWindowToMonitorAndFollow,
                &window,
            )?;
            request.secondary_argument = target.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Moved selected window to monitor {} and followed",
                    target.expression()
                ),
            )
            .await?;
        }

        Commands::Retile => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::Retile),
                "Retiled the active workspace on every monitor",
            )
            .await?;
        }

        Commands::ToggleTiling => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleTiling),
                "Toggled global tiling",
            )
            .await?;
        }

        Commands::ToggleWorkspaceTiling { selector } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::ToggleWorkspaceTiling);
            request.argument = selector.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!("Toggled tiling for workspace {}", selector.expression()),
            )
            .await?;
        }

        Commands::ToggleWorkspaceFloatOverride { selector } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::ToggleWorkspaceFloatOverride);
            request.argument = selector.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Toggled float override for workspace {}",
                    selector.expression()
                ),
            )
            .await?;
        }

        Commands::NewWorkspace { name } => {
            let mut request = dispatch_request(proto::DispatchType::NewWorkspace);
            if let Some(name) = &name {
                if name.trim().is_empty() {
                    return Err(anyhow!("Workspace name cannot be empty"));
                }
                request.argument = name.trim().to_string();
            }
            dispatch_command(&cli.address, request, "Created new workspace").await?;
        }

        Commands::CloseWorkspace => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CloseWorkspace),
                "Closed focused workspace",
            )
            .await?;
        }

        Commands::EnsureWorkspaces { monitor, count } => {
            let monitor = monitor
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            if !(1..=128).contains(&count) {
                return Err(anyhow!("Workspace count must be between 1 and 128"));
            }
            let mut request = dispatch_request(proto::DispatchType::EnsureWorkspaces);
            request.argument = monitor.expression();
            request.x = i32::try_from(count)
                .map_err(|_| anyhow!("Workspace count is too large for the protocol"))?;
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Ensured {} workspace(s) on monitor {}",
                    count,
                    monitor.expression()
                ),
            )
            .await?;
        }

        Commands::EnsureNamedWorkspace { monitor, name } => {
            let monitor = monitor
                .parse::<MonitorTarget>()
                .map_err(|message| anyhow!(message))?;
            if name.trim().is_empty() {
                return Err(anyhow!("Workspace name cannot be empty"));
            }
            let mut request = dispatch_request(proto::DispatchType::EnsureNamedWorkspace);
            request.argument = monitor.expression();
            request.secondary_argument = name.trim().to_string();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Ensured workspace '{}' exists on monitor {}",
                    name.trim(),
                    monitor.expression()
                ),
            )
            .await?;
        }

        Commands::ToggleWindowContainerBehaviour => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleWindowContainerBehaviour),
                "Toggled window container behaviour",
            )
            .await?;
        }

        Commands::ToggleWorkspaceWindowContainerBehaviour { selector } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request =
                dispatch_request(proto::DispatchType::ToggleWorkspaceWindowContainerBehaviour);
            request.argument = selector.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Toggled container behaviour for workspace {}",
                    selector.expression()
                ),
            )
            .await?;
        }

        Commands::ToggleTitleBar => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleTitleBar),
                "Toggled title bar for active window",
            )
            .await?;
        }

        Commands::ToggleWindowTitleBar { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowTitleBar, &selector)?;
            dispatch_command(
                &cli.address,
                request,
                "Toggled title bar for selected window",
            )
            .await?;
        }

        Commands::Manage { selector } => {
            let request = selected_window_request(proto::DispatchType::ManageWindow, &selector)?;
            dispatch_command(&cli.address, request, "Managing selected window").await?;
        }

        Commands::Unmanage { selector } => {
            let request = selected_window_request(proto::DispatchType::UnmanageWindow, &selector)?;
            dispatch_command(&cli.address, request, "Unmanaged selected window").await?;
        }

        Commands::RestoreWindows => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::RestoreWindows),
                "Restored hidden windows",
            )
            .await?;
        }

        Commands::ToggleMouseFollowsFocus => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleMouseFollowsFocus),
                "Toggled mouse-follows-focus",
            )
            .await?;
        }

        Commands::ToggleFloat => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleFloating),
                "Toggled floating mode for active window",
            )
            .await?;
        }

        Commands::Float => {
            let mut request = dispatch_request(proto::DispatchType::SetFloating);
            request.x = 1;
            dispatch_command(&cli.address, request, "Floated active window").await?;
        }

        Commands::Tile => {
            let mut request = dispatch_request(proto::DispatchType::SetFloating);
            request.x = 0;
            dispatch_command(&cli.address, request, "Tiled active window").await?;
        }

        Commands::FloatingAside => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleFloatingAside),
                "Floating windows put aside or brought back",
            )
            .await?;
        }

        Commands::ShowDesktop { switch } => {
            let mut request = dispatch_request(proto::DispatchType::ShowDesktop);
            request.argument = switch.argument().to_string();
            let done = match switch {
                CliSwitch::On => "The desktop is shown",
                CliSwitch::Off => "The windows are back",
                CliSwitch::Toggle => "The desktop is shown, or the windows are back",
            };
            dispatch_command(&cli.address, request, done).await?;
        }

        Commands::Center => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CenterActive),
                "Centered active window",
            )
            .await?;
        }

        Commands::Raise => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::RaiseActive),
                "Raised active window",
            )
            .await?;
        }

        Commands::Lower => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::LowerActive),
                "Lowered active window",
            )
            .await?;
        }

        Commands::TagAdd { tag } => {
            let mut request = dispatch_request(proto::DispatchType::AddWindowTag);
            request.argument = tag;
            dispatch_command(&cli.address, request, "Added window tag").await?;
        }

        Commands::TagRemove { tag } => {
            let mut request = dispatch_request(proto::DispatchType::RemoveWindowTag);
            request.argument = tag;
            dispatch_command(&cli.address, request, "Removed window tag").await?;
        }

        Commands::TagToggle { tag } => {
            let mut request = dispatch_request(proto::DispatchType::ToggleWindowTag);
            request.argument = tag;
            dispatch_command(&cli.address, request, "Toggled window tag").await?;
        }

        Commands::TagClear => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ClearWindowTags),
                "Cleared manual window tags",
            )
            .await?;
        }

        Commands::RuleEnable { name } => {
            let mut request = dispatch_request(proto::DispatchType::EnableRule);
            request.argument = name;
            dispatch_command(&cli.address, request, "Enabled rule").await?;
        }

        Commands::RuleDisable { name } => {
            let mut request = dispatch_request(proto::DispatchType::DisableRule);
            request.argument = name;
            dispatch_command(&cli.address, request, "Disabled rule").await?;
        }

        Commands::RuleToggle { name } => {
            let mut request = dispatch_request(proto::DispatchType::ToggleRule);
            request.argument = name;
            dispatch_command(&cli.address, request, "Toggled rule").await?;
        }

        Commands::Focus { direction } => {
            let mut request = dispatch_request(proto::DispatchType::FocusDirection);
            request.direction = proto::Direction::from(direction) as i32;
            dispatch_command(&cli.address, request, "Focused directional neighbor").await?;
        }

        Commands::Swap { direction } => {
            let mut request = dispatch_request(proto::DispatchType::SwapDirection);
            request.direction = proto::Direction::from(direction) as i32;
            dispatch_command(&cli.address, request, "Swapped directional neighbor").await?;
        }

        Commands::MoveActive { x, y } => {
            let mut request = dispatch_request(proto::DispatchType::MoveActive);
            request.x = x;
            request.y = y;
            dispatch_command(&cli.address, request, "Moved active window").await?;
        }

        Commands::ResizeActive { width, height } => {
            let mut request = dispatch_request(proto::DispatchType::ResizeActive);
            request.x = width;
            request.y = height;
            dispatch_command(&cli.address, request, "Resized active window").await?;
        }

        Commands::Close => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CloseActive),
                "Closed active window",
            )
            .await?;
        }

        Commands::Minimize => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::MinimizeActive),
                "Minimized active window",
            )
            .await?;
        }

        Commands::ToggleMaximize => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleMaximized),
                "Toggled maximized state",
            )
            .await?;
        }

        Commands::ToggleFullscreen => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleFullscreen),
                "Toggled fullscreen",
            )
            .await?;
        }

        Commands::TogglePin => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::TogglePinned),
                "Toggled always-on-top",
            )
            .await?;
        }

        Commands::Pin => {
            let mut request = dispatch_request(proto::DispatchType::SetPinned);
            request.x = 1;
            dispatch_command(&cli.address, request, "Pinned active window").await?;
        }

        Commands::Unpin => {
            let mut request = dispatch_request(proto::DispatchType::SetPinned);
            request.x = 0;
            dispatch_command(&cli.address, request, "Unpinned active window").await?;
        }

        Commands::MoveTo { selector } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::MoveToWorkspace);
            request.argument = selector.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Moved active window silently to workspace {}",
                    selector.expression()
                ),
            )
            .await?;
        }

        Commands::MoveToAndFollow { selector } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::MoveToWorkspaceAndFollow);
            request.argument = selector.expression();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Moved active window and followed workspace {}",
                    selector.expression()
                ),
            )
            .await?;
        }

        Commands::WorkspaceRename { selector, name } => {
            let selector = selector
                .parse::<WorkspaceSelector>()
                .map_err(|message| anyhow!(message))?;
            if name.trim().is_empty() {
                return Err(anyhow!("Workspace name cannot be empty"));
            }
            let mut request = dispatch_request(proto::DispatchType::RenameWorkspace);
            request.argument = selector.expression();
            request.secondary_argument = name.trim().to_string();
            dispatch_command(
                &cli.address,
                request,
                &format!(
                    "Renamed workspace {} to {}",
                    selector.expression(),
                    name.trim()
                ),
            )
            .await?;
        }

        Commands::LayoutNext => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CycleNextLayout),
                "Selected next layout",
            )
            .await?;
        }

        Commands::LayoutPrevious => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CyclePreviousLayout),
                "Selected previous layout",
            )
            .await?;
        }

        Commands::LayoutSet { layout } => {
            let layout = layout
                .parse::<LayoutType>()
                .map_err(|message| anyhow!(message))?;
            let mut request = dispatch_request(proto::DispatchType::SetLayout);
            request.argument = layout.config_name().to_string();
            dispatch_command(&cli.address, request, "Selected layout").await?;
        }

        Commands::LayoutRatio { ratio } => {
            if !(0.1..=0.9).contains(&ratio) {
                return Err(anyhow!("Layout ratio must be between 0.1 and 0.9"));
            }
            let mut request = dispatch_request(proto::DispatchType::SetLayoutRatio);
            request.x = (ratio * 1000.0).round() as i32;
            dispatch_command(&cli.address, request, "Updated master ratio").await?;
        }

        Commands::LayoutRatioAdjust { delta } => {
            let mut request = dispatch_request(proto::DispatchType::AdjustLayoutRatio);
            request.x = (delta * 1000.0).round() as i32;
            dispatch_command(&cli.address, request, "Adjusted master ratio").await?;
        }

        Commands::LayoutMasterCount { count } => {
            if !(1..=8).contains(&count) {
                return Err(anyhow!("Master count must be between 1 and 8"));
            }
            let mut request = dispatch_request(proto::DispatchType::SetMasterCount);
            request.x = count as i32;
            dispatch_command(&cli.address, request, "Updated master count").await?;
        }

        Commands::LayoutRotate => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::RotateLayout),
                "Rotated active layout",
            )
            .await?;
        }

        Commands::LayoutGap { pixels } => {
            if !(0..=500).contains(&pixels) {
                return Err(anyhow!("Layout gap must be between 0 and 500 pixels"));
            }
            let mut request = dispatch_request(proto::DispatchType::SetLayoutGap);
            request.x = pixels;
            dispatch_command(&cli.address, request, "Updated layout gap").await?;
        }

        Commands::LayoutGapAdjust { delta } => {
            let mut request = dispatch_request(proto::DispatchType::AdjustLayoutGap);
            request.x = delta;
            dispatch_command(&cli.address, request, "Adjusted layout gap").await?;
        }

        Commands::LayoutPromote => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::PromoteToMaster),
                "Promoted active window to master",
            )
            .await?;
        }

        Commands::ResizeEdge { edge, sizing } => {
            let mut request = dispatch_request(proto::DispatchType::ResizeEdge);
            request.direction = proto::Direction::from(edge) as i32;
            request.argument = sizing.config_name().to_string();
            dispatch_command(&cli.address, request, "Resized window edge").await?;
        }

        Commands::ResizeAxis { axis, sizing } => {
            let mut request = dispatch_request(proto::DispatchType::ResizeAxis);
            request.argument = axis.config_name().to_string();
            request.secondary_argument = sizing.config_name().to_string();
            dispatch_command(&cli.address, request, "Resized window axis").await?;
        }

        Commands::ResizeDelta { pixels } => {
            if !(1..=1000).contains(&pixels) {
                return Err(anyhow!("Resize delta must be between 1 and 1000 pixels"));
            }
            let mut request = dispatch_request(proto::DispatchType::SetResizeDelta);
            request.x = pixels;
            dispatch_command(&cli.address, request, "Updated resize delta").await?;
        }

        Commands::FlipLayout { axis } => {
            let mut request = dispatch_request(proto::DispatchType::FlipLayout);
            request.argument = axis.config_name().to_string();
            dispatch_command(&cli.address, request, "Flipped workspace layout").await?;
        }

        Commands::ToggleMonocle => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleMonocle),
                "Toggled monocle",
            )
            .await?;
        }

        Commands::ToggleLayoutLock => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleLayoutLock),
                "Toggled layout resize lock",
            )
            .await?;
        }

        Commands::StackAll => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::StackAll),
                "Stacked all windows on the workspace",
            )
            .await?;
        }

        Commands::UnstackAll => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::UnstackAll),
                "Unstacked the focused group",
            )
            .await?;
        }

        Commands::ToggleStackbar => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleStackbar),
                "Toggled stackbar",
            )
            .await?;
        }

        Commands::SaveResize { path } => {
            let mut request = dispatch_request(proto::DispatchType::SaveResize);
            request.argument = path.to_string_lossy().to_string();
            dispatch_command(&cli.address, request, "Saved resize dimensions").await?;
        }

        Commands::LoadResize { path } => {
            let mut request = dispatch_request(proto::DispatchType::LoadResize);
            request.argument = path.to_string_lossy().to_string();
            dispatch_command(&cli.address, request, "Loaded resize dimensions").await?;
        }

        Commands::QuickSaveResize => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::QuickSaveResize),
                "Saved resize dimensions to the temp file",
            )
            .await?;
        }

        Commands::QuickLoadResize => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::QuickLoadResize),
                "Loaded resize dimensions from the temp file",
            )
            .await?;
        }

        Commands::WatchConfiguration { enabled } => {
            let mut request = dispatch_request(proto::DispatchType::WatchConfiguration);
            request.x = i32::from(enabled);
            dispatch_command(
                &cli.address,
                request,
                if enabled {
                    "Configuration watch enabled"
                } else {
                    "Configuration watch disabled"
                },
            )
            .await?;
        }

        Commands::ToggleTransparency => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleTransparency),
                "Toggled transparency",
            )
            .await?;
        }

        Commands::TransparencyAlpha { alpha } => {
            let mut request = dispatch_request(proto::DispatchType::SetTransparencyAlpha);
            request.x = alpha;
            dispatch_command(&cli.address, request, "Set transparency alpha").await?;
        }

        Commands::GlobalWorkAreaOffset {
            left,
            top,
            right,
            bottom,
        } => {
            let mut request = dispatch_request(proto::DispatchType::SetGlobalWorkAreaOffset);
            request.argument = format!("{left},{top},{right},{bottom}");
            dispatch_command(&cli.address, request, "Set global work-area offset").await?;
        }

        Commands::WorkspaceWorkAreaOffset {
            left,
            top,
            right,
            bottom,
        } => {
            let mut request = dispatch_request(proto::DispatchType::SetWorkspaceWorkAreaOffset);
            request.argument = format!("{left},{top},{right},{bottom}");
            dispatch_command(&cli.address, request, "Set workspace work-area offset").await?;
        }

        Commands::WorkspacePadding { padding } => {
            let mut request = dispatch_request(proto::DispatchType::SetWorkspacePadding);
            request.x = padding;
            dispatch_command(&cli.address, request, "Set workspace padding").await?;
        }

        Commands::AdjustWorkspacePadding { delta } => {
            let mut request = dispatch_request(proto::DispatchType::AdjustWorkspacePadding);
            request.x = delta;
            dispatch_command(&cli.address, request, "Adjusted workspace padding").await?;
        }

        Commands::ContainerPadding { padding } => {
            let mut request = dispatch_request(proto::DispatchType::SetContainerPadding);
            request.x = padding;
            dispatch_command(&cli.address, request, "Set container padding").await?;
        }

        Commands::AdjustContainerPadding { delta } => {
            let mut request = dispatch_request(proto::DispatchType::AdjustContainerPadding);
            request.x = delta;
            dispatch_command(&cli.address, request, "Adjusted container padding").await?;
        }

        #[cfg(target_os = "macos")]
        Commands::EnableAutostart => {
            let agent = macos_app::enable_autostart(&cli.address)?;
            println!(
                "Autostart enabled: {} starts gevrek when you sign in",
                agent.display()
            );
        }
        #[cfg(target_os = "macos")]
        Commands::DisableAutostart => match macos_app::disable_autostart()? {
            Some(agent) => println!("Autostart disabled: removed {}", agent.display()),
            None => println!("Autostart was not enabled"),
        },
        #[cfg(target_os = "macos")]
        Commands::AutostartStatus => println!("{}", macos_app::autostart_status()),

        #[cfg(not(target_os = "macos"))]
        Commands::EnableAutostart | Commands::DisableAutostart | Commands::AutostartStatus
            if !cfg!(windows) =>
        {
            // On Linux gevrek is the session itself: it is chosen in the
            // login manager, not started at sign-in of another desktop.
            return Err(anyhow!(
                "Autostart at sign-in is for Windows and macOS. On Linux gevrek is a session of its own, chosen in the login manager."
            ));
        }
        Commands::Update {
            check,
            automatic,
            background,
        } => {
            if let Some(said) = automatic {
                let on = match said.to_ascii_lowercase().as_str() {
                    "on" | "true" | "yes" => true,
                    "off" | "false" | "no" => false,
                    other => return Err(anyhow!("gevrek update --automatic {other}: on or off")),
                };
                let folder = update_folder();
                let mut settings = gevrek_update::Settings::load(&folder);
                settings.automatic = on;
                settings.save(&folder)?;
                if on {
                    println!("gevrek looks for a newer release once a day when it starts, and installs it the next time it starts.");
                } else {
                    println!("gevrek does not look for updates by itself. `gevrek update` still does when you ask.");
                }
            } else if background {
                update_behind(&cli.address).await?;
            } else {
                update_now(&cli.address, check).await?;
            }
        }

        #[cfg(not(target_os = "macos"))]
        Commands::EnableAutostart => match enable_autostart()? {
            Autostart::Task(task) => {
                println!("Autostart enabled: the scheduled task {task} starts gevrek at sign-in")
            }
            Autostart::Script(path, why) => {
                println!("Autostart enabled: {}", path.display());
                println!(
                    "(A script in the Startup folder: the scheduled task could not be made: {why})"
                );
            }
        },

        #[cfg(not(target_os = "macos"))]
        Commands::DisableAutostart => {
            let removed = disable_autostart()?;
            if removed.is_empty() {
                println!("Autostart was not enabled");
            } else {
                println!("Autostart disabled: removed {}", removed.join(" and "));
            }
        }

        #[cfg(not(target_os = "macos"))]
        Commands::AutostartStatus => println!("{}", autostart_status()?),

        Commands::Promote => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::Promote),
                "Promoted active window to the front",
            )
            .await?;
        }

        Commands::PromoteFocus => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::PromoteFocus),
                "Focused the front container",
            )
            .await?;
        }

        Commands::PromoteWindow { direction } => {
            let mut request = dispatch_request(proto::DispatchType::PromoteWindow);
            request.direction = proto::Direction::from(direction) as i32;
            dispatch_command(&cli.address, request, "Promoted window in direction").await?;
        }

        Commands::DwindleDirection { direction } => {
            let mut request = dispatch_request(proto::DispatchType::SetDwindleDirection);
            request.argument = direction.config_name().to_string();
            dispatch_command(&cli.address, request, "Updated Dwindle split policy").await?;
        }

        Commands::DwindleCycleSplit => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CycleDwindleDirection),
                "Cycled Dwindle split policy",
            )
            .await?;
        }

        Commands::DwindlePreserveSplit => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleDwindlePreserveSplit),
                "Toggled Dwindle split preservation",
            )
            .await?;
        }

        Commands::DwindlePseudotile => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::TogglePseudotile),
                "Toggled pseudotiling for the active window",
            )
            .await?;
        }

        Commands::GroupToggle => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleGroup),
                "Toggled active group",
            )
            .await?;
        }
        Commands::GroupToggleWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowGroup, &selector)?;
            dispatch_command(&cli.address, request, "Toggled selected window group").await?;
        }

        Commands::GroupLock => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleGroupLock),
                "Toggled active group lock",
            )
            .await?;
        }
        Commands::GroupLockWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::ToggleWindowGroupLock, &selector)?;
            dispatch_command(&cli.address, request, "Toggled selected window group lock").await?;
        }

        Commands::GroupNext => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CycleGroupNext),
                "Focused next group tab",
            )
            .await?;
        }

        Commands::GroupPrevious => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CycleGroupPrevious),
                "Focused previous group tab",
            )
            .await?;
        }
        Commands::GroupNextWindow { selector } => {
            let mut request =
                selected_window_request(proto::DispatchType::CycleWindowGroup, &selector)?;
            request.x = 1;
            dispatch_command(&cli.address, request, "Focused next selected group tab").await?;
        }
        Commands::GroupPreviousWindow { selector } => {
            let request =
                selected_window_request(proto::DispatchType::CycleWindowGroup, &selector)?;
            dispatch_command(&cli.address, request, "Focused previous selected group tab").await?;
        }
        Commands::GroupFocus { index } => {
            let mut request = dispatch_request(proto::DispatchType::FocusGroupIndex);
            request.x = i32::try_from(index)
                .map_err(|_| anyhow!("Group tab index is too large for the protocol"))?;
            dispatch_command(&cli.address, request, "Focused group tab by index").await?;
        }
        Commands::GroupFocusWindow { selector, index } => {
            let mut request =
                selected_window_request(proto::DispatchType::FocusWindowGroupIndex, &selector)?;
            request.x = i32::try_from(index)
                .map_err(|_| anyhow!("Group tab index is too large for the protocol"))?;
            dispatch_command(&cli.address, request, "Focused selected group tab by index").await?;
        }
        Commands::GroupMoveForward => {
            let mut request = dispatch_request(proto::DispatchType::MoveGroupWindow);
            request.x = 1;
            dispatch_command(&cli.address, request, "Moved active group tab forward").await?;
        }
        Commands::GroupMoveBackward => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::MoveGroupWindow),
                "Moved active group tab backward",
            )
            .await?;
        }
        Commands::GroupMoveWindowForward { selector } => {
            let mut request =
                selected_window_request(proto::DispatchType::MoveWindowInGroup, &selector)?;
            request.x = 1;
            dispatch_command(&cli.address, request, "Moved selected group tab forward").await?;
        }
        Commands::GroupMoveWindowBackward { selector } => {
            let request =
                selected_window_request(proto::DispatchType::MoveWindowInGroup, &selector)?;
            dispatch_command(&cli.address, request, "Moved selected group tab backward").await?;
        }

        Commands::GroupMergeNext => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::MergeGroupNext),
                "Merged into next group",
            )
            .await?;
        }

        Commands::GroupMergePrevious => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::MergeGroupPrevious),
                "Merged into previous group",
            )
            .await?;
        }

        Commands::GroupRemove => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::UngroupActive),
                "Removed active window from group",
            )
            .await?;
        }

        Commands::Expose => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ToggleExpose),
                "Toggled window overview",
            )
            .await?;
        }
        Commands::Shortcuts => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ShowShortcuts),
                "Toggled the list of shortcuts",
            )
            .await?;
        }

        Commands::Scratchpad { id } => {
            let mut request = dispatch_request(proto::DispatchType::ToggleScratchpad);
            request.argument = id.clone();
            dispatch_command(
                &cli.address,
                request,
                &format!("Toggled scratchpad '{}'", id),
            )
            .await?;
        }
        Commands::Exec { program, args } => {
            let mut request = dispatch_request(proto::DispatchType::Exec);
            request.argument = program.clone();
            if !args.is_empty() {
                request.secondary_argument = serde_json::to_string(&args)?;
            }
            dispatch_command(&cli.address, request, &format!("Started {program}")).await?;
        }
        Commands::StartupLaunch { id } => {
            let mut request = dispatch_request(proto::DispatchType::LaunchStartup);
            request.argument = id;
            dispatch_command(&cli.address, request, "Launched startup application").await?;
        }
        Commands::StartupStop { id } => {
            let mut request = dispatch_request(proto::DispatchType::StopStartup);
            request.argument = id;
            dispatch_command(&cli.address, request, "Stopped startup application").await?;
        }
        Commands::StartupRestart { id } => {
            let mut request = dispatch_request(proto::DispatchType::RestartStartup);
            request.argument = id;
            dispatch_command(&cli.address, request, "Restarted startup application").await?;
        }
        Commands::SnapZone { index } => {
            let mut request = dispatch_request(proto::DispatchType::SnapActiveToZone);
            request.x = index as i32;
            dispatch_command(
                &cli.address,
                request,
                &format!("Snapped active window to zone {}", index),
            )
            .await?;
        }
        Commands::SnapZoneNext => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CycleActiveZoneNext),
                "Snapped active window to the next zone",
            )
            .await?;
        }
        Commands::SnapZonePrevious => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::CycleActiveZonePrevious),
                "Snapped active window to the previous zone",
            )
            .await?;
        }
        Commands::Submap { name } => {
            let mut request = dispatch_request(proto::DispatchType::SetBindingSubmap);
            request.argument = name.clone();
            dispatch_command(
                &cli.address,
                request,
                &format!("Entered binding submap '{}'", name),
            )
            .await?;
        }
        Commands::SubmapReset => {
            dispatch_command(
                &cli.address,
                dispatch_request(proto::DispatchType::ResetBindingSubmap),
                "Returned to the default binding submap",
            )
            .await?;
        }

        Commands::Zones {
            action,
            json,
            timeout_ms,
        } => {
            zones_command(&cli.address, action, json, timeout_ms).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        autostart_script_contents, autostart_script_path_in, autostart_task_name,
        autostart_task_xml, ensure_wm_endpoint_available, execution_event_to_json, upsert_rule,
        validate_complete_config, Cli, CliAxis, CliDirection, CliSizing, CliSwitch, Commands,
    };
    use clap::Parser;
    use gevrek_model::settings::WindowRuleProfile;
    use std::path::{Path, PathBuf};

    /// Read a command line as `main` does: on a thread with room. (The
    /// reader clap writes for so many commands needs about two megabytes
    /// of stack in a debug build, as much as a test's thread has in all;
    /// the type it reads into is small, 80 bytes.)
    fn read<I, T>(arguments: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T> + Send,
        T: Into<std::ffi::OsString> + Clone,
    {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(16 * 1024 * 1024)
                .spawn_scoped(scope, || Cli::try_parse_from(arguments))
                .expect("a thread to read the command line on")
                .join()
                .expect("the command line was read")
        })
    }

    #[test]
    fn the_wallpaper_command_edits_the_section() {
        use super::{
            wallpaper_clear, wallpaper_displays, wallpaper_fit, wallpaper_from_json, wallpaper_set,
            WallpaperAction,
        };
        use gevrek_model::wallpaper::{WallpaperConfig, WallpaperFit};

        let mut section = WallpaperConfig::default();
        // One display's own: added with the common fit, then replaced.
        section.fit = WallpaperFit::Fit;
        wallpaper_set(&mut section, "/a.png".into(), Some("DP-2"), None);
        wallpaper_set(
            &mut section,
            "/b.png".into(),
            Some("dp-2"),
            Some(WallpaperFit::Tile),
        );
        wallpaper_set(&mut section, "/c.png".into(), Some("DP-3"), None);
        assert_eq!(section.image, "");
        assert_eq!(section.outputs.len(), 2);
        assert_eq!(section.outputs[0].image, "/b.png");
        assert_eq!(section.outputs[0].fit, WallpaperFit::Tile);
        assert_eq!(section.outputs[1].fit, WallpaperFit::Fit);
        // One display's own taken away; one that has none.
        assert!(wallpaper_clear(&mut section, Some("DP-3")));
        assert!(!wallpaper_clear(&mut section, Some("DP-3")));
        assert_eq!(section.outputs.len(), 1);
        // Every display's: none keeps its own, the fit stays unless said.
        wallpaper_set(&mut section, "/all.jpg".into(), None, None);
        assert_eq!(
            (section.image.as_str(), section.fit),
            ("/all.jpg", WallpaperFit::Fit)
        );
        assert!(section.outputs.is_empty());
        wallpaper_set(
            &mut section,
            "/all.jpg".into(),
            None,
            Some(WallpaperFit::Fill),
        );
        assert_eq!(section.fit, WallpaperFit::Fill);
        // Cleared: the colour stays.
        let colour = section.color;
        assert!(wallpaper_clear(&mut section, None));
        assert!(!wallpaper_clear(&mut section, None));
        assert_eq!((section.image.as_str(), section.color), ("", colour));

        assert_eq!(wallpaper_fit("Centre").unwrap(), WallpaperFit::Centre);
        assert_eq!(wallpaper_fit("center").unwrap(), WallpaperFit::Centre);
        assert!(wallpaper_fit("span").is_err());

        let applied = wallpaper_from_json(
            r#"{"image": "/x.png", "outputs": [{"name": "DP-2", "image": "/y.png", "fit": "stretch"}]}"#,
        )
        .unwrap();
        assert_eq!(applied.image, "/x.png");
        assert_eq!(applied.color, WallpaperConfig::default().color);
        assert_eq!(applied.outputs[0].fit, WallpaperFit::Stretch);
        assert!(wallpaper_from_json("[]").is_err());
        assert!(wallpaper_from_json(r#"{"fit": "span"}"#).is_err());
        assert!(wallpaper_from_json("{").is_err());

        let displays = wallpaper_displays(&serde_json::json!({"monitors": [
            {"name": "Monitor (1)", "connector": "DP-2",
             "position": {"x": 2560.0, "y": 0.0}, "size": {"width": 2560, "height": 1440}},
            {"name": "Display 2", "position": {"x": 0.0, "y": 0.0}, "size": {"width": 1920, "height": 1080}},
        ]}));
        assert_eq!(
            displays[0],
            serde_json::json!({"name": "DP-2", "x": 2560, "y": 0, "width": 2560, "height": 1440})
        );
        assert_eq!(displays[1]["name"], "Display 2");

        match read(["gevrek", "wallpaper", "--json"])
            .expect("read")
            .command
        {
            Commands::Wallpaper {
                json: true,
                action: None,
            } => {}
            _ => panic!("gevrek wallpaper --json"),
        }
        match read([
            "gevrek",
            "wallpaper",
            "set",
            "a.png",
            "--output",
            "DP-2",
            "--fit",
            "tile",
        ])
        .expect("read")
        .command
        {
            Commands::Wallpaper {
                action: Some(action),
                ..
            } => assert_eq!(
                action,
                WallpaperAction::Set {
                    file: PathBuf::from("a.png"),
                    output: Some("DP-2".into()),
                    fit: Some("tile".into()),
                }
            ),
            _ => panic!("gevrek wallpaper set"),
        }
    }

    #[test]
    fn start_all_starts_the_shell_where_it_is_this_programs_to_start() {
        use super::{parse_components, shell_is_started, Component};
        // On Windows, when its program is there; on Linux the session
        // starts it.
        assert!(shell_is_started(true, true));
        assert!(!shell_is_started(false, true));
        assert!(!shell_is_started(true, false));

        // The settings program is not started with the rest: it is
        // opened by whoever wants it.
        let started = parse_components("all", true);
        assert_eq!(started[0], Component::Wm);
        assert!(!started.contains(&Component::Editor));
        assert!(started.len() <= 2);
        assert_eq!(parse_components("editor", true), [Component::Editor]);
        // (And stopped with the rest all the same, where it runs.)
        assert!(parse_components("all", false).contains(&Component::Editor));
        // Stopped: whatever this program can have started.
        let stopped = parse_components("all", false);
        assert_eq!(stopped.contains(&Component::Shell), cfg!(windows));
        assert_eq!(stopped[0], Component::Wm);
        assert_eq!(parse_components("shell", true), [Component::Shell]);
        assert_eq!(parse_components("panel", false), [Component::Shell]);
        // (The bar of before the shell is gone, and its name with it.)
        assert!(parse_components("overlay", true).is_empty());
        assert!(parse_components("nothing", true).is_empty());
    }

    #[test]
    fn what_the_on_screen_display_is_asked_to_show_is_words_and_maybe_a_level() {
        use super::osd_command;
        use gevrek_model::command::DispatchCommand;
        let said =
            |words: &[&str]| -> Vec<String> { words.iter().map(|word| word.to_string()).collect() };
        assert_eq!(
            osd_command(&said(&["Volume", "40"])).unwrap(),
            DispatchCommand::Osd {
                text: "Volume".to_string(),
                level: Some(40)
            }
        );
        assert_eq!(
            osd_command(&said(&["Grid"])).unwrap(),
            DispatchCommand::Osd {
                text: "Grid".to_string(),
                level: None
            }
        );
        // Nothing to show, a level that is none, or too much said.
        assert!(osd_command(&[]).is_err());
        assert!(osd_command(&said(&["Volume", "loud"])).is_err());
        assert!(osd_command(&said(&["Volume", "140"])).is_err());
        assert!(osd_command(&said(&["Volume", "40", "more"])).is_err());
    }

    #[test]
    fn resize_persistence_watch_and_autostart_commands_parse() {
        assert!(matches!(
            read(["gevrek", "save-resize", "layout.json"])
                .unwrap()
                .command,
            Commands::SaveResize { path } if path.as_path() == Path::new("layout.json")
        ));
        assert!(matches!(
            read(["gevrek", "load-resize", "C:\\layouts\\m.json"])
                .unwrap()
                .command,
            Commands::LoadResize { path } if path.as_path() == Path::new("C:\\layouts\\m.json")
        ));
        assert!(matches!(
            read(["gevrek", "quick-save-resize"]).unwrap().command,
            Commands::QuickSaveResize
        ));
        assert!(matches!(
            read(["gevrek", "quick-load-resize"]).unwrap().command,
            Commands::QuickLoadResize
        ));
        assert!(matches!(
            read(["gevrek", "watch-configuration", "true"])
                .unwrap()
                .command,
            Commands::WatchConfiguration { enabled: true }
        ));
        assert!(matches!(
            read(["gevrek", "watch-configuration", "false"])
                .unwrap()
                .command,
            Commands::WatchConfiguration { enabled: false }
        ));
        assert!(read(["gevrek", "watch-configuration"]).is_err());
        assert!(matches!(
            read(["gevrek", "toggle-transparency"]).unwrap().command,
            Commands::ToggleTransparency
        ));
        assert!(matches!(
            read(["gevrek", "transparency-alpha", "180"])
                .unwrap()
                .command,
            Commands::TransparencyAlpha { alpha: 180 }
        ));
        assert!(matches!(
            read(["gevrek", "global-work-area-offset", "10", "20", "30", "40"])
                .unwrap()
                .command,
            Commands::GlobalWorkAreaOffset {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            }
        ));
        assert!(read(["gevrek", "global-work-area-offset", "10", "20"]).is_err());
        assert!(matches!(
            read(["gevrek", "workspace-work-area-offset", "1", "2", "3", "4"])
                .unwrap()
                .command,
            Commands::WorkspaceWorkAreaOffset {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4,
            }
        ));
        assert!(matches!(
            read(["gevrek", "workspace-padding", "12"]).unwrap().command,
            Commands::WorkspacePadding { padding: 12 }
        ));
        assert!(matches!(
            read(["gevrek", "adjust-workspace-padding", "-5"])
                .unwrap()
                .command,
            Commands::AdjustWorkspacePadding { delta: -5 }
        ));
        assert!(matches!(
            read(["gevrek", "container-padding", "8"]).unwrap().command,
            Commands::ContainerPadding { padding: 8 }
        ));
        assert!(matches!(
            read(["gevrek", "adjust-container-padding", "3"])
                .unwrap()
                .command,
            Commands::AdjustContainerPadding { delta: 3 }
        ));
        assert!(matches!(
            read(["gevrek", "enable-autostart"]).unwrap().command,
            Commands::EnableAutostart
        ));
        assert!(matches!(
            read(["gevrek", "disable-autostart"]).unwrap().command,
            Commands::DisableAutostart
        ));
    }

    #[test]
    fn the_update_command_parses() {
        assert!(matches!(
            read(["gevrek", "update"]).unwrap().command,
            Commands::Update {
                check: false,
                automatic: None,
                background: false
            }
        ));
        assert!(matches!(
            read(["gevrek", "update", "--check"]).unwrap().command,
            Commands::Update { check: true, .. }
        ));
        assert!(matches!(
            read(["gevrek", "update", "--automatic", "off"]).unwrap().command,
            Commands::Update { automatic: Some(said), .. } if said == "off"
        ));
        // What `gevrek start` leaves running behind itself.
        assert!(matches!(
            read(["gevrek", "--address", "local", "update", "--background"])
                .unwrap()
                .command,
            Commands::Update {
                background: true,
                ..
            }
        ));
    }

    #[test]
    fn stopping_one_instance_leaves_the_programs_of_another() {
        use super::pids_of_instance;
        let claim = |instance: &str, pid: u32, running: bool| (instance.to_string(), pid, running);
        // Three window managers run: the main one's (an older build that
        // says nothing about itself), the main one's that does, and dev's.
        let discovered = [10, 20, 30];
        let claims = [claim("main", 20, true), claim("dev", 30, true)];
        assert_eq!(pids_of_instance("main", &discovered, &claims), [10, 20]);
        assert_eq!(pids_of_instance("dev", &discovered, &claims), [30]);
        // An instance with nothing running has nothing, whatever else runs.
        assert!(pids_of_instance("other", &discovered, &claims).is_empty());
        // A claim of a process that ended counts for nothing: its number
        // may be another program's by now.
        let stale = [claim("dev", 10, false)];
        assert_eq!(pids_of_instance("main", &discovered, &stale), [10, 20, 30]);
        assert!(pids_of_instance("dev", &discovered, &stale).is_empty());
    }

    #[test]
    fn a_graph_file_is_not_taken_for_a_configuration() {
        use super::is_bare_graph;
        let graph = serde_json::to_string(&gevrek_model::graph::NodeGraph::new()).unwrap();
        assert!(is_bare_graph(&graph));
        // The reason the question has to be asked: it reads as a
        // configuration, an empty one.
        assert!(gevrek_model::Config::from_json(&graph).is_ok());
        let config = serde_json::to_string(&gevrek_model::Config::new()).unwrap();
        assert!(!is_bare_graph(&config));
        assert!(!is_bare_graph("{}"));
        assert!(!is_bare_graph("not json"));
    }

    #[test]
    fn autostart_script_path_lives_in_the_startup_folder() {
        let folder = PathBuf::from(
            "C:\\Users\\test\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup",
        );
        assert_eq!(
            autostart_script_path_in(&folder),
            folder.join("gevrek-autostart.cmd")
        );
    }

    // Windows paths: the folder of the executable is found by the backslash.
    #[cfg(windows)]
    #[test]
    fn autostart_script_runs_start_all_from_the_executable_dir() {
        let contents =
            autostart_script_contents(Path::new("C:\\Program Files\\Gevrek\\gevrek.exe"));
        assert!(contents.starts_with("@echo off"));
        assert!(contents.contains("cd /d \"C:\\Program Files\\Gevrek\""));
        assert!(contents.contains("start \"\" \"C:\\Program Files\\Gevrek\\gevrek.exe\" start all"));
    }

    #[test]
    fn the_autostart_task_runs_start_all_for_the_person_signing_in() {
        let xml = autostart_task_xml(
            Path::new("C:\\Program Files\\A&B\\gevrek.exe"),
            "DESK\\ada",
            "main",
        );
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));
        // At that person's sign-in, as them, without raised rights.
        assert_eq!(xml.matches("<UserId>DESK\\ada</UserId>").count(), 2);
        assert!(
            xml.contains("<LogonTrigger>") && xml.contains("<RunLevel>LeastPrivilege</RunLevel>")
        );
        // The program with what XML needs written its way.
        assert!(xml.contains("<Arguments>start all</Arguments>"));
        if cfg!(windows) {
            assert!(xml.contains("<Command>C:\\Program Files\\A&amp;B\\gevrek.exe</Command>"));
            assert!(xml.contains("<WorkingDirectory>C:\\Program Files\\A&amp;B</WorkingDirectory>"));
        }
        // On battery too, and never ended for running long.
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    }

    #[test]
    fn another_instance_has_its_own_task_and_is_named_to_the_program() {
        assert_eq!(autostart_task_name("main"), r"\gevrek\autostart");
        assert_eq!(autostart_task_name("checks"), r"\gevrek\autostart-checks");
        let xml = autostart_task_xml(Path::new("C:\\g\\gevrek.exe"), "ada", "checks");
        assert!(xml.contains("<Command>cmd.exe</Command>"));
        assert!(xml.contains("set GEVREK_INSTANCE=checks&amp;&amp; "));
        assert!(xml.contains("start all&quot;</Arguments>"));
    }

    #[test]
    fn autostart_script_write_and_remove_round_trip() {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-autostart-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let path = autostart_script_path_in(&folder);
        std::fs::write(
            &path,
            autostart_script_contents(Path::new("C:\\gevrek\\gevrek.exe")),
        )
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("gevrek.exe"));
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir(&folder);
    }

    #[test]
    fn persistent_rule_upsert_preserves_order_and_replaces_by_exact_name() {
        let mut config = gevrek_model::Config::new();
        let first = WindowRuleProfile {
            name: "Browser".to_string(),
            ..WindowRuleProfile::default()
        };
        assert!(!upsert_rule(&mut config, first));
        assert_eq!(config.desktop.window_rules.len(), 1);

        let replacement = WindowRuleProfile {
            name: "Browser".to_string(),
            enabled: false,
            ..WindowRuleProfile::default()
        };
        assert!(upsert_rule(&mut config, replacement));
        assert_eq!(config.desktop.window_rules.len(), 1);
        assert!(!config.desktop.window_rules[0].enabled);
    }

    #[test]
    fn cli_validation_uses_the_complete_config_validator() {
        let mut config = gevrek_model::Config::new();
        assert!(validate_complete_config(&config).is_ok());
        config.desktop.animations.window.duration_ms = 1;
        assert!(validate_complete_config(&config).is_err());
    }

    #[test]
    fn runtime_extension_events_have_stable_ndjson_fields() {
        use gevrek_protocol::proto::execution_event::Event;

        let event = gevrek_protocol::proto::ExecutionEvent {
            event: Some(Event::RuntimeEvent(gevrek_protocol::proto::RuntimeEvent {
                kind: "window.moved".to_string(),
                timestamp_ms: 42,
                window_id: Some(99),
                fields: [
                    ("x".to_string(), "120".to_string()),
                    ("y".to_string(), "80".to_string()),
                ]
                .into_iter()
                .collect(),
            })),
        };
        let (json, is_error) = execution_event_to_json(event).unwrap();
        assert!(!is_error);
        assert_eq!(json["event"], "window.moved");
        assert_eq!(json["window_handle"], 99);
        assert_eq!(json["fields"]["x"], "120");
    }

    #[test]
    fn occupied_wm_endpoint_is_rejected_before_process_launch() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        assert!(ensure_wm_endpoint_available(&address).is_err());

        drop(listener);
        assert!(ensure_wm_endpoint_available(&address).is_ok());
    }

    #[test]
    fn precise_window_control_commands_parse() {
        for (name, matches) in [
            ("float", matches_command_float as fn(&Commands) -> bool),
            ("tile", matches_command_tile),
            ("center", matches_command_center),
            ("raise", matches_command_raise),
            ("lower", matches_command_lower),
            ("pin", matches_command_pin),
            ("unpin", matches_command_unpin),
            ("tag-clear", matches_command_tag_clear),
        ] {
            let cli = read(["gevrek", name]).unwrap();
            assert!(matches(&cli.command), "{name} parsed to the wrong command");
        }
    }

    #[test]
    fn targeted_focus_commands_parse() {
        assert!(matches!(
            read(["gevrek", "focus-last"]).unwrap().command,
            Commands::FocusLast
        ));
        assert!(matches!(
            read(["gevrek", "focus-window", "class:WindowsTerminal"])
                .unwrap()
                .command,
            Commands::FocusWindow { selector }
                if selector == "class:WindowsTerminal"
        ));
    }

    #[test]
    fn snap_zone_commands_parse_and_their_old_names_still_do() {
        for name in ["snap-zone", "zone"] {
            assert!(matches!(
                read(["gevrek", name, "2"]).unwrap().command,
                Commands::SnapZone { index: 2 }
            ));
        }
        for name in ["snap-zone-next", "zone-next"] {
            assert!(matches!(
                read(["gevrek", name]).unwrap().command,
                Commands::SnapZoneNext
            ));
        }
        for name in ["snap-zone-previous", "zone-previous"] {
            assert!(matches!(
                read(["gevrek", name]).unwrap().command,
                Commands::SnapZonePrevious
            ));
        }
        // The plural is not one of them: it is the zones on the desktop,
        // which are asked for something (`gevrek zones list`).
        assert!(read(["gevrek", "zones"]).is_err());
    }

    #[test]
    fn the_zones_commands_parse_and_are_not_the_snap_zones() {
        use gevrek::zones_cli::ask::{RuleScope, RulesAction, SnapshotAction, Switch, ZonesAction};
        // What was asked, whether as JSON, and how long to wait.
        let zones = |arguments: &[&str]| -> (ZonesAction, bool, Option<u32>) {
            let mut line = vec!["gevrek", "zones"];
            line.extend_from_slice(arguments);
            match read(line).unwrap().command {
                Commands::Zones {
                    action,
                    json,
                    timeout_ms,
                } => (action, json, timeout_ms),
                _ => panic!("{arguments:?} was read as another command"),
            }
        };
        let work = || Some("work".to_string());
        assert_eq!(zones(&["list"]), (ZonesAction::List { items: false }, false, None));
        assert_eq!(
            zones(&["list", "--json", "--items"]),
            (ZonesAction::List { items: true }, true, None)
        );
        // --json is every word's, before it or after.
        assert_eq!(zones(&["show", "work", "--json"]), (ZonesAction::Show { zone: work() }, true, None));
        assert_eq!(zones(&["--json", "info", "work"]), (ZonesAction::Show { zone: work() }, true, None));
        assert_eq!(zones(&["show"]), (ZonesAction::Show { zone: None }, false, None));
        assert_eq!(
            zones(&["send", r#"{"command":"quick_hide"}"#]),
            (
                ZonesAction::Send {
                    command: r#"{"command":"quick_hide"}"#.to_string()
                },
                false,
                None
            )
        );
        assert_eq!(
            zones(&["send", "{}", "--json", "--timeout-ms", "800"]),
            (
                ZonesAction::Send {
                    command: "{}".to_string()
                },
                true,
                Some(800)
            )
        );
        assert_eq!(zones(&["events"]), (ZonesAction::Events, false, None));
        assert_eq!(zones(&["events", "--json"]), (ZonesAction::Events, true, None));
        assert_eq!(zones(&["restore-explorer"]).0, ZonesAction::RestoreExplorer);
        assert_eq!(zones(&["peek"]).0, ZonesAction::Peek { to: Switch::Toggle });
        assert_eq!(
            zones(&["roll-up", "work", "on", "--timeout-ms", "100"]),
            (
                ZonesAction::RollUp {
                    zone: "work".to_string(),
                    to: Switch::On
                },
                false,
                Some(100)
            )
        );
        assert_eq!(
            zones(&["rules", "dry-run", "--scope", "all", "--json"]),
            (
                ZonesAction::Rules {
                    action: RulesAction::DryRun {
                        scope: RuleScope::All
                    }
                },
                true,
                None
            )
        );
        // "Apply a layout" is restoring a saved copy, under either name.
        assert_eq!(
            zones(&["layout", "apply", "tidy"]).0,
            ZonesAction::Snapshot {
                action: SnapshotAction::Restore {
                    name: "tidy".to_string()
                }
            }
        );
        // A place may begin with a minus and is no flag for that.
        assert_eq!(
            zones(&["move", "work", "-20,40"]).0,
            ZonesAction::Move {
                zone: "work".to_string(),
                to: "-20,40".to_string(),
                display: None,
                no_snap: false
            }
        );
        // The address is said before or after, as for every command.
        assert_eq!(
            read(["gevrek", "zones", "list", "--address", "local:test"])
                .unwrap()
                .address,
            "local:test"
        );
        // What is not one of them is refused, not sent anywhere.
        assert!(read(["gevrek", "zones"]).is_err());
        assert!(read(["gevrek", "zones", "send"]).is_err());
        assert!(read(["gevrek", "zones", "2"]).is_err());
        assert!(read(["gevrek", "zones", "delete"]).is_err());
        // `zone 2` is still the snap zone of a window.
        assert!(matches!(
            read(["gevrek", "zone", "2"]).unwrap().command,
            Commands::SnapZone { index: 2 }
        ));
    }

    #[test]
    fn targeted_window_action_commands_parse() {
        assert!(matches!(
            read([
                "gevrek",
                "move-window",
                "tag:scratch",
                "40",
                "-20",
                "--relative"
            ])
            .unwrap()
            .command,
            Commands::MoveWindow {
                selector,
                x: 40,
                y: -20,
                relative: true,
            } if selector == "tag:scratch"
        ));
        assert!(matches!(
            read(["gevrek", "float-window", "class:Dialog"])
                .unwrap()
                .command,
            Commands::FloatWindow { selector } if selector == "class:Dialog"
        ));
        assert!(matches!(
            read(["gevrek", "close-window", "handle:42"])
                .unwrap()
                .command,
            Commands::CloseWindow { selector } if selector == "handle:42"
        ));
        assert!(matches!(
            read([
                "gevrek",
                "tag-toggle-window",
                "tag:communication",
                "urgent"
            ])
            .unwrap()
            .command,
            Commands::TagToggleWindow { selector, tag }
                if selector == "tag:communication" && tag == "urgent"
        ));
        assert!(matches!(
            read([
                "gevrek",
                "toggle-fullscreen-window",
                "pid:4242"
            ])
            .unwrap()
            .command,
            Commands::ToggleFullscreenWindow { selector } if selector == "pid:4242"
        ));
        assert!(matches!(
            read([
                "gevrek",
                "move-window-to-and-follow",
                "tag:communication",
                "name:chat"
            ])
            .unwrap()
            .command,
            Commands::MoveWindowToAndFollow { window, workspace }
                if window == "tag:communication" && workspace == "name:chat"
        ));
        assert!(matches!(
            read(["gevrek", "group-next-window", "tag:tabs"])
                .unwrap()
                .command,
            Commands::GroupNextWindow { selector } if selector == "tag:tabs"
        ));
        assert!(matches!(
            read(["gevrek", "group-focus-window", "class:Browser", "2"])
                .unwrap()
                .command,
            Commands::GroupFocusWindow { selector, index: 2 }
                if selector == "class:Browser"
        ));
        assert!(matches!(
            read(["gevrek", "group-move-window-forward", "pid:4242"])
                .unwrap()
                .command,
            Commands::GroupMoveWindowForward { selector } if selector == "pid:4242"
        ));
    }

    #[test]
    fn monitor_move_and_tiling_commands_parse() {
        assert!(matches!(
            read(["gevrek", "move-to-monitor", "next"])
                .unwrap()
                .command,
            Commands::MoveToMonitor { selector } if selector == "next"
        ));
        assert!(matches!(
            read(["gevrek", "send-to-monitor", "primary"])
                .unwrap()
                .command,
            Commands::SendToMonitor { selector } if selector == "primary"
        ));
        assert!(matches!(
            read(["gevrek", "move-window-to-monitor", "class:Notepad", "1"])
                .unwrap()
                .command,
            Commands::MoveWindowToMonitor { window, selector }
                if window == "class:Notepad" && selector == "1"
        ));
        assert!(matches!(
            read([
                "gevrek",
                "move-window-to-monitor-and-follow",
                "active",
                "name:Side display"
            ])
            .unwrap()
            .command,
            Commands::MoveWindowToMonitorAndFollow { window, selector }
                if window == "active" && selector == "name:Side display"
        ));
        assert!(matches!(
            read(["gevrek", "retile"]).unwrap().command,
            Commands::Retile
        ));
        assert!(matches!(
            read(["gevrek", "toggle-tiling"]).unwrap().command,
            Commands::ToggleTiling
        ));
        assert!(matches!(
            read(["gevrek", "toggle-workspace-tiling", "name:code"])
                .unwrap()
                .command,
            Commands::ToggleWorkspaceTiling { selector } if selector == "name:code"
        ));
        assert!(matches!(
            read(["gevrek", "toggle-workspace-float-override", "2"])
                .unwrap()
                .command,
            Commands::ToggleWorkspaceFloatOverride { selector } if selector == "2"
        ));
        assert!(matches!(
            read(["gevrek", "new-workspace"]).unwrap().command,
            Commands::NewWorkspace { name: None }
        ));
        assert!(matches!(
            read(["gevrek", "new-workspace", "mail"])
                .unwrap()
                .command,
            Commands::NewWorkspace { name: Some(name) } if name == "mail"
        ));
        assert!(matches!(
            read(["gevrek", "close-workspace"]).unwrap().command,
            Commands::CloseWorkspace
        ));
        assert!(matches!(
            read(["gevrek", "ensure-workspaces", "name:Side display", "3"])
                .unwrap()
                .command,
            Commands::EnsureWorkspaces { monitor, count: 3 } if monitor == "name:Side display"
        ));
        assert!(matches!(
            read(["gevrek", "ensure-named-workspace", "primary", "chat"])
                .unwrap()
                .command,
            Commands::EnsureNamedWorkspace { monitor, name }
                if monitor == "primary" && name == "chat"
        ));
        assert!(matches!(
            read(["gevrek", "toggle-window-container-behaviour"])
                .unwrap()
                .command,
            Commands::ToggleWindowContainerBehaviour
        ));
        assert!(matches!(
            read(["gevrek", "toggle-workspace-window-container-behaviour", "name:code"])
                .unwrap()
                .command,
            Commands::ToggleWorkspaceWindowContainerBehaviour { selector } if selector == "name:code"
        ));
        assert!(matches!(
            read(["gevrek", "toggle-title-bar"]).unwrap().command,
            Commands::ToggleTitleBar
        ));
        assert!(matches!(
            read(["gevrek", "toggle-window-title-bar", "handle:42"])
                .unwrap()
                .command,
            Commands::ToggleWindowTitleBar { selector } if selector == "handle:42"
        ));
    }

    #[test]
    fn focus_manage_and_restore_commands_parse() {
        assert!(matches!(
            read(["gevrek", "focus-monitor-at-cursor"]).unwrap().command,
            Commands::FocusMonitorAtCursor
        ));
        assert!(matches!(
            read(["gevrek", "focus-monitor-workspace", "1", "name:code"])
                .unwrap()
                .command,
            Commands::FocusMonitorWorkspace { monitor, workspace }
                if monitor == "1" && workspace == "name:code"
        ));
        assert!(matches!(
            read(["gevrek", "manage", "handle:42"])
                .unwrap()
                .command,
            Commands::Manage { selector } if selector == "handle:42"
        ));
        assert!(matches!(
            read(["gevrek", "force-focus", "handle:42"])
                .unwrap()
                .command,
            Commands::ForceFocus { selector } if selector == "handle:42"
        ));
        assert!(matches!(
            read(["gevrek", "unmanage", "active"])
                .unwrap()
                .command,
            Commands::Unmanage { selector } if selector == "active"
        ));
        assert!(matches!(
            read(["gevrek", "restore-windows"]).unwrap().command,
            Commands::RestoreWindows
        ));
        for (words, switch) in [
            (&["gevrek", "show-desktop"][..], CliSwitch::Toggle),
            (&["gevrek", "show-desktop", "toggle"][..], CliSwitch::Toggle),
            (&["gevrek", "show-desktop", "on"][..], CliSwitch::On),
            (&["gevrek", "show-desktop", "off"][..], CliSwitch::Off),
        ] {
            assert!(
                matches!(
                    read(words.iter().copied()).unwrap().command,
                    Commands::ShowDesktop { switch: given } if given == switch
                ),
                "{words:?}"
            );
        }
        assert!(read(["gevrek", "show-desktop", "sometimes"]).is_err());
        assert_eq!(CliSwitch::On.argument(), "on");
        assert_eq!(CliSwitch::Off.argument(), "off");
        assert_eq!(CliSwitch::Toggle.argument(), "");
        assert!(matches!(
            read(["gevrek", "toggle-mouse-follows-focus"])
                .unwrap()
                .command,
            Commands::ToggleMouseFollowsFocus
        ));
    }

    #[test]
    fn layout_depth_commands_parse() {
        assert!(matches!(
            read(["gevrek", "resize-edge", "right", "increase"])
                .unwrap()
                .command,
            Commands::ResizeEdge {
                edge: CliDirection::Right,
                sizing: CliSizing::Increase,
            }
        ));
        assert!(matches!(
            read([
                "gevrek",
                "resize-axis",
                "horizontal-and-vertical",
                "decrease"
            ])
            .unwrap()
            .command,
            Commands::ResizeAxis {
                axis: CliAxis::HorizontalAndVertical,
                sizing: CliSizing::Decrease,
            }
        ));
        assert!(matches!(
            read(["gevrek", "resize-delta", "75"]).unwrap().command,
            Commands::ResizeDelta { pixels: 75 }
        ));
        assert!(read(["gevrek", "resize-axis", "diagonal", "increase"]).is_err());
        assert!(matches!(
            read(["gevrek", "flip-layout", "vertical"]).unwrap().command,
            Commands::FlipLayout {
                axis: CliAxis::Vertical,
            }
        ));
        assert!(matches!(
            read(["gevrek", "toggle-monocle"]).unwrap().command,
            Commands::ToggleMonocle
        ));
        assert!(matches!(
            read(["gevrek", "toggle-layout-lock"]).unwrap().command,
            Commands::ToggleLayoutLock
        ));
        assert!(matches!(
            read(["gevrek", "stack-all"]).unwrap().command,
            Commands::StackAll
        ));
        assert!(matches!(
            read(["gevrek", "unstack-all"]).unwrap().command,
            Commands::UnstackAll
        ));
        assert!(matches!(
            read(["gevrek", "toggle-stackbar"]).unwrap().command,
            Commands::ToggleStackbar
        ));
        assert!(matches!(
            read(["gevrek", "promote"]).unwrap().command,
            Commands::Promote
        ));
        assert!(matches!(
            read(["gevrek", "promote-focus"]).unwrap().command,
            Commands::PromoteFocus
        ));
        assert!(matches!(
            read(["gevrek", "promote-window", "down"]).unwrap().command,
            Commands::PromoteWindow {
                direction: CliDirection::Down,
            }
        ));
    }

    fn matches_command_float(command: &Commands) -> bool {
        matches!(command, Commands::Float)
    }

    fn matches_command_tile(command: &Commands) -> bool {
        matches!(command, Commands::Tile)
    }

    fn matches_command_center(command: &Commands) -> bool {
        matches!(command, Commands::Center)
    }

    fn matches_command_raise(command: &Commands) -> bool {
        matches!(command, Commands::Raise)
    }

    fn matches_command_lower(command: &Commands) -> bool {
        matches!(command, Commands::Lower)
    }

    fn matches_command_pin(command: &Commands) -> bool {
        matches!(command, Commands::Pin)
    }

    fn matches_command_unpin(command: &Commands) -> bool {
        matches!(command, Commands::Unpin)
    }

    fn matches_command_tag_clear(command: &Commands) -> bool {
        matches!(command, Commands::TagClear)
    }
}
