//! Workspaces: selectors, switching (which windows hide and show), moving
//! windows between workspaces and outputs, and closing and renaming
//! workspaces (docs/PHASE2_PLAN.md, step 4.3).
//!
//! Like `lifecycle`, these work on borrowed state (`World`). Layout passes,
//! switch animations and moving floating windows stay with the caller: the
//! functions say what changed and the caller decides when to lay out.

use crate::contract::Effect;
use crate::lifecycle::{Changes, World};
use crate::outputs::Outputs;
use crate::rule_state::WindowStateMap;
use crate::workspace::Workspace;
use gevrek_model::command::{MonitorTarget, WorkspaceSelector};
use gevrek_model::ids::{OutputId, WindowId};

/// Why a workspace operation was refused. The messages are user-facing (CLI,
/// gRPC replies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    NoMonitors,
    NoPrimaryMonitor,
    NoMonitorAtIndex { index: usize, count: usize },
    NoMonitorNamed(String),
    NoWorkspaceRing,
    NoWorkspaces,
    NoWorkspaceAtIndex { index: usize, count: usize },
    NoWorkspaceNamed(String),
    NoPreviousWorkspace,
    NoEmptyWorkspace,
    WorkspaceNotAvailable(usize),
    NoFocusedWorkspace,
    NotOnAWorkspace,
    RuleTargetMissing { workspace: usize, output: OutputId },
    LastWorkspace,
    WorkspaceNotEmpty { name: String, windows: usize },
    CloseFailed,
    EmptyName,
    DuplicateName(String),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMonitors => write!(f, "No attached monitors are available"),
            Self::NoPrimaryMonitor => write!(f, "No primary monitor is available"),
            Self::NoMonitorAtIndex { index, count } => write!(
                f,
                "Monitor index {index} does not exist; {count} monitor(s) are attached"
            ),
            Self::NoMonitorNamed(name) => write!(f, "No monitor named '{name}' is attached"),
            Self::NoWorkspaceRing => write!(f, "The active monitor has no workspace ring"),
            Self::NoWorkspaces => write!(f, "The active monitor has no workspaces"),
            Self::NoWorkspaceAtIndex { index, count } => write!(
                f,
                "Workspace index {index} does not exist; the active monitor has {count} workspace(s)"
            ),
            Self::NoWorkspaceNamed(name) => write!(f, "No workspace named '{name}' exists"),
            Self::NoPreviousWorkspace => {
                write!(f, "No previously visited workspace is available")
            }
            Self::NoEmptyWorkspace => write!(f, "No empty workspace exists on the active monitor"),
            Self::WorkspaceNotAvailable(index) => write!(f, "Workspace {index} is not available"),
            Self::NoFocusedWorkspace => write!(f, "The active monitor has no focused workspace"),
            Self::NotOnAWorkspace => write!(f, "The selected window is not assigned to a workspace"),
            Self::RuleTargetMissing { workspace, output } => write!(
                f,
                "Rule target workspace {workspace} does not exist on monitor {output:?}"
            ),
            Self::LastWorkspace => write!(f, "Cannot close the last workspace on the active monitor"),
            Self::WorkspaceNotEmpty { name, windows } => write!(
                f,
                "Workspace '{name}' still holds {windows} window(s); move or close them first"
            ),
            Self::CloseFailed => write!(f, "The focused workspace could not be closed"),
            Self::EmptyName => write!(f, "Workspace name cannot be empty"),
            Self::DuplicateName(name) => write!(
                f,
                "A workspace named '{name}' already exists on the active monitor"
            ),
        }
    }
}

impl std::error::Error for WorkspaceError {}

// ---- selectors ----------------------------------------------------------------

/// Resolve a monitor selector relative to `current` (ring order).
pub fn resolve_monitor_target(
    outputs: &Outputs,
    current: OutputId,
    target: &MonitorTarget,
) -> Result<OutputId, WorkspaceError> {
    let monitors = outputs.get_all_monitors_ordered();
    if monitors.is_empty() {
        return Err(WorkspaceError::NoMonitors);
    }
    let current_index = monitors
        .iter()
        .position(|monitor| monitor.handle == current)
        .unwrap_or(0);
    Ok(match target {
        MonitorTarget::Primary => outputs
            .get_primary_monitor_handle()
            .ok_or(WorkspaceError::NoPrimaryMonitor)?,
        MonitorTarget::Index(index) => monitors.get(*index).map(|monitor| monitor.handle).ok_or(
            WorkspaceError::NoMonitorAtIndex {
                index: *index,
                count: monitors.len(),
            },
        )?,
        MonitorTarget::Name(name) => monitors
            .iter()
            .find(|monitor| monitor.is_called(name))
            .map(|monitor| monitor.handle)
            .ok_or_else(|| WorkspaceError::NoMonitorNamed(name.trim().to_string()))?,
        MonitorTarget::Next => monitors[(current_index + 1) % monitors.len()].handle,
        MonitorTarget::Previous => {
            monitors[(current_index + monitors.len() - 1) % monitors.len()].handle
        }
    })
}

/// Resolve a workspace selector on `output` without creating workspaces.
pub fn resolve_existing_workspace(
    outputs: &Outputs,
    output: OutputId,
    selector: &WorkspaceSelector,
) -> Result<usize, WorkspaceError> {
    let workspaces = outputs
        .get_workspaces(output)
        .ok_or(WorkspaceError::NoWorkspaceRing)?;
    let len = workspaces.len();
    if len == 0 {
        return Err(WorkspaceError::NoWorkspaces);
    }
    let current = workspaces.focused_idx();
    match selector {
        WorkspaceSelector::Index(index) if *index < len => Ok(*index),
        WorkspaceSelector::Index(index) => Err(WorkspaceError::NoWorkspaceAtIndex {
            index: *index,
            count: len,
        }),
        WorkspaceSelector::Name(name) => workspaces
            .iter()
            .position(|workspace| workspace.name.trim().eq_ignore_ascii_case(name.trim()))
            .or_else(|| {
                workspaces
                    .iter()
                    .position(|workspace| workspace.is_called(name))
            })
            .ok_or_else(|| WorkspaceError::NoWorkspaceNamed(name.trim().to_string())),
        WorkspaceSelector::Next => Ok((current + 1) % len),
        WorkspaceSelector::Previous => Ok((current + len - 1) % len),
        WorkspaceSelector::PreviousVisited => outputs
            .get_previous_workspace_index(output)
            .filter(|index| *index < len)
            .ok_or(WorkspaceError::NoPreviousWorkspace),
        WorkspaceSelector::FirstEmpty => workspaces
            .iter()
            .position(|workspace| workspace.all_windows().is_empty())
            .ok_or(WorkspaceError::NoEmptyWorkspace),
        WorkspaceSelector::Relative(offset) => {
            Ok((current as i64 + i64::from(*offset)).rem_euclid(len as i64) as usize)
        }
    }
}

// ---- visibility -------------------------------------------------------------

/// The WM hides windows (workspace switch, group tabs).
pub fn hide_windows(states: &mut WindowStateMap, windows: &[WindowId], changes: &mut Changes) {
    for id in windows {
        states.get_mut(*id).hidden_by_wm = true;
        changes.effects.push(Effect::SetVisible {
            id: *id,
            visible: false,
        });
    }
}

/// The WM shows windows it hid.
pub fn show_windows(states: &mut WindowStateMap, windows: &[WindowId], changes: &mut Changes) {
    for id in windows {
        changes.effects.push(Effect::SetVisible {
            id: *id,
            visible: true,
        });
        states.get_mut(*id).hidden_by_wm = false;
    }
}

/// What showing a workspace means for its windows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reveal {
    /// Floating windows and the focused window of every container.
    pub shown: Vec<WindowId>,
    /// The other tabs of each container.
    pub hidden: Vec<WindowId>,
    /// The window to focus: the focused tiled one, else the focused
    /// floating one.
    pub focus: Option<WindowId>,
}

pub fn reveal_of(workspace: &Workspace) -> Reveal {
    let mut shown: Vec<WindowId> = workspace.floating_windows.iter().copied().collect();
    let mut hidden = Vec::new();
    for container in workspace.containers.iter() {
        // A column out of view (scrolling layout) is not shown at all.
        if container.offscreen {
            hidden.extend(container.all_windows());
            continue;
        }
        hidden.extend(container.non_focused_windows());
        if let Some(window) = container.focused_window() {
            shown.push(window);
        }
    }
    let focus = workspace
        .focused_window()
        .or_else(|| workspace.floating_windows.focused().copied());
    Reveal {
        shown,
        hidden,
        focus,
    }
}

/// Show the focused workspace of `output`: hide the background tabs, then
/// show the rest. The caller lays the workspace out after the effects are
/// applied (some apps ignore moves while hidden) and focuses `focus`.
pub fn reveal(
    world: &mut World,
    output: OutputId,
    changes: &mut Changes,
) -> Result<Reveal, WorkspaceError> {
    let reveal = reveal_of(
        world
            .outputs
            .get_focused_workspace(output)
            .ok_or(WorkspaceError::NoFocusedWorkspace)?,
    );
    hide_windows(world.windows, &reveal.hidden, changes);
    show_windows(world.windows, &reveal.shown, changes);
    Ok(reveal)
}

/// Hide every window of the focused workspace of `output`.
pub fn hide_focused_workspace(world: &mut World, output: OutputId, changes: &mut Changes) {
    let windows = world
        .outputs
        .get_focused_workspace(output)
        .map(|workspace| workspace.all_windows())
        .unwrap_or_default();
    hide_windows(world.windows, &windows, changes);
}

/// First half of a workspace switch on `output`: hide the current
/// workspace's windows and focus `target`. Returns the index left, or `None`
/// when `target` is already focused. The caller then lays the new workspace
/// out, starts its transition, and calls `reveal`.
pub fn begin_switch(
    world: &mut World,
    output: OutputId,
    target: usize,
    changes: &mut Changes,
) -> Result<Option<usize>, WorkspaceError> {
    let current = world
        .outputs
        .get_workspaces(output)
        .map(|workspaces| workspaces.focused_idx())
        .ok_or(WorkspaceError::NoWorkspaces)?;
    if current == target {
        return Ok(None);
    }
    hide_focused_workspace(world, output, changes);
    if !world.outputs.focus_workspace_at(output, target) {
        return Err(WorkspaceError::WorkspaceNotAvailable(target));
    }
    Ok(Some(current))
}

// ---- moving windows -----------------------------------------------------------

/// Where a moved window came from and went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    /// `None` when the window was not a workspace member before.
    pub from: Option<(OutputId, usize)>,
    pub to: (OutputId, usize),
    pub floating: bool,
    /// The target workspace is the visible one on its output, so the window
    /// is shown; otherwise it is hidden.
    pub visible: bool,
}

/// Move a window into workspace `index` of `output`. `floating`: `None`
/// keeps how the window is managed now. The window is shown when the target
/// is the visible workspace and hidden otherwise. Returns `None` when it is
/// already there. The caller lays out the workspaces involved.
pub fn move_window(
    world: &mut World,
    id: WindowId,
    output: OutputId,
    index: usize,
    floating: Option<bool>,
    changes: &mut Changes,
) -> Result<Option<Moved>, WorkspaceError> {
    let count = world
        .outputs
        .get_workspaces(output)
        .map(|workspaces| workspaces.len())
        .unwrap_or(0);
    if index >= count {
        return Err(WorkspaceError::WorkspaceNotAvailable(index));
    }
    if world.outputs.find_window(id) == Some((output, index)) {
        return Ok(None);
    }
    // A window that floats only because its workspace has the Floating
    // layout does not take that with it: on a workspace that tiles it is
    // tiled (and under the Floating layout it floats again, in the middle).
    // One the user or a rule floated stays floating wherever it goes.
    let by_layout = world.outputs.find_window(id).is_some_and(|(from, at)| {
        world
            .outputs
            .get_workspaces(from)
            .and_then(|workspaces| workspaces.elements().get(at))
            .is_some_and(|workspace| workspace.floated_by_layout.contains(&id))
    });
    let floating = floating.unwrap_or_else(|| world.outputs.is_window_floating(id)) && !by_layout;
    let from = world.outputs.find_and_remove_window(id);
    let visible = world
        .outputs
        .get_workspaces(output)
        .is_some_and(|workspaces| workspaces.focused_idx() == index);
    let workspace = world
        .outputs
        .get_workspaces_mut(output)
        .and_then(|workspaces| workspaces.get_mut(index))
        .ok_or(WorkspaceError::WorkspaceNotAvailable(index))?;
    if floating {
        workspace.add_floating_window(id);
    } else {
        workspace.add_window(id, true);
    }
    if visible {
        show_windows(world.windows, &[id], changes);
    } else {
        hide_windows(world.windows, &[id], changes);
    }
    Ok(Some(Moved {
        from,
        to: (output, index),
        floating,
        visible,
    }))
}

// ---- workspace management ---------------------------------------------------

/// Close the focused workspace of `output`. Refused while it holds windows
/// or is the output's last workspace. Returns the closed workspace's name;
/// the caller reveals the workspace that took its place.
pub fn close_focused_workspace(
    outputs: &mut Outputs,
    output: OutputId,
) -> Result<String, WorkspaceError> {
    let (name, windows, len) = {
        let workspaces = outputs
            .get_workspaces(output)
            .ok_or(WorkspaceError::NoWorkspaceRing)?;
        let workspace = workspaces
            .focused()
            .ok_or(WorkspaceError::NoFocusedWorkspace)?;
        (
            workspace.name.clone(),
            workspace.all_windows().len(),
            workspaces.len(),
        )
    };
    if len <= 1 {
        return Err(WorkspaceError::LastWorkspace);
    }
    if windows > 0 {
        return Err(WorkspaceError::WorkspaceNotEmpty { name, windows });
    }
    outputs
        .close_focused_empty_workspace(output)
        .ok_or(WorkspaceError::CloseFailed)
}

/// A rename that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    pub workspace_id: usize,
    pub old_name: String,
    /// Its windows, whose dynamic rules may match differently now.
    pub windows: Vec<WindowId>,
}

/// Rename workspace `index` of `output`: give it a title (what is shown of
/// it). The name it is known by to what names it stays, so keys and rules
/// go on meaning it. No two workspaces of an output are shown or named
/// alike (case-insensitive). Its own name for a title is no title.
/// Returns `None` when nothing changes; `old_name` is what it was shown
/// by.
pub fn rename_workspace(
    outputs: &mut Outputs,
    output: OutputId,
    index: usize,
    name: &str,
) -> Result<Option<Renamed>, WorkspaceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(WorkspaceError::EmptyName);
    }
    let duplicate = outputs.get_workspaces(output).is_some_and(|workspaces| {
        workspaces
            .iter()
            .enumerate()
            .any(|(candidate, workspace)| candidate != index && workspace.is_called(name))
    });
    if duplicate {
        return Err(WorkspaceError::DuplicateName(name.to_string()));
    }
    let workspace = outputs
        .get_workspaces_mut(output)
        .and_then(|workspaces| workspaces.get_mut(index))
        .ok_or(WorkspaceError::WorkspaceNotAvailable(index))?;
    if workspace.label() == name {
        return Ok(None);
    }
    let old_name = workspace.label().to_string();
    workspace.title = (workspace.name.trim() != name).then(|| name.to_string());
    // (One that was given a name by hand is not a workspace of the moment
    // any more.)
    workspace.persistent = true;
    Ok(Some(Renamed {
        workspace_id: workspace.id,
        old_name,
        windows: workspace.all_windows(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::LayoutType;

    const A: OutputId = OutputId(1);
    const B: OutputId = OutputId(2);

    struct Fixture {
        outputs: Outputs,
        windows: WindowStateMap,
    }

    impl Fixture {
        /// Output A with workspaces one, two, three; output B with "b".
        fn new() -> Self {
            let mut outputs = Outputs::new();
            outputs.add_test_monitor(A, "A");
            outputs.add_test_monitor(B, "B");
            for name in ["one", "two", "three"] {
                outputs.create_workspace(A, name.into(), LayoutType::BSP);
            }
            outputs.create_workspace(B, "b".into(), LayoutType::BSP);
            outputs.focus_workspace_at(A, 0);
            Self {
                outputs,
                windows: WindowStateMap::new(),
            }
        }

        fn world(&mut self) -> World<'_> {
            World {
                outputs: &mut self.outputs,
                windows: &mut self.windows,
            }
        }

        fn add(&mut self, output: OutputId, index: usize, id: WindowId) {
            let ws = self
                .outputs
                .get_workspaces_mut(output)
                .unwrap()
                .get_mut(index)
                .unwrap();
            ws.add_window(id, true);
        }
    }

    fn visibility(changes: &Changes) -> Vec<(WindowId, bool)> {
        changes
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::SetVisible { id, visible } => Some((*id, *visible)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn selectors_resolve_like_the_executor_did() {
        let f = Fixture::new();
        let o = &f.outputs;
        assert_eq!(
            resolve_existing_workspace(o, A, &WorkspaceSelector::Next),
            Ok(1)
        );
        assert_eq!(
            resolve_existing_workspace(o, A, &WorkspaceSelector::Previous),
            Ok(2)
        );
        assert_eq!(
            resolve_existing_workspace(o, A, &WorkspaceSelector::Relative(-4)),
            Ok(2)
        );
        assert_eq!(
            resolve_existing_workspace(o, A, &WorkspaceSelector::Name(" TWO ".into())),
            Ok(1)
        );
        assert_eq!(
            resolve_existing_workspace(o, A, &WorkspaceSelector::FirstEmpty),
            Ok(0)
        );
        let missing = resolve_existing_workspace(o, A, &WorkspaceSelector::Index(7)).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "Workspace index 7 does not exist; the active monitor has 3 workspace(s)"
        );

        assert_eq!(resolve_monitor_target(o, A, &MonitorTarget::Next), Ok(B));
        assert_eq!(
            resolve_monitor_target(o, A, &MonitorTarget::Previous),
            Ok(B)
        );
        assert_eq!(resolve_monitor_target(o, B, &MonitorTarget::Primary), Ok(A));
        assert_eq!(
            resolve_monitor_target(o, A, &MonitorTarget::Name("nope".into()))
                .unwrap_err()
                .to_string(),
            "No monitor named 'nope' is attached"
        );
    }

    #[test]
    fn a_switch_hides_the_old_workspace_and_reveals_the_new_one() {
        let mut f = Fixture::new();
        let (old, new) = (WindowId(1), WindowId(2));
        f.add(A, 0, old);
        f.add(A, 1, new);

        let mut changes = Changes::default();
        assert_eq!(
            begin_switch(&mut f.world(), A, 1, &mut changes),
            Ok(Some(0))
        );
        let reveal = reveal(&mut f.world(), A, &mut changes).unwrap();

        assert_eq!(visibility(&changes), vec![(old, false), (new, true)]);
        assert_eq!(reveal.focus, Some(new));
        assert!(f.windows.get(old).hidden_by_wm);
        assert!(!f.windows.get(new).hidden_by_wm);
        assert_eq!(f.outputs.get_workspaces(A).unwrap().focused_idx(), 1);

        assert_eq!(
            begin_switch(&mut f.world(), A, 1, &mut Changes::default()),
            Ok(None)
        );
    }

    #[test]
    fn background_tabs_stay_hidden_when_a_workspace_is_revealed() {
        let mut f = Fixture::new();
        let (front, back) = (WindowId(1), WindowId(2));
        let ws = f.outputs.get_focused_workspace_mut(A).unwrap();
        ws.add_window(back, true);
        ws.containers.focused_mut().unwrap().add_window(front);
        ws.containers.focused_mut().unwrap().focus_window(front);

        let mut changes = Changes::default();
        let reveal = reveal(&mut f.world(), A, &mut changes).unwrap();
        assert_eq!(reveal.hidden, vec![back]);
        assert_eq!(visibility(&changes), vec![(back, false), (front, true)]);
    }

    #[test]
    fn a_moved_window_is_shown_only_on_a_visible_workspace() {
        let mut f = Fixture::new();
        let w = WindowId(1);
        f.add(A, 0, w);

        let mut changes = Changes::default();
        let moved = move_window(&mut f.world(), w, A, 2, None, &mut changes)
            .unwrap()
            .unwrap();
        assert_eq!(moved.from, Some((A, 0)));
        assert!(!moved.visible);
        assert_eq!(visibility(&changes), vec![(w, false)]);
        assert!(
            f.windows.get(w).hidden_by_wm,
            "a WM hide, so restore-windows finds it"
        );

        let mut changes = Changes::default();
        let moved = move_window(&mut f.world(), w, B, 0, None, &mut changes)
            .unwrap()
            .unwrap();
        assert!(moved.visible);
        assert_eq!(visibility(&changes), vec![(w, true)]);
        assert_eq!(f.outputs.find_window(w), Some((B, 0)));

        assert_eq!(
            move_window(&mut f.world(), w, B, 0, None, &mut Changes::default()),
            Ok(None)
        );
        assert_eq!(
            move_window(&mut f.world(), w, B, 5, None, &mut Changes::default()),
            Err(WorkspaceError::WorkspaceNotAvailable(5))
        );
    }

    #[test]
    fn only_an_empty_workspace_that_is_not_the_last_closes() {
        let mut f = Fixture::new();
        f.add(A, 0, WindowId(1));
        assert_eq!(
            close_focused_workspace(&mut f.outputs, A),
            Err(WorkspaceError::WorkspaceNotEmpty {
                name: "one".into(),
                windows: 1
            })
        );
        assert_eq!(
            close_focused_workspace(&mut f.outputs, B),
            Err(WorkspaceError::LastWorkspace)
        );
        f.outputs.focus_workspace_at(A, 1);
        assert_eq!(close_focused_workspace(&mut f.outputs, A), Ok("two".into()));
        assert_eq!(f.outputs.get_workspaces(A).unwrap().len(), 2);
    }

    #[test]
    fn renames_are_unique_per_output_and_report_the_windows() {
        let mut f = Fixture::new();
        f.add(A, 0, WindowId(1));
        assert_eq!(
            rename_workspace(&mut f.outputs, A, 0, "TWO"),
            Err(WorkspaceError::DuplicateName("TWO".into()))
        );
        assert_eq!(
            rename_workspace(&mut f.outputs, A, 0, "  "),
            Err(WorkspaceError::EmptyName)
        );
        let renamed = rename_workspace(&mut f.outputs, A, 0, "main")
            .unwrap()
            .unwrap();
        assert_eq!(renamed.old_name, "one");
        assert_eq!(renamed.windows, vec![WindowId(1)]);
        assert_eq!(rename_workspace(&mut f.outputs, A, 0, "main"), Ok(None));
    }
}
