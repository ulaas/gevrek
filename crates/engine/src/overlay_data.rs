//! Live desktop state the runtime publishes for the overlay binary, the
//! editor and `QueryState` (monitors, workspaces, windows, rules,
//! performance). Phase 2 turns this into the engine's `StateSnapshot`.

use gevrek_model::ids::{OutputId as MonitorHandle, WindowId as WindowHandle};
use gevrek_model::window::WindowInfo;

/// Data sent from the WM runtime to the debug overlay
#[derive(Debug, Clone)]
pub struct OverlayData {
    pub timestamp: std::time::SystemTime,
    pub monitors: Vec<MonitorData>,
    pub graph_node_count: usize,
    pub total_windows: usize,
    pub event_loop_fps: f64,
    pub last_event: Option<String>,
    pub active_animations: usize,
    pub active_binding_submap: String,
    /// Whether global tiling is currently suspended.
    pub tiling_suspended: bool,
    pub gesture: GestureStateData,
    pub rule_states: Vec<RuleStateData>,
    pub scratchpad_states: Vec<ScratchpadStateData>,
    pub startup_states: Vec<StartupStateData>,
    /// Performance metrics
    pub metrics: Option<PerformanceMetrics>,
    /// What the window manager has to tell the person at the machine.
    pub notices: Vec<Notice>,
    /// The accent the desktop has now, where it is not the
    /// configuration's own: the wallpaper's colour.
    pub accent: Option<(u8, u8, u8)>,
    /// The keyboard layout in use, where the system tells it.
    pub keyboard_layout: Option<KeyboardLayoutNow>,
    /// How the frames of each display got there, where the window
    /// manager puts them there itself.
    pub frames: Vec<DisplayFrames>,
}

/// How the frames of a display got there since the window manager
/// started: handed over as a window drew them (a window that covers the
/// display, a game: nothing is drawn again, nothing is copied), or put
/// together by the window manager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplayFrames {
    /// The display's name (`DP-1`).
    pub output: String,
    /// Frames that were a window's own picture.
    pub direct: u64,
    /// Frames the window manager drew.
    pub composed: u64,
    /// The last frame was a window's own picture.
    pub direct_now: bool,
}

/// The keyboard layout in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLayoutNow {
    /// As the configuration writes it (`tr`), or the first letters of
    /// its name.
    pub short: String,
    /// Its full name (`Turkish`).
    pub name: String,
    /// Its place among the layouts, from 0, and how many there are.
    pub index: u32,
    pub count: u32,
}

impl KeyboardLayoutNow {
    /// The short word for a layout: the one at its place in the
    /// configuration's list (`us,tr`), else the first two letters of its
    /// name.
    pub fn short_of(configured: &str, index: u32, name: &str) -> String {
        configured
            .split(',')
            .map(str::trim)
            .nth(index as usize)
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| name.chars().take(2).collect::<String>().to_lowercase())
    }
}

/// Something the window manager itself has to tell the person at the
/// machine, for as long as it is so: another program that reacts to the
/// same keys, a setting that cannot be followed here. Not a notification
/// of a program (those are the shell's): a notice is there while its
/// cause is, and gone with it. The panel shows them; `gevrek state` lists
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// What it is about, the same for the same cause (`fancyzones-shift-drag`).
    pub id: String,
    /// One line.
    pub title: String,
    /// What is the matter and what can be done about it.
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct GestureStateData {
    pub available: bool,
    pub backend: String,
    pub device_count: u32,
    pub configured_count: u32,
    pub active: Option<GestureProgressData>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GestureProgressData {
    pub phase: String,
    pub fingers: u8,
    pub direction: Option<String>,
    pub progress: f32,
    pub delta_x: f32,
    pub delta_y: f32,
}

#[derive(Debug, Clone)]
pub struct RuleStateData {
    pub name: String,
    pub enabled: bool,
    pub dynamic: bool,
}

#[derive(Debug, Clone)]
pub struct ScratchpadStateData {
    pub id: String,
    pub enabled: bool,
    pub handle: Option<WindowHandle>,
    pub visible: bool,
    pub pending_spawn: bool,
}

#[derive(Debug, Clone)]
pub struct StartupStateData {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub status: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

/// Performance metrics for monitoring WM health
#[derive(Debug, Clone, Default)]
pub struct PerformanceMetrics {
    /// Average event loop iteration time in microseconds
    pub avg_loop_time_us: f64,
    /// Maximum event loop iteration time in microseconds
    pub max_loop_time_us: f64,
    /// Number of window events processed per second
    pub events_per_second: f64,
    /// Number of layout operations per second
    pub layouts_per_second: f64,
    /// Memory usage estimate (tracked window count * estimated bytes per window)
    pub estimated_memory_kb: usize,
    /// Number of active connections (editors)
    pub active_connections: usize,
    /// Uptime in seconds
    pub uptime_seconds: u64,
    /// Number of hotkeys registered
    pub registered_hotkeys: usize,
    /// Number of active rules
    pub active_rules: usize,
    /// Number of scratchpad windows tracked
    pub scratchpad_count: usize,
}

#[derive(Debug, Clone)]
pub struct MonitorData {
    pub handle: MonitorHandle,
    pub name: String,
    /// The connector or device the output is on.
    pub connector: String,
    pub is_primary: bool,
    pub is_focused: bool,
    /// The currently focused workspace, retained for compact overlays.
    pub workspace: WorkspaceData,
    /// Every materialized workspace on this monitor in stable ring order.
    pub workspaces: Vec<WorkspaceData>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub work_x: i32,
    pub work_y: i32,
    pub work_width: i32,
    pub work_height: i32,
    pub reserved_left: i32,
    pub reserved_top: i32,
    pub reserved_right: i32,
    pub reserved_bottom: i32,
    /// Where the bar belongs on this monitor: x, y, width, height.
    pub bar: Option<(i32, i32, i32, i32)>,
    /// Device pixels per logical pixel (1.0, 1.25, 2.0 ...).
    pub scale: f64,
    /// The refresh rate and the modes, where the backend knows them.
    pub detail: crate::outputs::OutputDetail,
}

/// What changed between two snapshots of the outputs, as the events
/// subscribers are told of: outputs that came, went or changed, another
/// workspace shown on an output, another layout of the workspace shown. A
/// host calls this each time it publishes a snapshot, so a panel follows
/// the desktop without asking again and again.
pub fn changes(
    before: &[MonitorData],
    after: &[MonitorData],
) -> Vec<(crate::executor::RuntimeEventKind, Vec<(String, String)>)> {
    use crate::executor::RuntimeEventKind as Kind;
    let field = |name: &str, value: String| (name.to_string(), value);
    let output_fields = |monitor: &MonitorData| {
        vec![
            field("output", monitor.handle.0.to_string()),
            field("name", monitor.name.clone()),
            field("connector", monitor.connector.clone()),
            field("x", monitor.x.to_string()),
            field("y", monitor.y.to_string()),
            field("width", monitor.width.to_string()),
            field("height", monitor.height.to_string()),
            field("scale", monitor.scale.to_string()),
            field("primary", monitor.is_primary.to_string()),
        ]
    };
    let mut events = Vec::new();
    for old in before {
        if !after.iter().any(|new| new.handle == old.handle) {
            events.push((Kind::OutputRemoved, output_fields(old)));
        }
    }
    for new in after {
        let Some(old) = before.iter().find(|old| old.handle == new.handle) else {
            events.push((Kind::OutputAdded, output_fields(new)));
            continue;
        };
        let shape = |monitor: &MonitorData| {
            (
                (monitor.x, monitor.y, monitor.width, monitor.height),
                (
                    monitor.work_x,
                    monitor.work_y,
                    monitor.work_width,
                    monitor.work_height,
                ),
                monitor.scale.to_bits(),
                monitor.is_primary,
                monitor.name.clone(),
            )
        };
        if shape(old) != shape(new) {
            events.push((Kind::OutputChanged, output_fields(new)));
        }
        if old.workspace.id != new.workspace.id || old.workspace.name != new.workspace.name {
            events.push((
                Kind::WorkspaceSwitched,
                vec![
                    field("output", new.handle.0.to_string()),
                    field("workspace_id", new.workspace.id.to_string()),
                    field("name", new.workspace.name.clone()),
                    field("previous_id", old.workspace.id.to_string()),
                    field("previous_name", old.workspace.name.clone()),
                ],
            ));
        } else if old.workspace.layout_type != new.workspace.layout_type {
            events.push((
                Kind::LayoutChanged,
                vec![
                    field("output", new.handle.0.to_string()),
                    field("workspace_id", new.workspace.id.to_string()),
                    field("name", new.workspace.name.clone()),
                    field("layout", new.workspace.layout_type.clone()),
                    field("previous_layout", old.workspace.layout_type.clone()),
                ],
            ));
        }
    }
    events
}

#[derive(Debug, Clone)]
pub struct WorkspaceData {
    pub id: usize,
    /// What it is known by to what names it (keys, rules, commands).
    pub name: String,
    /// What is shown of it: its title, or its name where it has none.
    pub title: String,
    /// The configuration has an entry for it on its display.
    pub configured: bool,
    /// It stays when it is left empty.
    pub persistent: bool,
    pub layout_type: String,
    pub total_windows: usize,
    pub tiled_windows: usize,
    pub floating_windows: usize,
    pub container_count: usize,
    pub dwindle_direction: String,
    pub dwindle_preserve_split: bool,
    pub pseudotiled_windows: usize,
    pub master_position: String,
    pub new_window_placement: String,
    pub tiling_enabled: bool,
    pub float_override: bool,
    pub window_container_behaviour: String,
    pub windows: Vec<WindowInfo>,
    pub detailed_windows: Vec<DetailedWindowInfo>, // Detailed window information
}

/// Detailed window information including management status
#[derive(Debug, Clone)]
pub struct DetailedWindowInfo {
    pub handle: WindowHandle,
    pub title: String,
    pub class: String,
    pub exe: String,
    /// What the program calls itself (see `WindowQueries::app_id`).
    pub app_id: String,
    /// The picture the window's program gave it itself (see
    /// `WindowQueries::icon`); empty where it gave none.
    pub icon: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub is_focused: bool,
    pub is_minimized: bool,
    pub is_maximized: bool,
    pub is_managed: bool,       // Whether window is managed by WM
    pub is_tiled: bool,         // Window is in tiling layout (not floating)
    pub is_floating: bool,      // Window is floating
    pub has_border: bool,       // Window has decorative border
    pub is_border_window: bool, // This IS a border window (internal)
    pub workspace_id: usize,    // Which workspace contains this window
    pub tags: Vec<String>,
    pub matched_rules: Vec<String>,
    pub effective_rule_effects: Vec<String>,
    pub effective_opacity: u8,
    pub is_dimmed: bool,
    pub shadow_policy: String,
}

#[cfg(test)]
mod change_tests {
    use super::*;
    use crate::executor::RuntimeEventKind as Kind;

    fn workspace(id: usize, name: &str, layout: &str) -> WorkspaceData {
        WorkspaceData {
            id,
            name: name.to_string(),
            title: name.to_string(),
            configured: false,
            persistent: true,
            layout_type: layout.to_string(),
            total_windows: 0,
            tiled_windows: 0,
            floating_windows: 0,
            container_count: 0,
            dwindle_direction: String::new(),
            dwindle_preserve_split: false,
            pseudotiled_windows: 0,
            master_position: String::new(),
            new_window_placement: String::new(),
            tiling_enabled: true,
            float_override: false,
            window_container_behaviour: String::new(),
            windows: Vec::new(),
            detailed_windows: Vec::new(),
        }
    }

    fn monitor(handle: isize, x: i32, shown: WorkspaceData) -> MonitorData {
        MonitorData {
            handle: crate::outputs::MonitorHandle(handle),
            name: format!("out-{handle}"),
            connector: format!("DP-{handle}"),
            is_primary: handle == 1,
            is_focused: handle == 1,
            workspaces: vec![shown.clone()],
            workspace: shown,
            x,
            y: 0,
            width: 1920,
            height: 1080,
            work_x: x,
            work_y: 0,
            work_width: 1920,
            work_height: 1080,
            reserved_left: 0,
            reserved_top: 0,
            reserved_right: 0,
            reserved_bottom: 0,
            bar: None,
            scale: 1.0,
            detail: Default::default(),
        }
    }

    fn kinds(before: &[MonitorData], after: &[MonitorData]) -> Vec<Kind> {
        changes(before, after)
            .into_iter()
            .map(|(kind, _)| kind)
            .collect()
    }

    #[test]
    fn nothing_changed_means_no_event() {
        let outputs = vec![monitor(1, 0, workspace(0, "1", "Dwindle"))];
        assert!(changes(&outputs, &outputs.clone()).is_empty());
    }

    #[test]
    fn outputs_coming_going_and_changing_are_told() {
        let one = vec![monitor(1, 0, workspace(0, "1", "Dwindle"))];
        let two = vec![
            monitor(1, 0, workspace(0, "1", "Dwindle")),
            monitor(2, 1920, workspace(0, "1", "Dwindle")),
        ];
        assert_eq!(kinds(&[], &one), [Kind::OutputAdded]);
        assert_eq!(kinds(&one, &two), [Kind::OutputAdded]);
        assert_eq!(kinds(&two, &one), [Kind::OutputRemoved]);
        let mut scaled = one.clone();
        scaled[0].scale = 2.0;
        assert_eq!(kinds(&one, &scaled), [Kind::OutputChanged]);
    }

    #[test]
    fn another_workspace_or_layout_on_an_output_is_told_with_its_names() {
        let before = vec![monitor(1, 0, workspace(0, "1", "Dwindle"))];
        let switched = vec![monitor(1, 0, workspace(1, "2", "Grid"))];
        let events = changes(&before, &switched);
        assert_eq!(
            events.len(),
            1,
            "a switch is one event, not also a layout change"
        );
        assert_eq!(events[0].0, Kind::WorkspaceSwitched);
        let fields: std::collections::BTreeMap<_, _> = events[0].1.iter().cloned().collect();
        assert_eq!(fields["name"], "2");
        assert_eq!(fields["previous_name"], "1");

        let relaid = vec![monitor(1, 0, workspace(0, "1", "Monocle"))];
        let events = changes(&before, &relaid);
        assert_eq!(events[0].0, Kind::LayoutChanged);
        let fields: std::collections::BTreeMap<_, _> = events[0].1.iter().cloned().collect();
        assert_eq!(
            (
                fields["layout"].as_str(),
                fields["previous_layout"].as_str()
            ),
            ("Monocle", "Dwindle")
        );
    }
}
