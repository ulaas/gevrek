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
    /// Who the display is wherever it is plugged in
    /// (`MonitorInfo::stable_id`).
    pub stable_id: String,
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
    /// How much of the room the work area leaves at each side is the
    /// shell's zones' own, as taken off for this work area and in its
    /// units. The shell reads it back: what it tells the zones of the
    /// room a display has must be without the zones' own.
    pub zones_room: gevrek_model::settings::ReservedArea,
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
            field("stable_id", monitor.stable_id.clone()),
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
    // A window that is listed for another workspace, or another output,
    // than before. Whatever brought it there (a drag that ended, a key of
    // the graph, a rule, an output that went) is told here once, from
    // what is published: a panel lists a window for its own output only,
    // and one that was never told showed the window on the output it had
    // left until something else happened.
    let (were, are) = (homes(before), homes(after));
    for (window, now) in &are {
        let Some(was) = were.get(window) else {
            continue;
        };
        if (was.output, was.workspace_id) != (now.output, now.workspace_id) {
            events.push((
                Kind::WindowWorkspaceChanged,
                vec![
                    field("window", window.0.to_string()),
                    field("output", now.output.0.to_string()),
                    field("workspace_id", now.workspace_id.to_string()),
                    field("name", now.workspace_name.to_string()),
                    field("previous_output", was.output.0.to_string()),
                    field("previous_workspace_id", was.workspace_id.to_string()),
                    field("previous_name", was.workspace_name.to_string()),
                ],
            ));
        }
    }
    events
}

/// Where a window is listed in a snapshot of the outputs: the output and
/// the workspace it belongs to, by the window manager's own account of
/// its workspaces. Never by where the window lies: its rectangle passes
/// through other outputs while its workspace slides in, stays where it
/// was while its workspace is put away, and can lie across two outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Home<'a> {
    pub output: MonitorHandle,
    pub workspace_id: usize,
    pub workspace_name: &'a str,
}

/// Every window a snapshot lists, with the output and the workspace it is
/// listed for. A window is listed once; were it ever listed twice, the
/// first listing is the one kept here (and [`listed_amiss`] says so).
pub fn homes(monitors: &[MonitorData]) -> std::collections::BTreeMap<WindowHandle, Home<'_>> {
    let mut found = std::collections::BTreeMap::new();
    for monitor in monitors {
        for workspace in &monitor.workspaces {
            for window in &workspace.detailed_windows {
                found.entry(window.handle).or_insert(Home {
                    output: monitor.handle,
                    workspace_id: workspace.id,
                    workspace_name: &workspace.name,
                });
            }
        }
    }
    found
}

/// What is wrong with how a snapshot lists its windows, if anything: a
/// window listed for two outputs or twice for one (two panels would show
/// it, or one would show it twice), or an output whose shown workspace
/// is none of its own. Each panel draws one output's part of the
/// snapshot and nothing else, so a snapshot without such a fault is what
/// keeps a panel its own output's alone.
pub fn listed_amiss(monitors: &[MonitorData]) -> Option<String> {
    let mut seen: std::collections::BTreeMap<WindowHandle, (MonitorHandle, &str)> =
        std::collections::BTreeMap::new();
    for monitor in monitors {
        if !monitor
            .workspaces
            .iter()
            .any(|workspace| workspace.id == monitor.workspace.id)
        {
            return Some(format!(
                "the workspace {} shown on {} is none of that output's",
                monitor.workspace.name, monitor.name
            ));
        }
        for workspace in &monitor.workspaces {
            for window in &workspace.detailed_windows {
                if let Some((output, name)) =
                    seen.insert(window.handle, (monitor.handle, &workspace.name))
                {
                    return Some(format!(
                        "{:?} is listed for workspace {name} of {output:?} and again for workspace {} of {:?}",
                        window.handle, workspace.name, monitor.handle
                    ));
                }
            }
        }
    }
    None
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
            stable_id: format!("GSM-5B7F-0000000{handle}"),
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
            zones_room: Default::default(),
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

    fn window(handle: isize) -> DetailedWindowInfo {
        DetailedWindowInfo {
            handle: gevrek_model::ids::WindowId(handle),
            title: format!("window {handle}"),
            class: String::new(),
            exe: String::new(),
            app_id: String::new(),
            icon: String::new(),
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            is_focused: false,
            is_minimized: false,
            is_maximized: false,
            is_managed: true,
            is_tiled: false,
            is_floating: true,
            has_border: false,
            is_border_window: false,
            workspace_id: 0,
            tags: Vec::new(),
            matched_rules: Vec::new(),
            effective_rule_effects: Vec::new(),
            effective_opacity: 255,
            is_dimmed: false,
            shadow_policy: String::new(),
        }
    }

    /// Two outputs side by side, each with the windows named for its one
    /// workspace.
    fn two_outputs(first: &[isize], second: &[isize]) -> Vec<MonitorData> {
        let with = |handle: isize, x: i32, windows: &[isize]| {
            let mut shown = workspace(0, "1", "Floating");
            shown.detailed_windows = windows.iter().map(|id| window(*id)).collect();
            shown.total_windows = windows.len();
            monitor(handle, x, shown)
        };
        vec![with(1, 0, first), with(2, 1920, second)]
    }

    /// The fault that was reported: a window let go of on the other
    /// output belonged to that output's workspace from then on, and no
    /// event said so that a panel asks for the state after (a drag's end
    /// was none of them). The panel of the output it had left went on
    /// showing it. The change is told from the snapshots themselves.
    #[test]
    fn a_window_that_is_listed_for_another_output_is_told() {
        let before = two_outputs(&[7, 8], &[9]);
        let after = two_outputs(&[8], &[9, 7]);
        let events = changes(&before, &after);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].0, Kind::WindowWorkspaceChanged);
        assert_eq!(events[0].0.as_str(), "window.workspace_changed");
        let fields: std::collections::BTreeMap<_, _> = events[0].1.iter().cloned().collect();
        assert_eq!(
            (
                fields["window"].as_str(),
                fields["previous_output"].as_str(),
                fields["output"].as_str()
            ),
            ("7", "1", "2")
        );
        // Told once: the next snapshot is the same.
        assert!(changes(&after, &after.clone()).is_empty());
    }

    #[test]
    fn a_window_that_is_listed_for_another_workspace_of_its_output_is_told() {
        let mut before = two_outputs(&[7], &[]);
        before[0].workspaces.push(workspace(1, "2", "Dwindle"));
        let mut after = before.clone();
        let moved = after[0].workspaces[0].detailed_windows.remove(0);
        after[0].workspace.detailed_windows.clear();
        after[0].workspaces[1].detailed_windows.push(moved);
        let events = changes(&before, &after);
        assert_eq!(kinds(&before, &after), [Kind::WindowWorkspaceChanged]);
        let fields: std::collections::BTreeMap<_, _> = events[0].1.iter().cloned().collect();
        assert_eq!(
            (fields["previous_name"].as_str(), fields["name"].as_str()),
            ("1", "2")
        );
    }

    /// Windows that open and close have events of their own, and one that
    /// only lies elsewhere on its output's workspace is where it was.
    #[test]
    fn windows_that_come_go_or_only_lie_elsewhere_are_not_told_as_moved() {
        let before = two_outputs(&[7], &[9]);
        assert!(changes(&before, &two_outputs(&[7, 8], &[9])).is_empty());
        assert!(changes(&before, &two_outputs(&[7], &[])).is_empty());
        // Its rectangle is on the second output (its workspace slides, or
        // it floats across the edge): it is the first output's still.
        let mut slid = before.clone();
        slid[0].workspaces[0].detailed_windows[0].x = 2400;
        assert!(changes(&before, &slid).is_empty());
        assert_eq!(homes(&slid)[&gevrek_model::ids::WindowId(7)].output.0, 1);
    }

    #[test]
    fn a_snapshot_that_lists_a_window_for_two_outputs_is_amiss() {
        assert_eq!(listed_amiss(&two_outputs(&[7, 8], &[9])), None);
        let twice = listed_amiss(&two_outputs(&[7, 8], &[7])).expect("amiss");
        assert!(twice.contains("WindowId(7)"), "{twice}");
        assert!(listed_amiss(&two_outputs(&[7, 7], &[])).is_some());
        // The workspace an output shows is one of its own.
        let mut strange = two_outputs(&[7], &[]);
        strange[1].workspace.id = 5;
        assert!(listed_amiss(&strange).is_some());
    }
}
