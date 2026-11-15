use crate::layout::{DwindleDirection, LayoutAxis, LayoutType, Sizing};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// A spatial direction used by focus and container movement commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Selects a workspace on the active window's monitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WorkspaceSelector {
    Index(usize),
    Name(String),
    Next,
    Previous,
    PreviousVisited,
    FirstEmpty,
    Relative(i32),
}

impl WorkspaceSelector {
    pub fn expression(&self) -> String {
        match self {
            Self::Index(index) => index.to_string(),
            Self::Name(name) => format!("name:{name}"),
            Self::Next => "next".to_string(),
            Self::Previous => "previous".to_string(),
            Self::PreviousVisited => "back".to_string(),
            Self::FirstEmpty => "empty".to_string(),
            Self::Relative(offset) => format!("{offset:+}"),
        }
    }
}

impl FromStr for WorkspaceSelector {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err("Workspace selector cannot be empty".to_string());
        }
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            let index = value
                .parse::<usize>()
                .map_err(|_| format!("Invalid workspace index '{value}'"))?;
            return Ok(Self::Index(index));
        }
        let lower = value.to_ascii_lowercase();
        match lower.as_str() {
            "next" => Ok(Self::Next),
            "previous" | "prev" => Ok(Self::Previous),
            "back" | "previous_visited" | "previous-visited" => Ok(Self::PreviousVisited),
            "empty" | "first_empty" | "first-empty" => Ok(Self::FirstEmpty),
            _ => {
                if lower.starts_with("name:") {
                    let name = &value[5..];
                    if name.trim().is_empty() {
                        Err("Named workspace selector requires a name".to_string())
                    } else {
                        Ok(Self::Name(name.trim().to_string()))
                    }
                } else if lower.starts_with("relative:") {
                    let offset = &value[9..];
                    offset
                        .parse::<i32>()
                        .map(Self::Relative)
                        .map_err(|_| format!("Invalid relative workspace offset '{offset}'"))
                } else if (value.starts_with('+') || value.starts_with('-')) && value.len() > 1 {
                    value
                        .parse::<i32>()
                        .map(Self::Relative)
                        .map_err(|_| format!("Invalid relative workspace offset '{value}'"))
                } else {
                    Err(format!(
                        "Unknown workspace selector '{value}'; use an index, name:<name>, next, previous, back, empty, or a signed offset"
                    ))
                }
            }
        }
    }
}

/// Selects a monitor for multi-monitor dispatch commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MonitorTarget {
    Primary,
    Index(usize),
    Name(String),
    Next,
    Previous,
}

impl MonitorTarget {
    pub fn expression(&self) -> String {
        match self {
            Self::Primary => "primary".to_string(),
            Self::Index(index) => index.to_string(),
            Self::Name(name) => format!("name:{name}"),
            Self::Next => "next".to_string(),
            Self::Previous => "previous".to_string(),
        }
    }
}

impl FromStr for MonitorTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err("Monitor selector cannot be empty".to_string());
        }
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            return value
                .parse::<usize>()
                .map(Self::Index)
                .map_err(|_| format!("Invalid monitor index '{value}'"));
        }
        let lower = value.to_ascii_lowercase();
        match lower.as_str() {
            "primary" => Ok(Self::Primary),
            "next" => Ok(Self::Next),
            "previous" | "prev" => Ok(Self::Previous),
            _ if lower.starts_with("name:") => {
                let name = value[5..].trim();
                if name.is_empty() {
                    Err("Named monitor selector requires a name".to_string())
                } else {
                    Ok(Self::Name(name.to_string()))
                }
            }
            _ => Err(format!(
                "Unknown monitor selector '{value}'; use primary, an index, name:<name>, next, or previous"
            )),
        }
    }
}

/// Selects a managed window for targeted dispatch commands.
///
/// Text selectors use case-insensitive substring matching by default. Prefix
/// the value with `re:` for a case-insensitive regular expression. Tags use
/// the same normalized exact matching as the window rule system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WindowSelector {
    Active,
    Floating,
    Tiled,
    Handle(i64),
    ProcessId(u32),
    Title(String),
    InitialTitle(String),
    Class(String),
    InitialClass(String),
    Executable(String),
    Path(String),
    Tag(String),
}

impl WindowSelector {
    pub fn expression(&self) -> String {
        match self {
            Self::Active => "active".to_string(),
            Self::Floating => "floating".to_string(),
            Self::Tiled => "tiled".to_string(),
            Self::Handle(handle) => format!("handle:{handle}"),
            Self::ProcessId(pid) => format!("pid:{pid}"),
            Self::Title(title) => format!("title:{title}"),
            Self::InitialTitle(title) => format!("initialtitle:{title}"),
            Self::Class(class) => format!("class:{class}"),
            Self::InitialClass(class) => format!("initialclass:{class}"),
            Self::Executable(executable) => format!("exe:{executable}"),
            Self::Path(path) => format!("path:{path}"),
            Self::Tag(tag) => format!("tag:{tag}"),
        }
    }
}

impl FromStr for WindowSelector {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        fn parse_text(kind: &str, value: &str) -> Result<String, String> {
            if let Some(pattern) = value.strip_prefix("re:") {
                if pattern.is_empty() {
                    return Err(format!(
                        "Window selector '{kind}:re:' requires a regular expression"
                    ));
                }
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .map_err(|error| {
                        format!(
                            "Window selector '{kind}:re:{pattern}' has an invalid regular expression: {error}"
                        )
                    })?;
            }
            Ok(value.to_string())
        }

        fn parse_handle(value: &str) -> Result<i64, String> {
            let value = value.trim();
            if let Some(hex) = value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
            {
                i64::from_str_radix(hex, 16).map_err(|_| format!("Invalid window handle '{value}'"))
            } else {
                value
                    .parse::<i64>()
                    .map_err(|_| format!("Invalid window handle '{value}'"))
            }
        }

        let value = value.trim();
        if value.is_empty() {
            return Err("Window selector cannot be empty".to_string());
        }
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            return parse_handle(value).map(Self::Handle);
        }
        match value.to_ascii_lowercase().as_str() {
            "active" | "activewindow" => return Ok(Self::Active),
            "floating" => return Ok(Self::Floating),
            "tiled" => return Ok(Self::Tiled),
            _ => {}
        }

        let (kind, raw_value) = value.split_once(':').ok_or_else(|| {
            format!(
                "Unknown window selector '{value}'; use active, floating, tiled, handle/address:<id>, pid:<id>, title/initialtitle:<text>, class/initialclass:<text>, exe:<text>, path:<text>, or tag:<tag>"
            )
        })?;
        let selector_value = raw_value.trim();
        if selector_value.is_empty() {
            return Err(format!("Window selector '{kind}:' requires a value"));
        }
        match kind.trim().to_ascii_lowercase().as_str() {
            "handle" | "address" => parse_handle(selector_value).map(Self::Handle),
            "pid" => selector_value
                .parse::<u32>()
                .map(Self::ProcessId)
                .map_err(|_| format!("Invalid process ID '{selector_value}'")),
            "title" => parse_text("title", selector_value).map(Self::Title),
            "initialtitle" | "initial_title" => {
                parse_text("initialtitle", selector_value).map(Self::InitialTitle)
            }
            "class" => parse_text("class", selector_value).map(Self::Class),
            "initialclass" | "initial_class" => {
                parse_text("initialclass", selector_value).map(Self::InitialClass)
            }
            "exe" | "executable" => parse_text("exe", selector_value).map(Self::Executable),
            "path" => parse_text("path", selector_value).map(Self::Path),
            "tag" => Ok(Self::Tag(selector_value.to_string())),
            _ => Err(format!(
                "Unknown window selector kind '{kind}'; use handle/address, pid, title, initialtitle, class, initialclass, exe, path, or tag"
            )),
        }
    }
}

/// Commands that can be dispatched independently of a node graph.
///
/// Keeping these commands typed gives the CLI, editor, hotkey nodes, and
/// protocol one shared vocabulary. New frontends should use this type instead
/// of inventing string aliases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case")]
#[serde(remote = "Self")]
pub enum DispatchCommand {
    /// Something this version does not know (written by a newer gevrek, or
    /// mistyped): kept as it was read and written back unchanged. It does
    /// nothing when run; `validate` reports it.
    #[serde(skip)]
    Unknown(serde_json::Value),
    FocusNext,
    FocusPrevious,
    FocusLast,
    FocusWindow {
        selector: WindowSelector,
    },
    /// Focus a window even when a `NoFocus`/`NoInitialFocus` rule would
    /// prevent it. The rule keeps applying to future
    /// automatic focus; this is a one-shot explicit user override.
    ForceFocusWindow {
        selector: WindowSelector,
    },
    FocusDirection {
        direction: Direction,
    },
    SwapNext,
    SwapPrevious,
    SwapDirection {
        direction: Direction,
    },
    MoveActive {
        delta_x: i32,
        delta_y: i32,
    },
    MoveWindow {
        selector: WindowSelector,
        x: i32,
        y: i32,
        relative: bool,
    },
    ResizeActive {
        delta_width: i32,
        delta_height: i32,
    },
    ResizeWindow {
        selector: WindowSelector,
        width: i32,
        height: i32,
        relative: bool,
    },
    CloseActive,
    CloseWindow {
        selector: WindowSelector,
    },
    MinimizeActive,
    ToggleMaximized,
    ToggleFullscreen,
    ToggleWindowFullscreen {
        selector: WindowSelector,
    },
    TogglePinned,
    ToggleWindowPinned {
        selector: WindowSelector,
    },
    ToggleFloating,
    ToggleWindowFloating {
        selector: WindowSelector,
    },
    SetPinned {
        enabled: bool,
    },
    SetWindowPinned {
        selector: WindowSelector,
        enabled: bool,
    },
    SetFloating {
        enabled: bool,
    },
    SetWindowFloating {
        selector: WindowSelector,
        enabled: bool,
    },
    /// Put the floating windows of the shown workspace out of the way
    /// (all but a strip of each off the display), so that the tiled
    /// windows below them can be worked on; again, or when another
    /// workspace is shown, they come back to where they were.
    ToggleFloatingAside,
    CenterActive,
    CenterWindow {
        selector: WindowSelector,
    },
    RaiseActive,
    RaiseWindow {
        selector: WindowSelector,
    },
    LowerActive,
    LowerWindow {
        selector: WindowSelector,
    },
    ClearWindowTags,
    ClearSelectedWindowTags {
        selector: WindowSelector,
    },
    AddWindowTag {
        tag: String,
    },
    AddSelectedWindowTag {
        selector: WindowSelector,
        tag: String,
    },
    RemoveWindowTag {
        tag: String,
    },
    RemoveSelectedWindowTag {
        selector: WindowSelector,
        tag: String,
    },
    ToggleWindowTag {
        tag: String,
    },
    ToggleSelectedWindowTag {
        selector: WindowSelector,
        tag: String,
    },
    EnableRule {
        name: String,
    },
    DisableRule {
        name: String,
    },
    ToggleRule {
        name: String,
    },
    SwitchWorkspaceSelector {
        selector: WorkspaceSelector,
    },
    MoveToWorkspaceSelector {
        selector: WorkspaceSelector,
    },
    MoveToWorkspaceAndFollow {
        selector: WorkspaceSelector,
    },
    MoveWindowToWorkspace {
        window: WindowSelector,
        workspace: WorkspaceSelector,
    },
    MoveWindowToWorkspaceAndFollow {
        window: WindowSelector,
        workspace: WorkspaceSelector,
    },
    RenameWorkspace {
        selector: WorkspaceSelector,
        name: String,
    },
    FocusMonitor {
        target: MonitorTarget,
    },
    FocusMonitorAtCursor,
    FocusMonitorWorkspace {
        target: MonitorTarget,
        workspace: WorkspaceSelector,
    },
    MoveWorkspaceToMonitor {
        target: MonitorTarget,
    },
    SwapActiveWorkspaces {
        target: MonitorTarget,
    },
    MoveWindowToMonitor {
        window: WindowSelector,
        target: MonitorTarget,
    },
    MoveWindowToMonitorAndFollow {
        window: WindowSelector,
        target: MonitorTarget,
    },
    Retile,
    ToggleTiling,
    ToggleWorkspaceTiling {
        selector: WorkspaceSelector,
    },
    /// Materialize a new workspace on the focused monitor now: named when
    /// `name` is given (honoring a workspace
    /// profile with that name), otherwise the next `Workspace N` index.
    NewWorkspace {
        name: Option<String>,
    },
    /// Close the focused workspace on the focused monitor when it is empty
    ///. Fails loudly while the workspace still
    /// holds windows or is the monitor's last workspace.
    CloseWorkspace,
    /// Create workspaces on the selected monitor until it has at least
    /// `count`.
    EnsureWorkspaces {
        target: MonitorTarget,
        count: usize,
    },
    /// Create a workspace named `name` on the selected monitor when none
    /// exists yet. With a layout: the workspace has it, made now or there
    /// before (a layout for a workspace by its name, whichever display
    /// and workspace have the keyboard and the pointer).
    EnsureNamedWorkspace {
        target: MonitorTarget,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<LayoutType>,
    },
    /// Toggle the focused workspace's window container behaviour between
    /// Column (new windows get their own container) and Append (new windows
    /// join the focused container).
    ToggleWindowContainerBehaviour,
    /// Toggle the window container behaviour of the selected workspace on
    /// the focused monitor.
    ToggleWorkspaceWindowContainerBehaviour {
        selector: WorkspaceSelector,
    },
    ToggleWorkspaceFloatOverride {
        selector: WorkspaceSelector,
    },
    ToggleTitleBar,
    ToggleWindowTitleBar {
        selector: WindowSelector,
    },
    ManageWindow {
        selector: WindowSelector,
    },
    UnmanageWindow {
        selector: WindowSelector,
    },
    RestoreWindows,
    ToggleMouseFollowsFocus,
    CycleNextLayout,
    CyclePreviousLayout,
    SetLayout {
        layout: LayoutType,
    },
    SetLayoutRatio {
        ratio: f32,
    },
    AdjustLayoutRatio {
        delta: f32,
    },
    SetMasterCount {
        count: usize,
    },
    AdjustMasterCount {
        delta: i32,
    },
    RotateLayout,
    FlipLayout {
        axis: LayoutAxis,
    },
    ResizeEdge {
        edge: Direction,
        sizing: Sizing,
    },
    ResizeAxis {
        axis: LayoutAxis,
        sizing: Sizing,
    },
    SetResizeDelta {
        pixels: i32,
    },
    ToggleMonocle,
    ToggleLayoutLock,
    SetLayoutGap {
        gap: i32,
    },
    AdjustLayoutGap {
        delta: i32,
    },
    PromoteToMaster,
    Promote,
    PromoteFocus,
    PromoteWindow {
        direction: Direction,
    },
    SetDwindleDirection {
        direction: DwindleDirection,
    },
    CycleDwindleDirection,
    ToggleDwindlePreserveSplit,
    TogglePseudotile,
    ToggleWindowPseudotile {
        selector: WindowSelector,
    },
    ToggleGroup,
    ToggleWindowGroup {
        selector: WindowSelector,
    },
    ToggleGroupLock,
    ToggleWindowGroupLock {
        selector: WindowSelector,
    },
    CycleGroupNext,
    CycleGroupPrevious,
    CycleWindowGroup {
        selector: WindowSelector,
        forward: bool,
    },
    FocusGroupIndex {
        index: usize,
    },
    FocusWindowGroupIndex {
        selector: WindowSelector,
        index: usize,
    },
    MoveGroupWindow {
        forward: bool,
    },
    MoveWindowInGroup {
        selector: WindowSelector,
        forward: bool,
    },
    MergeGroupNext,
    MergeGroupPrevious,
    UngroupActive,
    StackAll,
    UnstackAll,
    ToggleStackbar,
    /// Persist every workspace's resize dimensions (layout, master ratio and
    /// count, BSP split ratios) plus the session resize delta to a JSON file
    ///.
    SaveResize {
        path: String,
    },
    /// Restore resize dimensions saved by `SaveResize`, matched by monitor and workspace index.
    LoadResize {
        path: String,
    },
    /// `SaveResize` to the standard temp-file location.
    QuickSaveResize,
    /// `LoadResize` from the standard temp-file location.
    QuickLoadResize,
    /// Toggle watching the configuration file for external changes and
    /// hot-reloading on modification.
    WatchConfiguration {
        enabled: bool,
    },
    /// Enable/disable the whole transparency system at runtime. When off,
    /// every managed window renders fully
    /// opaque regardless of the configured opacity values.
    ToggleTransparency,
    /// Set the inactive-window alpha at runtime, mirrored into the live config.
    SetTransparencyAlpha {
        alpha: u8,
    },
    /// Set the global work-area offset applied to every monitor after the
    /// Windows work area and monitor reservations.
    SetGlobalWorkAreaOffset {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    /// Set the focused workspace's per-edge work-area offset, applied after
    /// the global offset.
    SetWorkspaceWorkAreaOffset {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    /// Set the focused workspace's outer padding (`gaps_out`).
    SetWorkspacePadding {
        padding: i32,
    },
    /// Adjust the focused workspace's outer padding.
    AdjustWorkspacePadding {
        delta: i32,
    },
    /// Set the focused workspace's container padding (`gaps_in`).
    SetContainerPadding {
        padding: i32,
    },
    /// Adjust the focused workspace's container padding.
    AdjustContainerPadding {
        delta: i32,
    },
    ToggleExpose,
    /// Show every workspace of every display at once, each a small
    /// picture of itself with its windows in it, or put that away again.
    /// A press on a window goes to its workspace and gives it the
    /// keyboard; a press on a workspace's empty room goes to that
    /// workspace. (A compositor shows the workspaces themselves; on
    /// Windows the shell draws them from pictures it remembers of the
    /// windows: `SHELL_WORKSPACE_OVERVIEW`.)
    WorkspaceOverview,
    /// The next press of the left mouse button picks the window under
    /// the pointer instead of reaching it: subscribers are told which
    /// (`window.picked`, with its `class` and `title`). Escape, or a
    /// press where no window is, ends it with nothing picked
    /// (`cancelled`). Asked for while it is on: ended the same way.
    PickWindow,
    /// Open (or close) the window that lists the current shortcuts.
    ShowShortcuts,
    ToggleScratchpad {
        id: String,
    },
    /// End the session: the window manager stops (on Linux, where it is the
    /// compositor, every program of the session ends with it).
    Quit,
    /// Switch the keyboard layout among those of `desktop.input`.
    KeyboardLayout {
        target: LayoutTarget,
    },
    /// Move the view along the strip of the scrolling layout by so many
    /// pixels (more than nothing: to the right, towards the strip's
    /// end), on the display the pointer is on, else the one with the
    /// keyboard. The view stays there until the keyboard goes to another
    /// column. A wheel or fingers on a touchpad send it; where only whole
    /// columns can be shown it does nothing.
    ScrollView {
        pixels: i32,
    },
    /// The next new window is to open on this display (by its name,
    /// `DP-2`), wherever the pointer and the focus are when it comes: a
    /// launcher says so before it starts what was chosen, so that the
    /// application opens on the display the launcher was shown on. Held
    /// for a little while; the first new window takes it.
    ExpectWindow {
        output: String,
    },
    /// Give every key back to gevrek from a program that asked for all of
    /// them (a virtual machine, a nested session), or hand them over again.
    ShortcutsInhibit,
    /// Answer the trial of new display settings (a new mode, a display
    /// switched on or off): keep them, or go back at once to the ones
    /// before. Without an answer they are taken back by themselves.
    OutputTrial {
        answer: TrialAnswer,
    },
    /// Something the shell does (the launcher, the lock screen, the
    /// volume): subscribers are told (`shell.command`), and the shell that
    /// is running carries it out. Without a shell nothing happens.
    Shell {
        action: ShellAction,
    },
    /// Something to tell the person at the machine (`gevrek notify`):
    /// subscribers are told (`shell.command` with the action `notify`, the
    /// title and the text), and the shell that is running shows it as a
    /// notification. Without a shell nothing happens.
    Notify {
        title: String,
        #[serde(default)]
        text: String,
    },
    /// A value for the shell's on-screen display (`gevrek shell osd`): a
    /// few words, and a level from 0 to 100 where there is one. Told and
    /// carried out as [`DispatchCommand::Notify`] is, with the action `osd`.
    Osd {
        text: String,
        #[serde(default)]
        level: Option<u8>,
    },
    /// Show the displays at a colour temperature for a few seconds, and
    /// then as they were (the night light as the configuration has it, or
    /// their own colours): to look at a temperature before it is chosen.
    /// Nothing of the configuration is changed. Asked again meanwhile,
    /// the new temperature is shown and the time begins anew.
    NightLightPreview {
        /// In kelvin: 6500 is daylight, 2500 candle light.
        temperature: u32,
        /// How long it stays (1 to 30; 4 where none is said).
        #[serde(default = "night_light_preview_seconds")]
        seconds: u32,
    },
    /// A shortcut an application asked the desktop for (the
    /// GlobalShortcuts portal): subscribers are told (`shell.command`
    /// with the action `app_shortcut`, the application, the shortcut's
    /// own name, and whether its keys went down or were let go), and
    /// the shell that is running tells the application. Bound to keys
    /// as any command is; `released` in the binding that is for the
    /// keys let go.
    AppShortcut {
        app: String,
        id: String,
        #[serde(default)]
        released: bool,
    },
    /// Start a program: the file to run and its arguments, as they are
    /// (no shell reads them). Every use starts another one.
    Exec {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    LaunchStartup {
        id: String,
    },
    StopStartup {
        id: String,
    },
    RestartStartup {
        id: String,
    },
    SnapActiveToZone {
        index: usize,
    },
    CycleActiveZoneNext,
    CycleActiveZonePrevious,
    SetBindingSubmap {
        name: String,
    },
    ResetBindingSubmap,
}

impl DispatchCommand {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Unknown(_) => "Unknown command",
            Self::FocusNext => "Focus next window",
            Self::FocusPrevious => "Focus previous window",
            Self::FocusLast => "Focus last window",
            Self::FocusWindow { .. } => "Focus selected window",
            Self::ForceFocusWindow { .. } => "Force focus selected window",
            Self::FocusDirection { .. } => "Focus window in direction",
            Self::SwapNext => "Swap with next window",
            Self::SwapPrevious => "Swap with previous window",
            Self::SwapDirection { .. } => "Swap window in direction",
            Self::MoveActive { .. } => "Move active window",
            Self::MoveWindow { .. } => "Move selected window",
            Self::ResizeActive { .. } => "Resize active window",
            Self::ResizeWindow { .. } => "Resize selected window",
            Self::CloseActive => "Close active window",
            Self::CloseWindow { .. } => "Close selected window",
            Self::MinimizeActive => "Minimize active window",
            Self::ToggleMaximized => "Toggle maximized",
            Self::ToggleFullscreen => "Toggle fullscreen",
            Self::ToggleWindowFullscreen { .. } => "Toggle selected window fullscreen",
            Self::TogglePinned => "Toggle pinned",
            Self::ToggleWindowPinned { .. } => "Toggle selected window pinned",
            Self::ToggleFloating => "Toggle floating",
            Self::ToggleWindowFloating { .. } => "Toggle selected window floating",
            Self::SetPinned { enabled: true } => "Pin active window",
            Self::SetPinned { enabled: false } => "Unpin active window",
            Self::SetWindowPinned { enabled: true, .. } => "Pin selected window",
            Self::SetWindowPinned { enabled: false, .. } => "Unpin selected window",
            Self::SetFloating { enabled: true } => "Float active window",
            Self::SetFloating { enabled: false } => "Tile active window",
            Self::SetWindowFloating { enabled: true, .. } => "Float selected window",
            Self::SetWindowFloating { enabled: false, .. } => "Tile selected window",
            Self::ToggleFloatingAside => "Floating windows out of the way, or back",
            Self::CenterActive => "Center active window",
            Self::CenterWindow { .. } => "Center selected window",
            Self::RaiseActive => "Raise active window",
            Self::RaiseWindow { .. } => "Raise selected window",
            Self::LowerActive => "Lower active window",
            Self::LowerWindow { .. } => "Lower selected window",
            Self::ClearWindowTags => "Clear window tags",
            Self::ClearSelectedWindowTags { .. } => "Clear selected window tags",
            Self::AddWindowTag { .. } => "Add window tag",
            Self::AddSelectedWindowTag { .. } => "Add tag to selected window",
            Self::RemoveWindowTag { .. } => "Remove window tag",
            Self::RemoveSelectedWindowTag { .. } => "Remove tag from selected window",
            Self::ToggleWindowTag { .. } => "Toggle window tag",
            Self::ToggleSelectedWindowTag { .. } => "Toggle selected window tag",
            Self::EnableRule { .. } => "Enable rule",
            Self::DisableRule { .. } => "Disable rule",
            Self::ToggleRule { .. } => "Toggle rule",
            Self::SwitchWorkspaceSelector { .. } => "Switch workspace",
            Self::MoveToWorkspaceSelector { .. } => "Move window to workspace silently",
            Self::MoveToWorkspaceAndFollow { .. } => "Move window to workspace and follow",
            Self::MoveWindowToWorkspace { .. } => "Move selected window to workspace silently",
            Self::MoveWindowToWorkspaceAndFollow { .. } => {
                "Move selected window to workspace and follow"
            }
            Self::RenameWorkspace { .. } => "Rename workspace",
            Self::FocusMonitor { .. } => "Focus monitor",
            Self::FocusMonitorAtCursor => "Focus monitor at cursor",
            Self::FocusMonitorWorkspace { .. } => "Focus workspace on monitor",
            Self::MoveWorkspaceToMonitor { .. } => "Move active workspace to monitor",
            Self::SwapActiveWorkspaces { .. } => "Swap active workspaces between monitors",
            Self::MoveWindowToMonitor { .. } => "Move selected window to monitor silently",
            Self::MoveWindowToMonitorAndFollow { .. } => {
                "Move selected window to monitor and follow"
            }
            Self::Retile => "Retile active workspaces",
            Self::ToggleTiling => "Toggle tiling",
            Self::ToggleWorkspaceTiling { .. } => "Toggle workspace tiling",
            Self::NewWorkspace { .. } => "Create new workspace",
            Self::CloseWorkspace => "Close focused workspace",
            Self::EnsureWorkspaces { .. } => "Ensure workspace count on monitor",
            Self::EnsureNamedWorkspace { .. } => "Ensure named workspace on monitor",
            Self::ToggleWindowContainerBehaviour => "Toggle window container behaviour",
            Self::ToggleWorkspaceWindowContainerBehaviour { .. } => {
                "Toggle workspace window container behaviour"
            }
            Self::ToggleWorkspaceFloatOverride { .. } => "Toggle workspace float override",
            Self::ToggleTitleBar => "Toggle title bar",
            Self::ToggleWindowTitleBar { .. } => "Toggle selected window title bar",
            Self::ManageWindow { .. } => "Manage selected window",
            Self::UnmanageWindow { .. } => "Unmanage selected window",
            Self::RestoreWindows => "Restore hidden windows",
            Self::ToggleMouseFollowsFocus => "Toggle mouse follows focus",
            Self::CycleNextLayout => "Next layout",
            Self::CyclePreviousLayout => "Previous layout",
            Self::SetLayout { .. } => "Set layout",
            Self::SetLayoutRatio { .. } => "Set master ratio",
            Self::AdjustLayoutRatio { .. } => "Adjust master ratio",
            Self::SetMasterCount { .. } => "Set master count",
            Self::AdjustMasterCount { .. } => "Adjust master count",
            Self::RotateLayout => "Rotate layout",
            Self::FlipLayout { .. } => "Flip layout",
            Self::ResizeEdge { .. } => "Resize window edge",
            Self::ResizeAxis { .. } => "Resize window axis",
            Self::SetResizeDelta { .. } => "Set resize delta",
            Self::ToggleMonocle => "Toggle monocle",
            Self::ToggleLayoutLock => "Toggle layout resize lock",
            Self::SetLayoutGap { .. } => "Set layout gap",
            Self::AdjustLayoutGap { .. } => "Adjust layout gap",
            Self::PromoteToMaster => "Promote to master",
            Self::Promote => "Promote window to front",
            Self::PromoteFocus => "Focus the front container",
            Self::PromoteWindow { .. } => "Promote window in direction",
            Self::SetDwindleDirection { .. } => "Set Dwindle split policy",
            Self::CycleDwindleDirection => "Cycle Dwindle split policy",
            Self::ToggleDwindlePreserveSplit => "Toggle Dwindle preserve split",
            Self::TogglePseudotile => "Toggle pseudotile",
            Self::ToggleWindowPseudotile { .. } => "Toggle selected window pseudotile",
            Self::ToggleGroup => "Toggle group",
            Self::ToggleWindowGroup { .. } => "Toggle selected window group",
            Self::ToggleGroupLock => "Toggle group lock",
            Self::ToggleWindowGroupLock { .. } => "Toggle selected window group lock",
            Self::CycleGroupNext => "Next group tab",
            Self::CycleGroupPrevious => "Previous group tab",
            Self::CycleWindowGroup { forward: true, .. } => "Next selected group tab",
            Self::CycleWindowGroup { forward: false, .. } => "Previous selected group tab",
            Self::FocusGroupIndex { .. } => "Focus group tab by index",
            Self::FocusWindowGroupIndex { .. } => "Focus selected group tab by index",
            Self::MoveGroupWindow { forward: true } => "Move group tab forward",
            Self::MoveGroupWindow { forward: false } => "Move group tab backward",
            Self::MoveWindowInGroup { forward: true, .. } => {
                "Move selected window forward in group"
            }
            Self::MoveWindowInGroup { forward: false, .. } => {
                "Move selected window backward in group"
            }
            Self::MergeGroupNext => "Merge with next group",
            Self::MergeGroupPrevious => "Merge with previous group",
            Self::UngroupActive => "Remove from group",
            Self::StackAll => "Stack all windows",
            Self::UnstackAll => "Unstack all windows",
            Self::ToggleStackbar => "Toggle stackbar",
            Self::SaveResize { .. } => "Save resize dimensions",
            Self::LoadResize { .. } => "Load resize dimensions",
            Self::QuickSaveResize => "Quick-save resize dimensions",
            Self::QuickLoadResize => "Quick-load resize dimensions",
            Self::WatchConfiguration { enabled: true } => "Enable configuration watch",
            Self::WatchConfiguration { enabled: false } => "Disable configuration watch",
            Self::ToggleTransparency => "Toggle transparency",
            Self::SetTransparencyAlpha { .. } => "Set transparency alpha",
            Self::SetGlobalWorkAreaOffset { .. } => "Set global work-area offset",
            Self::SetWorkspaceWorkAreaOffset { .. } => "Set workspace work-area offset",
            Self::SetWorkspacePadding { .. } => "Set workspace padding",
            Self::AdjustWorkspacePadding { .. } => "Adjust workspace padding",
            Self::SetContainerPadding { .. } => "Set container padding",
            Self::AdjustContainerPadding { .. } => "Adjust container padding",
            Self::ToggleExpose => "Toggle overview",
            Self::WorkspaceOverview => "Overview of every workspace",
            Self::PickWindow => "Pick a window by clicking it",
            Self::ShowShortcuts => "Show shortcuts",
            Self::ToggleScratchpad { .. } => "Toggle scratchpad",
            Self::Quit => "Quit the session",
            Self::KeyboardLayout { .. } => "Switch keyboard layout",
            Self::ExpectWindow { .. } => "The next window opens on a display",
            Self::ScrollView { .. } => "Scroll along the strip",
            Self::ShortcutsInhibit => "Take the keys back / hand them over",
            Self::OutputTrial {
                answer: TrialAnswer::Keep,
            } => "Keep the new display settings",
            Self::OutputTrial {
                answer: TrialAnswer::Revert,
            } => "Take the new display settings back",
            Self::Shell { action } => action.display_name(),
            Self::Notify { .. } => "Show a notification",
            Self::Osd { .. } => "Show a value on the screen",
            Self::NightLightPreview { .. } => "Show a colour temperature for a moment",
            Self::AppShortcut { .. } => "An application's own shortcut",
            Self::Exec { .. } => "Start a program",
            Self::LaunchStartup { .. } => "Launch startup application",
            Self::StopStartup { .. } => "Stop startup application",
            Self::RestartStartup { .. } => "Restart startup application",
            Self::SnapActiveToZone { .. } => "Snap active window to zone",
            Self::CycleActiveZoneNext => "Snap active window to next zone",
            Self::CycleActiveZonePrevious => "Snap active window to previous zone",
            Self::SetBindingSubmap { .. } => "Enter binding submap",
            Self::ResetBindingSubmap => "Return to default binding submap",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_have_stable_tagged_json() {
        let command = DispatchCommand::FocusDirection {
            direction: Direction::Left,
        };
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(json, r#"{"command":"focus_direction","direction":"left"}"#);
        assert_eq!(
            serde_json::from_str::<DispatchCommand>(&json).unwrap(),
            command
        );
    }

    #[test]
    fn parameterized_commands_round_trip() {
        let commands = [
            DispatchCommand::MoveActive {
                delta_x: -20,
                delta_y: 15,
            },
            DispatchCommand::ResizeActive {
                delta_width: 80,
                delta_height: -40,
            },
            DispatchCommand::FocusLast,
            DispatchCommand::FocusWindow {
                selector: WindowSelector::Class("WindowsTerminal".to_string()),
            },
            DispatchCommand::ForceFocusWindow {
                selector: WindowSelector::Active,
            },
            DispatchCommand::MoveWindow {
                selector: WindowSelector::Tag("scratch".to_string()),
                x: 120,
                y: 80,
                relative: false,
            },
            DispatchCommand::ResizeWindow {
                selector: WindowSelector::Executable("notepad.exe".to_string()),
                width: 900,
                height: 600,
                relative: false,
            },
            DispatchCommand::CloseWindow {
                selector: WindowSelector::Title("Temporary".to_string()),
            },
            DispatchCommand::ToggleWindowPinned {
                selector: WindowSelector::Tag("overlay".to_string()),
            },
            DispatchCommand::ToggleWindowFloating {
                selector: WindowSelector::Tag("scratch".to_string()),
            },
            DispatchCommand::SetWindowPinned {
                selector: WindowSelector::Class("Tool".to_string()),
                enabled: true,
            },
            DispatchCommand::SetWindowFloating {
                selector: WindowSelector::Class("Dialog".to_string()),
                enabled: true,
            },
            DispatchCommand::CenterWindow {
                selector: WindowSelector::Tag("scratch".to_string()),
            },
            DispatchCommand::RaiseWindow {
                selector: WindowSelector::Tag("overlay".to_string()),
            },
            DispatchCommand::LowerWindow {
                selector: WindowSelector::Tag("desktop".to_string()),
            },
            DispatchCommand::ToggleWindowFullscreen {
                selector: WindowSelector::Class("Presentation".to_string()),
            },
            DispatchCommand::ToggleWindowPseudotile {
                selector: WindowSelector::Tag("compact".to_string()),
            },
            DispatchCommand::ToggleWindowGroup {
                selector: WindowSelector::Tag("tabs".to_string()),
            },
            DispatchCommand::ToggleWindowGroupLock {
                selector: WindowSelector::ProcessId(4242),
            },
            DispatchCommand::CycleWindowGroup {
                selector: WindowSelector::Class("Browser".to_string()),
                forward: false,
            },
            DispatchCommand::FocusGroupIndex { index: 2 },
            DispatchCommand::FocusWindowGroupIndex {
                selector: WindowSelector::Tag("tabs".to_string()),
                index: 1,
            },
            DispatchCommand::MoveGroupWindow { forward: true },
            DispatchCommand::MoveWindowInGroup {
                selector: WindowSelector::Executable("code.exe".to_string()),
                forward: false,
            },
            DispatchCommand::ClearSelectedWindowTags {
                selector: WindowSelector::ProcessId(4242),
            },
            DispatchCommand::AddSelectedWindowTag {
                selector: WindowSelector::Class("Dialog".to_string()),
                tag: "urgent".to_string(),
            },
            DispatchCommand::RemoveSelectedWindowTag {
                selector: WindowSelector::Tag("temporary".to_string()),
                tag: "temporary".to_string(),
            },
            DispatchCommand::ToggleSelectedWindowTag {
                selector: WindowSelector::Executable("slack.exe".to_string()),
                tag: "communication".to_string(),
            },
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("code".to_string()),
            },
            DispatchCommand::MoveToWorkspaceSelector {
                selector: WorkspaceSelector::Relative(1),
            },
            DispatchCommand::MoveToWorkspaceAndFollow {
                selector: WorkspaceSelector::Name("web".to_string()),
            },
            DispatchCommand::MoveWindowToWorkspace {
                window: WindowSelector::Tag("communication".to_string()),
                workspace: WorkspaceSelector::Name("chat".to_string()),
            },
            DispatchCommand::MoveWindowToWorkspaceAndFollow {
                window: WindowSelector::ProcessId(4242),
                workspace: WorkspaceSelector::Relative(1),
            },
            DispatchCommand::RenameWorkspace {
                selector: WorkspaceSelector::Index(0),
                name: "code".to_string(),
            },
            DispatchCommand::SetPinned { enabled: true },
            DispatchCommand::SetFloating { enabled: false },
            DispatchCommand::CenterActive,
            DispatchCommand::ToggleFloatingAside,
            DispatchCommand::RaiseActive,
            DispatchCommand::LowerActive,
            DispatchCommand::ClearWindowTags,
            DispatchCommand::FocusMonitor {
                target: MonitorTarget::Next,
            },
            DispatchCommand::FocusMonitorAtCursor,
            DispatchCommand::FocusMonitorWorkspace {
                target: MonitorTarget::Index(1),
                workspace: WorkspaceSelector::Name("code".to_string()),
            },
            DispatchCommand::MoveWorkspaceToMonitor {
                target: MonitorTarget::Name("Side display".to_string()),
            },
            DispatchCommand::SwapActiveWorkspaces {
                target: MonitorTarget::Previous,
            },
            DispatchCommand::MoveWindowToMonitor {
                window: WindowSelector::Active,
                target: MonitorTarget::Next,
            },
            DispatchCommand::MoveWindowToMonitorAndFollow {
                window: WindowSelector::Handle(1234),
                target: MonitorTarget::Name("Side display".to_string()),
            },
            DispatchCommand::Retile,
            DispatchCommand::ToggleTiling,
            DispatchCommand::NewWorkspace {
                name: Some("mail".to_string()),
            },
            DispatchCommand::NewWorkspace { name: None },
            DispatchCommand::CloseWorkspace,
            DispatchCommand::EnsureWorkspaces {
                target: MonitorTarget::Name("Side display".to_string()),
                count: 3,
            },
            DispatchCommand::EnsureNamedWorkspace {
                target: MonitorTarget::Primary,
                name: "chat".to_string(),
                layout: None,
            },
            DispatchCommand::ToggleWindowContainerBehaviour,
            DispatchCommand::ToggleWorkspaceWindowContainerBehaviour {
                selector: WorkspaceSelector::Index(2),
            },
            DispatchCommand::ToggleWorkspaceTiling {
                selector: WorkspaceSelector::Name("code".to_string()),
            },
            DispatchCommand::ToggleWorkspaceFloatOverride {
                selector: WorkspaceSelector::Index(1),
            },
            DispatchCommand::ToggleTitleBar,
            DispatchCommand::ToggleWindowTitleBar {
                selector: WindowSelector::Class("Notepad".to_string()),
            },
            DispatchCommand::ManageWindow {
                selector: WindowSelector::Handle(4212),
            },
            DispatchCommand::UnmanageWindow {
                selector: WindowSelector::Active,
            },
            DispatchCommand::RestoreWindows,
            DispatchCommand::ToggleMouseFollowsFocus,
            DispatchCommand::SetLayout {
                layout: LayoutType::CenteredMaster,
            },
            DispatchCommand::FlipLayout {
                axis: LayoutAxis::HorizontalAndVertical,
            },
            DispatchCommand::ResizeEdge {
                edge: Direction::Right,
                sizing: Sizing::Increase,
            },
            DispatchCommand::ResizeAxis {
                axis: LayoutAxis::Vertical,
                sizing: Sizing::Decrease,
            },
            DispatchCommand::SetResizeDelta { pixels: 75 },
            DispatchCommand::ToggleMonocle,
            DispatchCommand::ToggleLayoutLock,
            DispatchCommand::StackAll,
            DispatchCommand::UnstackAll,
            DispatchCommand::ToggleStackbar,
            DispatchCommand::SaveResize {
                path: "C:\\layouts\\main.json".to_string(),
            },
            DispatchCommand::LoadResize {
                path: "snapshots/resume.json".to_string(),
            },
            DispatchCommand::QuickSaveResize,
            DispatchCommand::QuickLoadResize,
            DispatchCommand::WatchConfiguration { enabled: true },
            DispatchCommand::ToggleTransparency,
            DispatchCommand::SetTransparencyAlpha { alpha: 180 },
            DispatchCommand::SetGlobalWorkAreaOffset {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            },
            DispatchCommand::SetWorkspaceWorkAreaOffset {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4,
            },
            DispatchCommand::SetWorkspacePadding { padding: 12 },
            DispatchCommand::AdjustWorkspacePadding { delta: -4 },
            DispatchCommand::SetContainerPadding { padding: 6 },
            DispatchCommand::AdjustContainerPadding { delta: 2 },
            DispatchCommand::Promote,
            DispatchCommand::PromoteFocus,
            DispatchCommand::PromoteWindow {
                direction: Direction::Down,
            },
            DispatchCommand::AdjustLayoutRatio { delta: -0.05 },
            DispatchCommand::SetMasterCount { count: 2 },
            DispatchCommand::SetDwindleDirection {
                direction: DwindleDirection::Vertical,
            },
            DispatchCommand::TogglePseudotile,
            DispatchCommand::MergeGroupNext,
            DispatchCommand::ToggleScratchpad {
                id: "terminal".to_string(),
            },
            DispatchCommand::RestartStartup {
                id: "status-bar".to_string(),
            },
            DispatchCommand::SnapActiveToZone { index: 2 },
            DispatchCommand::CycleActiveZonePrevious,
            DispatchCommand::SetBindingSubmap {
                name: "resize".to_string(),
            },
            DispatchCommand::ResetBindingSubmap,
            DispatchCommand::ToggleWindowTag {
                tag: "communication".to_string(),
            },
            DispatchCommand::DisableRule {
                name: "Float picture-in-picture".to_string(),
            },
        ];

        for command in commands {
            let json = serde_json::to_string(&command).unwrap();
            assert_eq!(
                serde_json::from_str::<DispatchCommand>(&json).unwrap(),
                command
            );
        }
    }

    #[test]
    fn window_selectors_parse_and_round_trip_expressions() {
        let selectors = [
            ("active", WindowSelector::Active),
            ("floating", WindowSelector::Floating),
            ("tiled", WindowSelector::Tiled),
            ("42", WindowSelector::Handle(42)),
            ("handle:42", WindowSelector::Handle(42)),
            ("address:0x2A", WindowSelector::Handle(42)),
            ("pid:1234", WindowSelector::ProcessId(1234)),
            (
                "title:Project: Gevrek",
                WindowSelector::Title("Project: Gevrek".to_string()),
            ),
            (
                "initialtitle:Project: Gevrek",
                WindowSelector::InitialTitle("Project: Gevrek".to_string()),
            ),
            (
                "class:WindowsTerminal",
                WindowSelector::Class("WindowsTerminal".to_string()),
            ),
            (
                "class:re:^(WindowsTerminal|CASCADIA_HOSTING_WINDOW_CLASS)$",
                WindowSelector::Class(
                    "re:^(WindowsTerminal|CASCADIA_HOSTING_WINDOW_CLASS)$".to_string(),
                ),
            ),
            (
                "initialclass:WindowsTerminal",
                WindowSelector::InitialClass("WindowsTerminal".to_string()),
            ),
            (
                "exe:code.exe",
                WindowSelector::Executable("code.exe".to_string()),
            ),
            (
                "path:C:\\Apps\\Editor.exe",
                WindowSelector::Path("C:\\Apps\\Editor.exe".to_string()),
            ),
            ("tag:dev", WindowSelector::Tag("dev".to_string())),
        ];
        for (expression, expected) in selectors {
            assert_eq!(expression.parse::<WindowSelector>().unwrap(), expected);
            assert_eq!(
                expected
                    .expression()
                    .parse::<WindowSelector>()
                    .expect("generated expression should parse"),
                expected
            );
        }
    }

    #[test]
    fn window_selectors_reject_missing_or_unknown_values() {
        assert!("".parse::<WindowSelector>().is_err());
        assert!("title:".parse::<WindowSelector>().is_err());
        assert!("mystery:value".parse::<WindowSelector>().is_err());
        assert!("handle:not-a-number".parse::<WindowSelector>().is_err());
        assert!("title:re:".parse::<WindowSelector>().is_err());
        assert!("class:re:(".parse::<WindowSelector>().is_err());
    }

    #[test]
    fn workspace_selector_expressions_are_unambiguous() {
        let cases = [
            ("3", WorkspaceSelector::Index(3)),
            ("name:code", WorkspaceSelector::Name("code".to_string())),
            ("next", WorkspaceSelector::Next),
            ("prev", WorkspaceSelector::Previous),
            ("back", WorkspaceSelector::PreviousVisited),
            ("empty", WorkspaceSelector::FirstEmpty),
            ("+2", WorkspaceSelector::Relative(2)),
            ("relative:-3", WorkspaceSelector::Relative(-3)),
            ("NAME:Code", WorkspaceSelector::Name("Code".to_string())),
            ("RELATIVE:4", WorkspaceSelector::Relative(4)),
        ];
        for (expression, expected) in cases {
            assert_eq!(expression.parse::<WorkspaceSelector>().unwrap(), expected);
        }
        assert!("name:".parse::<WorkspaceSelector>().is_err());
    }

    #[test]
    fn monitor_target_expressions_are_unambiguous() {
        let cases = [
            ("primary", MonitorTarget::Primary),
            ("1", MonitorTarget::Index(1)),
            (
                "name:Side display",
                MonitorTarget::Name("Side display".to_string()),
            ),
            ("next", MonitorTarget::Next),
            ("prev", MonitorTarget::Previous),
        ];
        for (expression, expected) in cases {
            assert_eq!(expression.parse::<MonitorTarget>().unwrap(), expected);
        }
        assert!("name:".parse::<MonitorTarget>().is_err());
    }
}

/// What to do with display settings that are on trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrialAnswer {
    Keep,
    Revert,
}

/// Which keyboard layout to switch to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum LayoutTarget {
    Next,
    Previous,
    /// The layout at this place of the list, from 0.
    Index(u32),
}

fn night_light_preview_seconds() -> u32 {
    4
}

/// What a shell is asked to do. The words are the same on every system;
/// what a system's shell cannot do it ignores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellAction {
    Launcher,
    ControlCenter,
    NotificationCenter,
    Lock,
    PowerMenu,
    Logout,
    ScreenshotScreen,
    ScreenshotWindow,
    ScreenshotRegion,
    /// Begin a film of the display with the keyboard; the same again
    /// ends it and says where the file is.
    RecordScreen,
    /// The same of the window with the keyboard, wherever it is moved to.
    RecordWindow,
    /// The same of a part of the display, chosen by dragging.
    RecordRegion,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    MediaStop,
    DisplaysOff,
    DoNotDisturb,
    NightLight,
    /// The windows that are open, to go to one of them: held open by the
    /// key it was called with, gone through with Tab, left by letting go.
    WindowSwitcher,
    /// The tutorial: begun, or taken up where it was left.
    Tutorial,
    /// The tutorial's card, by keys (it never has the keyboard itself):
    /// on from a step, back to the one before, out of the tutorial.
    /// Nothing while no lesson is taken.
    TutorialNext,
    TutorialBack,
    TutorialLeave,
    /// The welcome: the window of the first launch, opened again.
    Welcome,
}

/// The action `shell.command` names for the list of shortcuts, where the
/// shell shows it (`DispatchCommand::ShowShortcuts` on Windows).
pub const SHELL_SHORTCUTS: &str = "shortcuts";

/// The action `shell.command` names for the overview of every workspace,
/// where the shell draws it (`DispatchCommand::WorkspaceOverview` on
/// Windows; a compositor shows the workspaces itself).
pub const SHELL_WORKSPACE_OVERVIEW: &str = "workspace_overview";

impl DispatchCommand {
    /// What subscribers are told (`shell.command`) for a command that asks
    /// the shell to show something said with it: the action and what goes
    /// with it. `None` for every other command.
    pub fn shell_fields(&self) -> Option<Vec<(String, String)>> {
        let pair = |name: &str, value: &str| (name.to_string(), value.to_string());
        match self {
            Self::Notify { title, text } => Some(vec![
                pair("action", "notify"),
                pair("title", title),
                pair("text", text),
            ]),
            Self::Osd { text, level } => {
                let mut fields = vec![pair("action", "osd"), pair("text", text)];
                if let Some(level) = level {
                    fields.push(pair("level", &(*level).min(100).to_string()));
                }
                Some(fields)
            }
            Self::AppShortcut { app, id, released } => Some(vec![
                pair("action", "app_shortcut"),
                pair("app", app),
                pair("id", id),
                pair("released", if *released { "true" } else { "false" }),
            ]),
            _ => None,
        }
    }
}

impl ShellAction {
    pub const ALL: [Self; 31] = [
        Self::Launcher,
        Self::ControlCenter,
        Self::NotificationCenter,
        Self::Lock,
        Self::PowerMenu,
        Self::Logout,
        Self::ScreenshotScreen,
        Self::ScreenshotWindow,
        Self::ScreenshotRegion,
        Self::RecordScreen,
        Self::RecordWindow,
        Self::RecordRegion,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::VolumeMute,
        Self::MicMute,
        Self::BrightnessUp,
        Self::BrightnessDown,
        Self::MediaPlayPause,
        Self::MediaNext,
        Self::MediaPrevious,
        Self::MediaStop,
        Self::DisplaysOff,
        Self::DoNotDisturb,
        Self::NightLight,
        Self::WindowSwitcher,
        Self::Tutorial,
        Self::TutorialNext,
        Self::TutorialBack,
        Self::TutorialLeave,
        Self::Welcome,
    ];

    /// The word the action has in the configuration and in events.
    pub fn id(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default()
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Launcher => "Launcher",
            Self::ControlCenter => "Control centre",
            Self::NotificationCenter => "Notifications",
            Self::Lock => "Lock the screen",
            Self::PowerMenu => "Power menu",
            Self::Logout => "Log out",
            Self::ScreenshotScreen => "Screenshot of the screen",
            Self::ScreenshotWindow => "Screenshot of the window",
            Self::ScreenshotRegion => "Screenshot of a region",
            Self::RecordScreen => "Record the screen (again: stop)",
            Self::RecordWindow => "Record the window (again: stop)",
            Self::RecordRegion => "Record a region (again: stop)",
            Self::VolumeUp => "Volume up",
            Self::VolumeDown => "Volume down",
            Self::VolumeMute => "Mute",
            Self::MicMute => "Mute the microphone",
            Self::BrightnessUp => "Brightness up",
            Self::BrightnessDown => "Brightness down",
            Self::MediaPlayPause => "Play / pause",
            Self::MediaNext => "Next track",
            Self::MediaPrevious => "Previous track",
            Self::MediaStop => "Stop playing",
            Self::DisplaysOff => "Displays off",
            Self::DoNotDisturb => "Do not disturb",
            Self::NightLight => "Night light",
            Self::WindowSwitcher => "Window switcher",
            Self::Tutorial => "The tutorial",
            Self::TutorialNext => "Tutorial: on",
            Self::TutorialBack => "Tutorial: back",
            Self::TutorialLeave => "Tutorial: leave",
            Self::Welcome => "The welcome",
        }
    }
}

/// A command line as one text, for showing and editing an [`DispatchCommand::Exec`]:
/// the parts separated by spaces, a part with a space or a quote in it
/// inside double quotes (`\"` for a quote).
pub fn join_command_line(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.is_empty() || part.contains([' ', '\t', '"']) {
                format!("\"{}\"", part.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                part.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The parts of a command line written as [`join_command_line`] writes it.
pub fn split_command_line(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = characters.next() {
                    part.push(next);
                }
            }
            ' ' | '\t' if !quoted => {
                if started {
                    parts.push(std::mem::take(&mut part));
                    started = false;
                }
            }
            other => {
                part.push(other);
                started = true;
            }
        }
    }
    if started {
        parts.push(part);
    }
    parts
}

#[cfg(test)]
mod command_line_tests {
    use super::*;

    #[test]
    fn a_command_line_survives_being_written_and_read() {
        for parts in [
            vec!["kitty"],
            vec!["flatpak", "run", "io.missioncenter.MissionCenter"],
            vec!["sh", "-c", "echo \"two words\" > /tmp/x"],
            vec!["C:\\Program Files\\App\\app.exe", "", "--flag"],
        ] {
            let parts: Vec<String> = parts.into_iter().map(str::to_string).collect();
            assert_eq!(split_command_line(&join_command_line(&parts)), parts);
        }
        assert_eq!(
            split_command_line("  kitty   --hold  "),
            ["kitty", "--hold"]
        );
        assert!(split_command_line("   ").is_empty());
    }

    #[test]
    fn what_the_shell_is_to_show_is_told_with_its_words() {
        let notify: DispatchCommand =
            serde_json::from_str(r#"{"command":"notify","title":"Done"}"#).unwrap();
        assert_eq!(
            notify,
            DispatchCommand::Notify {
                title: "Done".to_string(),
                text: String::new()
            }
        );
        let fields = notify.shell_fields().expect("told to the shell");
        assert_eq!(fields[0], ("action".to_string(), "notify".to_string()));
        assert_eq!(fields[1], ("title".to_string(), "Done".to_string()));
        let osd: DispatchCommand =
            serde_json::from_str(r#"{"command":"osd","text":"Volume","level":250}"#).unwrap();
        let fields = osd.shell_fields().expect("told to the shell");
        assert!(fields.contains(&("action".to_string(), "osd".to_string())));
        // A level is never more than all of it.
        assert!(fields.contains(&("level".to_string(), "100".to_string())));
        let bare = DispatchCommand::Osd {
            text: "Grid".to_string(),
            level: None,
        };
        assert!(!bare
            .shell_fields()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "level"));
        assert!(DispatchCommand::Quit.shell_fields().is_none());
    }

    #[test]
    fn exec_is_written_as_program_and_args() {
        let command = DispatchCommand::Exec {
            program: "kitty".to_string(),
            args: vec!["--hold".to_string()],
        };
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(
            json,
            r#"{"command":"exec","program":"kitty","args":["--hold"]}"#
        );
        let short: DispatchCommand =
            serde_json::from_str(r#"{"command":"exec","program":"kitty"}"#).unwrap();
        assert_eq!(
            short,
            DispatchCommand::Exec {
                program: "kitty".to_string(),
                args: vec![]
            }
        );
    }
}

crate::tolerant_enum!(DispatchCommand);
