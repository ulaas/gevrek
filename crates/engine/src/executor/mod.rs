//! The executor: the automation graph's interpreter and the window-management
//! logic that is not yet split into the modules beside it. It holds the
//! per-window state, the rule metadata, the scratchpads and the graph, and
//! acts on the desktop only through `contract::Desktop`.

use crate::commands::{self, LayoutScope};
use crate::constraints::{self, resolve_rule_rect, PersistentSizes};
use crate::contract::{
    BorderChange, Desktop, Effect, PlatformEffect, StackOp, StackbarContainerState, StateChange,
    WindowStates, WindowsEffect,
};
use crate::decoration::stackbar_content_rect;
use crate::events::WindowEvent;
use crate::expose::{self, AfterDestroy};
use crate::focus::{self, FocusDecision, InitialFocus};
use crate::focus_manager::FocusManager;
use crate::groups;
use crate::layouts::{EdgeDirection, LayoutAxis, Sizing};
use crate::lifecycle::{self, Adoption, Changes, Minimize, Placement, Removal, Restore, World};
use crate::outputs::{MonitorHandle, Outputs as MonitorManager};
use crate::persistence::PersistentWindowSize;
use crate::rule_state::{
    DragState, DynamicRuleBaseline, DynamicRuleState, PersistentSizeKey, RuleSize, WindowStateMap,
};
use crate::rules::{RuleResolution, RuleResolutionScope};
use crate::scratchpads::{self, Scratchpads, Toggle};
use crate::workspace_profile::WorkspaceProfileApply;
use crate::workspaces;
use crate::zones::ZoneManager;
use crate::WindowHandle;
use anyhow::Result;
use gevrek_model::command::{
    Direction, DispatchCommand, MonitorTarget, WindowSelector, WorkspaceSelector,
};
use gevrek_model::graph::{
    AnimationType, BorderStyle, LogEntry, NodeGraph, NodeId, NodeType, PinId, RuleDefinition,
    TileLayout, WindowMode,
};
use gevrek_model::settings::{
    AppearanceProfile, LaunchProfile, MonitorSelector, ScratchpadProfile, WorkspaceProfile,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod commands_focus;
mod commands_group;
mod commands_layout;
mod commands_other;
mod commands_rules;
mod commands_window;
mod commands_workspace;
mod dispatch;
mod events;
mod graph;
mod layout;
mod nodes_events;
mod nodes_focus;
mod nodes_layout;
mod nodes_query;
mod nodes_rules;
mod nodes_scratchpad;
mod nodes_value;
mod nodes_window;
mod nodes_workspace;
mod rules;
mod scratchpad;
mod window;
mod workspace;

/// How a node's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeDone {
    /// It ran to its end.
    Normal,
    /// It stopped early (its inputs were missing or its target is gone).
    Early,
}

/// Upper bound for dynamically created workspaces per monitor ring.
const MAX_DYNAMIC_WORKSPACES: usize = 128;

#[derive(Debug, Clone)]
pub enum Value {
    Execution,
    WindowHandle(WindowHandle),
    WindowList(Vec<WindowHandle>),
    Number(f64),
    String(String),
    Bool(bool),
    Color(u8, u8, u8),                          // RGB values (0-255 each)
    BorderStyle(BorderStyle),                   // Square, Rounded
    Monitor(MonitorHandle),                     // Single monitor reference
    MonitorList(Vec<MonitorHandle>),            // List of monitors
    Layout(TileLayout),                         // Workspace layout type
    Rule(RuleDefinition),                       // Single rule definition
    RuleList(Vec<RuleDefinition>),              // List of rule definitions
    MatchKind(crate::rules::MatchKind),         // Window matching kind (Title, Class, Exe)
    MatchStrategy(crate::rules::MatchStrategy), // Window matching strategy (Equals, Contains, etc.)
    None,
}

/// Pending focus operation to be applied by the Runtime
#[derive(Debug, Clone, Copy)]
pub enum PendingFocusOp {
    CycleNext,
    CyclePrevious,
}

#[derive(Debug, Clone)]
pub enum NodeExecutionEvent {
    Started {
        node_id: NodeId,
        node_type: NodeType,
        timestamp_ms: i64,
    },
    Completed {
        node_id: NodeId,
        node_type: NodeType,
        timestamp_ms: i64,
        duration_ms: i64,
    },
    Failed {
        node_id: NodeId,
        node_type: NodeType,
        timestamp_ms: i64,
        error: String,
    },
    Runtime(RuntimeExtensionEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEventKind {
    RuntimeStarted,
    RuntimeStopping,
    ConfigurationReloaded,
    BindingTriggered,
    GestureStarted,
    GestureUpdated,
    GestureCompleted,
    GestureCancelled,
    DispatchSucceeded,
    DispatchFailed,
    WindowCreated,
    WindowDestroyed,
    WindowFocused,
    WindowMoved,
    WindowResized,
    WindowDragStarted,
    WindowDragEnded,
    WindowMinimized,
    WindowRestored,
    WindowMetadataChanged,
    WindowUnmapped,
    WorkspaceCreated,
    WorkspaceDestroyed,
    WorkspaceRenamed,
    /// Another workspace is shown on an output.
    WorkspaceSwitched,
    /// The layout of the workspace an output shows changed.
    LayoutChanged,
    OutputAdded,
    OutputRemoved,
    /// An output's place, size, scale or usable area changed.
    OutputChanged,
    /// New display settings are on trial (`state` is `started`, with the
    /// `seconds` left to keep them), or the trial is over (`kept`,
    /// `reverted`).
    OutputTrial,
    /// The shell is asked to do something (`DispatchCommand::Shell`).
    ShellCommand,
    /// Another keyboard layout is in use (by a command, the panel, or
    /// the layout's own keys).
    KeyboardLayoutChanged,
    /// What the window manager has to tell the person is another than it
    /// was (a notice arose, or holds no longer); the state names them.
    NoticesChanged,
    /// A mouse button was pressed on a window or on the bare desktop
    /// (not on the shell's own surfaces). What the shell has open
    /// beside the windows without holding the keyboard (the control
    /// centre, the notifications) is put away by it.
    DesktopPressed,
    /// A window was picked by a press on it (`DispatchCommand::PickWindow`):
    /// the window, with its `class` and `title`; or nothing was
    /// (`cancelled`).
    WindowPicked,
}

impl RuntimeEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeStarted => "runtime.started",
            Self::RuntimeStopping => "runtime.stopping",
            Self::ConfigurationReloaded => "configuration.reloaded",
            Self::BindingTriggered => "binding.triggered",
            Self::GestureStarted => "gesture.started",
            Self::GestureUpdated => "gesture.updated",
            Self::GestureCompleted => "gesture.completed",
            Self::GestureCancelled => "gesture.cancelled",
            Self::DispatchSucceeded => "dispatch.succeeded",
            Self::DispatchFailed => "dispatch.failed",
            Self::WindowCreated => "window.created",
            Self::WindowDestroyed => "window.destroyed",
            Self::WindowFocused => "window.focused",
            Self::WindowMoved => "window.moved",
            Self::WindowResized => "window.resized",
            Self::WindowDragStarted => "window.drag_started",
            Self::WindowDragEnded => "window.drag_ended",
            Self::WindowMinimized => "window.minimized",
            Self::WindowRestored => "window.restored",
            Self::WindowMetadataChanged => "window.metadata_changed",
            Self::WindowUnmapped => "window.unmapped",
            Self::WorkspaceCreated => "workspace.created",
            Self::WorkspaceDestroyed => "workspace.destroyed",
            Self::WorkspaceRenamed => "workspace.renamed",
            Self::WorkspaceSwitched => "workspace.switched",
            Self::LayoutChanged => "layout.changed",
            Self::OutputAdded => "output.added",
            Self::OutputRemoved => "output.removed",
            Self::OutputChanged => "output.changed",
            Self::OutputTrial => "output.trial",
            Self::ShellCommand => "shell.command",
            Self::KeyboardLayoutChanged => "keyboard.layout_changed",
            Self::NoticesChanged => "notices.changed",
            Self::DesktopPressed => "desktop.pressed",
            Self::WindowPicked => "window.picked",
        }
    }
}

impl RuntimeEventKind {
    /// Every kind, for lists and tests.
    pub const ALL: [Self; 35] = [
        Self::RuntimeStarted,
        Self::RuntimeStopping,
        Self::ConfigurationReloaded,
        Self::BindingTriggered,
        Self::GestureStarted,
        Self::GestureUpdated,
        Self::GestureCompleted,
        Self::GestureCancelled,
        Self::DispatchSucceeded,
        Self::DispatchFailed,
        Self::WindowCreated,
        Self::WindowDestroyed,
        Self::WindowFocused,
        Self::WindowMoved,
        Self::WindowResized,
        Self::WindowDragStarted,
        Self::WindowDragEnded,
        Self::WindowMinimized,
        Self::WindowRestored,
        Self::WindowMetadataChanged,
        Self::WindowUnmapped,
        Self::WorkspaceCreated,
        Self::WorkspaceDestroyed,
        Self::WorkspaceRenamed,
        Self::WorkspaceSwitched,
        Self::LayoutChanged,
        Self::OutputAdded,
        Self::OutputRemoved,
        Self::OutputChanged,
        Self::OutputTrial,
        Self::ShellCommand,
        Self::KeyboardLayoutChanged,
        Self::NoticesChanged,
        Self::DesktopPressed,
        Self::WindowPicked,
    ];

    /// The event subscribers are told of for a window event of the
    /// desktop: its kind, the window, and its fields. The same on every
    /// backend.
    pub fn of_window_event(
        event: &crate::events::WindowEvent,
    ) -> (Self, WindowHandle, Vec<(String, String)>) {
        use crate::events::WindowEvent;
        match event {
            WindowEvent::Created(handle) => (Self::WindowCreated, *handle, vec![]),
            WindowEvent::Destroyed(handle) => (Self::WindowDestroyed, *handle, vec![]),
            WindowEvent::Focused(handle) => (Self::WindowFocused, *handle, vec![]),
            WindowEvent::Moved(handle, x, y) => (
                Self::WindowMoved,
                *handle,
                vec![
                    ("x".to_string(), x.to_string()),
                    ("y".to_string(), y.to_string()),
                ],
            ),
            WindowEvent::Resized(handle, width, height) => (
                Self::WindowResized,
                *handle,
                vec![
                    ("width".to_string(), width.to_string()),
                    ("height".to_string(), height.to_string()),
                ],
            ),
            WindowEvent::DragStarted(handle) => (Self::WindowDragStarted, *handle, vec![]),
            WindowEvent::DragEnded(handle) => (Self::WindowDragEnded, *handle, vec![]),
            WindowEvent::Minimized(handle) => (Self::WindowMinimized, *handle, vec![]),
            WindowEvent::Restored(handle) => (Self::WindowRestored, *handle, vec![]),
            WindowEvent::MetadataChanged(handle) => (Self::WindowMetadataChanged, *handle, vec![]),
            WindowEvent::Unmapped(handle) => (Self::WindowUnmapped, *handle, vec![]),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeExtensionEvent {
    pub kind: RuntimeEventKind,
    pub timestamp_ms: i64,
    pub window_handle: Option<WindowHandle>,
    pub fields: std::collections::BTreeMap<String, String>,
}

pub use crate::scratchpads::ScratchpadRuntimeState;
pub use workspace::{WorkspaceBeside, WorkspaceView};

/// What is called with each line the graph logs, for whoever listens
/// (the studio): the words, how grave they are, and when.
pub type LogBroadcast = Arc<dyn Fn(String, gevrek_model::graph::LogLevel, i64) + Send + Sync>;
/// What is called as each node of the graph is run.
pub type NodeEventBroadcast = Arc<dyn Fn(NodeExecutionEvent) + Send + Sync>;

pub struct Executor {
    graph: NodeGraph,
    pin_values: HashMap<(NodeId, PinId), Value>,
    log_entries: Arc<Mutex<Vec<LogEntry>>>,
    /// Callback to broadcast log messages to connected clients (editors)
    log_broadcast: Option<LogBroadcast>,
    node_event_broadcast: Option<NodeEventBroadcast>,
    /// Windows that we're currently tiling (suppress events for these)
    suppressed_windows: crate::move_suppression::SuppressedWindows,
    /// Per-window policy state: adoption, minimize, WM-hidden, drag, rule
    /// flags and size constraints.
    pub window_state: WindowStateMap,
    /// Previously focused window - used for O(1) border unfocus instead of iterating all windows
    last_focused_window: Option<WindowHandle>,
    /// Logical monitor focus. Windows has no native focused-monitor primitive,
    /// so this remains sticky until an actual window focus event changes it.
    pub focused_monitor: Option<MonitorHandle>,
    /// Whether we're in startup phase (affects monitor detection logic)
    is_startup_phase: bool,
    /// Shared reference to the RuleEngine for applying window rules
    rule_engine: Arc<Mutex<crate::rules::RuleEngine>>,
    /// Tags and matched rules per window (engine state).
    pub rule_metadata: crate::rule_state::RuleMetadataMap,
    /// Programs to start for workspaces that were just created empty:
    /// the workspace name and what to launch.
    pending_workspace_launches: Vec<(String, LaunchProfile)>,
    persistent_window_sizes: PersistentSizes,
    /// Pending focus operation to be applied after hotkey execution
    pending_focus_op: Option<PendingFocusOp>,
    /// Monitors that need layout re-application after event processing.
    /// Events mark monitors as "dirty" and the runtime flushes them once per tick,
    /// coalescing multiple events into a single layout pass per monitor.
    pub pending_layout_monitors: HashSet<crate::outputs::MonitorHandle>,
    /// New windows whose entrance animation starts after the deferred layout
    /// has put them in place (slide/scale animate towards the current rect).
    pending_window_enters: Vec<WindowHandle>,
    /// Workspaces the graph's `CreateWorkspace` nodes produced, by monitor
    /// and lowercase name, with the settings they produced. A reload compares
    /// against them: unchanged settings leave the workspace alone, and a
    /// workspace missing from the new run is no longer defined by the graph.
    graph_workspaces: HashMap<(MonitorHandle, String), crate::outputs::WorkspaceSettings>,
    /// Set while a hot reload re-runs startup automation: the previous run's
    /// `graph_workspaces`. `CreateWorkspace` then updates an existing
    /// workspace instead of creating a duplicate.
    reload_previous_workspaces:
        Option<HashMap<(MonitorHandle, String), crate::outputs::WorkspaceSettings>>,
    /// Global tiling suspension: while true, the
    /// layout engine does not reposition windows, though windows remain
    /// managed. Explicit `Retile` bypasses the suspension. Unlike pause, this
    /// does not freeze the automation graph or event processing.
    tiling_suspended: bool,
    /// Pixel delta used by tiled resize commands (adjustable at runtime
    /// with `SetResizeDelta`). Session-only.
    resize_delta: i32,
    /// Scratchpad profiles, the window bound to each scratchpad and whether
    /// it shows, and the scratchpads waiting for a launched window.
    pub scratchpads: Scratchpads,
    /// Enabled named workspace profiles used to materialize non-persistent
    /// workspaces only when a selector first visits them.
    /// The configuration's workspace entries, as they are written (those
    /// that are switched off too: a display's own entry that is off
    /// means the display does not have the workspace).
    workspace_profiles: Vec<WorkspaceProfile>,
    workspace_appearance: AppearanceProfile,
    /// The wallpaper's colour, for a theme that takes its accent from it
    /// (kept current by `WmCore::apply_workspace_appearance`).
    pub wallpaper_accent: Option<(u8, u8, u8)>,
    /// Workspace IDs with active expose mode
    expose_workspaces: HashSet<usize>,
    /// Session-local locks for tabbed window groups.
    /// Keyed by monitor, workspace ID, and the stable container ID.
    pub locked_groups: groups::GroupLocks,
    /// Timestamp of last expose promote/demote transition — used to suppress
    /// the Moved handler's recenter feedback loop during transitions
    last_expose_transition: Instant,
}

/// Carry out one engine effect on the desktop. The executor says what should
/// happen to a window as an `Effect`; only `crate::apply` turns that into
/// Win32 calls.
fn effect(window_manager: &mut dyn Desktop, effect: Effect) -> Result<()> {
    window_manager.apply(&effect)
}

/// The monitor most of a window is on.
fn monitor_of(
    desktop: &dyn Desktop,
    outputs: &MonitorManager,
    window: WindowHandle,
) -> Option<crate::outputs::MonitorInfo> {
    desktop
        .output_of(window)
        .and_then(|output| outputs.get_monitor(output).cloned())
}

/// What a display is known by beyond this run of the system: its device
/// path, or its name where it has none (a saved session finds it by this
/// when its handle is another).
pub fn display_known_as(display: &crate::outputs::MonitorInfo) -> String {
    if display.device_path.trim().is_empty() {
        display.name.clone()
    } else {
        display.device_path.clone()
    }
}

/// The display a saved session means: the one with that handle, or else
/// the one known as it was.
fn display_of_session(
    outputs: &MonitorManager,
    handle: MonitorHandle,
    known_as: Option<&str>,
) -> Option<MonitorHandle> {
    if outputs.get_monitor(handle).is_some() {
        return Some(handle);
    }
    let known_as = known_as.map(str::trim).filter(|known| !known.is_empty())?;
    outputs
        .get_all_monitors()
        .iter()
        .find(|display| display_known_as(display).eq_ignore_ascii_case(known_as))
        .map(|display| display.handle)
}

/// The monitor the pointer is on.
fn monitor_at_cursor(
    desktop: &dyn Desktop,
    outputs: &MonitorManager,
) -> Option<crate::outputs::MonitorInfo> {
    desktop
        .output_at_cursor()
        .and_then(|output| outputs.get_monitor(output).cloned())
}

/// The window facts the graph's query nodes read, as the engine defines
/// them (`WindowQueries`), not the Win32 window manager's own interface.
fn queries(window_manager: &dyn Desktop) -> &dyn crate::contract::WindowQueries {
    window_manager
}

impl Executor {
    pub fn new(
        graph: NodeGraph,
        log_entries: Arc<Mutex<Vec<LogEntry>>>,
        rule_engine: Arc<Mutex<crate::rules::RuleEngine>>,
    ) -> Self {
        Self::new_with_broadcast(graph, log_entries, None, rule_engine)
    }

    pub fn new_with_broadcast(
        graph: NodeGraph,
        log_entries: Arc<Mutex<Vec<LogEntry>>>,
        log_broadcast: Option<LogBroadcast>,
        rule_engine: Arc<Mutex<crate::rules::RuleEngine>>,
    ) -> Self {
        Self::new_with_events(graph, log_entries, log_broadcast, None, rule_engine)
    }

    pub fn new_with_events(
        graph: NodeGraph,
        log_entries: Arc<Mutex<Vec<LogEntry>>>,
        log_broadcast: Option<LogBroadcast>,
        node_event_broadcast: Option<NodeEventBroadcast>,
        rule_engine: Arc<Mutex<crate::rules::RuleEngine>>,
    ) -> Self {
        Self {
            graph,
            pin_values: HashMap::new(),
            log_entries,
            log_broadcast,
            node_event_broadcast,
            suppressed_windows: crate::move_suppression::SuppressedWindows::new(),
            window_state: WindowStateMap::new(),
            last_focused_window: None,
            focused_monitor: None,
            is_startup_phase: false,
            rule_engine,
            rule_metadata: crate::rule_state::RuleMetadataMap::new(),
            pending_workspace_launches: Vec::new(),
            persistent_window_sizes: PersistentSizes::new(),
            pending_focus_op: None,
            pending_layout_monitors: HashSet::new(),
            pending_window_enters: Vec::new(),
            graph_workspaces: HashMap::new(),
            reload_previous_workspaces: None,
            tiling_suspended: false,
            resize_delta: 50,
            scratchpads: Scratchpads::new(),
            workspace_profiles: Vec::new(),
            workspace_appearance: AppearanceProfile::default(),
            wallpaper_accent: None,
            expose_workspaces: HashSet::new(),
            locked_groups: HashSet::new(),
            last_expose_transition: Instant::now(),
        }
    }

    pub fn set_scratchpad_profiles(&mut self, profiles: Vec<ScratchpadProfile>) {
        self.scratchpads.set_profiles(profiles);
    }

    pub fn set_workspace_profiles(
        &mut self,
        profiles: Vec<WorkspaceProfile>,
        appearance: AppearanceProfile,
    ) {
        self.workspace_profiles = profiles;
        self.workspace_appearance = appearance;
    }

    pub fn set_persistent_window_sizes(&mut self, sizes: Vec<PersistentWindowSize>) {
        self.persistent_window_sizes.load(sizes.iter().map(|size| {
            (
                size.exe.as_str(),
                size.initial_class.as_str(),
                size.initial_title.as_str(),
                size.width,
                size.height,
            )
        }));
    }

    pub fn persistent_window_sizes(&self) -> Vec<PersistentWindowSize> {
        self.persistent_window_sizes
            .entries()
            .into_iter()
            .map(
                |((exe, initial_class, initial_title), (width, height))| PersistentWindowSize {
                    exe,
                    initial_class,
                    initial_title,
                    width,
                    height,
                },
            )
            .collect()
    }

    pub fn focused_monitor(&self) -> Option<MonitorHandle> {
        self.focused_monitor
    }

    /// Whether automatic tiling is currently suspended globally.
    pub fn tiling_suspended(&self) -> bool {
        self.tiling_suspended
    }

    /// Take and clear any pending focus operation
    pub fn take_pending_focus_op(&mut self) -> Option<PendingFocusOp> {
        self.pending_focus_op.take()
    }

    /// Whether an overview is up on some workspace.
    pub fn overview_is_up(&self) -> bool {
        !self.expose_workspaces.is_empty()
    }

    /// Whether this window is on a shown workspace whose overview is up.
    pub fn is_in_overview(&self, monitor_manager: &MonitorManager, handle: WindowHandle) -> bool {
        self.is_in_expose(monitor_manager, handle)
    }

    /// True when `handle` is on a visible workspace that is in expose mode.
    fn is_in_expose(&self, monitor_manager: &MonitorManager, handle: WindowHandle) -> bool {
        monitor_manager
            .find_window(handle)
            .and_then(|(monitor, _)| monitor_manager.get_focused_workspace(monitor))
            .is_some_and(|workspace| self.expose_workspaces.contains(&workspace.id))
    }

    /// Reset per-execution state before handling an event or trigger.
    ///
    /// Move suppressions are deliberately kept: they belong to an earlier
    /// `SetWindowPos` whose event may still be queued behind this one. They
    /// expire on their own (see `move_suppression`).
    pub fn clear_state(&mut self) {
        self.pin_values.clear();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_focused_monitor_for_tests(&mut self, monitor: MonitorHandle) {
        self.focused_monitor = Some(monitor);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_layouts_is_empty(&self) -> bool {
        self.pending_layout_monitors.is_empty()
    }
}
