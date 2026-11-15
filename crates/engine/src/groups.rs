//! Tab groups: containers holding several windows, of which one shows
//! (docs/PHASE2_PLAN.md, step 4.5). Stacking and unstacking, merging and
//! ungrouping, cycling and reordering tabs, session locks, and the stackbar
//! descriptions a backend draws.
//!
//! The functions change workspace membership and tab focus only. The caller
//! lays the workspace out and applies the visibility they report.

use crate::outputs::Outputs;
use crate::workspace::Workspace;
use gevrek_model::geometry::Rect;
use gevrek_model::ids::{OutputId, WindowId};
use std::collections::HashSet;

/// A group for locking purposes: output, workspace id, container id.
pub type GroupKey = (OutputId, usize, usize);
/// Groups whose membership is locked for this session.
pub type GroupLocks = HashSet<GroupKey>;

/// Why a group operation was refused. The messages are user-facing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    NotManaged,
    WorkspaceGone,
    Floating,
    GroupGone,
    NotInGroup,
    GroupEmpty,
    NoTab { index: usize, tabs: usize },
    NoNeighbor,
    TargetGone,
    SourceGone,
    BothLocked,
    Locked,
    NoFocusedWorkspace,
    NothingToStack,
    UnlockAllFirst,
    FocusedContainerGone,
    FocusedNotGroup,
    UnlockBeforeUnstack,
}

impl std::fmt::Display for GroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotManaged => write!(f, "The selected window is not managed"),
            Self::WorkspaceGone => write!(f, "The selected workspace no longer exists"),
            Self::Floating => write!(f, "The selected window is floating"),
            Self::GroupGone => write!(f, "The selected group no longer exists"),
            Self::NotInGroup => write!(f, "The selected window is not in a group"),
            Self::GroupEmpty => write!(f, "The selected group is empty"),
            Self::NoTab { index, tabs } => write!(
                f,
                "Group tab index {index} does not exist; the group has {tabs} tab(s)"
            ),
            Self::NoNeighbor => write!(f, "There is no neighboring container to merge with"),
            Self::TargetGone => write!(f, "The target group no longer exists"),
            Self::SourceGone => write!(f, "The source group no longer exists"),
            Self::BothLocked => write!(f, "Unlock both groups before changing their membership"),
            Self::Locked => write!(
                f,
                "Unlock the selected group before changing its membership"
            ),
            Self::NoFocusedWorkspace => write!(f, "The active monitor has no focused workspace"),
            Self::NothingToStack => write!(f, "There is nothing to stack on this workspace"),
            Self::UnlockAllFirst => write!(f, "Unlock all groups before stacking the workspace"),
            Self::FocusedContainerGone => write!(f, "The focused container no longer exists"),
            Self::FocusedNotGroup => write!(f, "The focused container is not a group"),
            Self::UnlockBeforeUnstack => write!(f, "Unlock the group before unstacking it"),
        }
    }
}

impl std::error::Error for GroupError {}

/// Where a tiled window's container is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupLocation {
    pub output: OutputId,
    pub workspace_index: usize,
    pub workspace_id: usize,
    pub container_id: usize,
    /// The workspace is the visible one on its output.
    pub visible: bool,
}

impl GroupLocation {
    pub fn key(&self) -> GroupKey {
        (self.output, self.workspace_id, self.container_id)
    }
}

/// Find the container of a tiled window.
pub fn locate(outputs: &Outputs, window: WindowId) -> Result<GroupLocation, GroupError> {
    let (output, workspace_index) = outputs.find_window(window).ok_or(GroupError::NotManaged)?;
    let workspaces = outputs
        .get_workspaces(output)
        .ok_or(GroupError::WorkspaceGone)?;
    let workspace = workspaces
        .get(workspace_index)
        .ok_or(GroupError::WorkspaceGone)?;
    let container_index = workspace
        .find_container(window)
        .ok_or(GroupError::Floating)?;
    let container_id = workspace
        .containers
        .get(container_index)
        .map(|container| container.id)
        .ok_or(GroupError::GroupGone)?;
    Ok(GroupLocation {
        output,
        workspace_index,
        workspace_id: workspace.id,
        container_id,
        visible: workspaces.focused_idx() == workspace_index,
    })
}

fn workspace_mut<'a>(
    outputs: &'a mut Outputs,
    location: &GroupLocation,
) -> Result<&'a mut Workspace, GroupError> {
    outputs
        .get_workspaces_mut(location.output)
        .and_then(|workspaces| workspaces.get_mut(location.workspace_index))
        .ok_or(GroupError::WorkspaceGone)
}

/// Whether the window shares its container with other windows.
pub fn is_grouped(outputs: &Outputs, window: WindowId) -> Result<bool, GroupError> {
    let location = locate(outputs, window)?;
    Ok(outputs
        .get_workspaces(location.output)
        .and_then(|workspaces| workspaces.get(location.workspace_index))
        .and_then(|workspace| {
            workspace
                .find_container(window)
                .and_then(|index| workspace.containers.get(index))
        })
        .is_some_and(|container| container.windows.len() > 1))
}

/// Lock or unlock the group of `window`. Returns whether it is now locked.
pub fn toggle_lock(
    outputs: &Outputs,
    locks: &mut GroupLocks,
    window: WindowId,
) -> Result<bool, GroupError> {
    let key = locate(outputs, window)?.key();
    if locks.remove(&key) {
        Ok(false)
    } else {
        locks.insert(key);
        Ok(true)
    }
}

/// A workspace moved to another output: its locks follow.
pub fn move_workspace_locks(
    locks: &mut GroupLocks,
    workspace_id: usize,
    from: OutputId,
    to: OutputId,
) {
    *locks = locks
        .drain()
        .map(|(output, id, container)| {
            if output == from && id == workspace_id {
                (to, id, container)
            } else {
                (output, id, container)
            }
        })
        .collect();
}

/// The tab that shows after a tab change, and the tabs that hide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabChange {
    pub location: GroupLocation,
    pub selected: WindowId,
    pub hidden: Vec<WindowId>,
}

/// Show the next or previous tab of the group of `window`.
pub fn cycle(
    outputs: &mut Outputs,
    window: WindowId,
    forward: bool,
) -> Result<TabChange, GroupError> {
    let location = locate(outputs, window)?;
    let workspace = workspace_mut(outputs, &location)?;
    let container_index = workspace
        .find_container(window)
        .ok_or(GroupError::Floating)?;
    let container = workspace
        .containers
        .get_mut(container_index)
        .ok_or(GroupError::GroupGone)?;
    if container.windows.len() < 2 {
        return Err(GroupError::NotInGroup);
    }
    container.focus_window(window);
    if forward {
        container.windows.focus_next();
    } else {
        container.windows.focus_previous();
    }
    Ok(TabChange {
        location,
        selected: container.focused_window().ok_or(GroupError::GroupEmpty)?,
        hidden: container.non_focused_windows(),
    })
}

/// Show tab `index` of the group of `window`.
pub fn focus_tab(
    outputs: &mut Outputs,
    window: WindowId,
    index: usize,
) -> Result<TabChange, GroupError> {
    let location = locate(outputs, window)?;
    let workspace = workspace_mut(outputs, &location)?;
    let container_index = workspace
        .find_container(window)
        .ok_or(GroupError::Floating)?;
    let container = workspace
        .containers
        .get_mut(container_index)
        .ok_or(GroupError::GroupGone)?;
    if container.windows.len() < 2 {
        return Err(GroupError::NotInGroup);
    }
    if !container.windows.focus_at(index) {
        return Err(GroupError::NoTab {
            index,
            tabs: container.windows.len(),
        });
    }
    Ok(TabChange {
        location,
        selected: container.focused_window().ok_or(GroupError::GroupEmpty)?,
        hidden: container.non_focused_windows(),
    })
}

/// Move `window` one place forward or back in its group's tab order. The
/// focused tab stays focused.
pub fn move_tab(outputs: &mut Outputs, window: WindowId, forward: bool) -> Result<(), GroupError> {
    let location = locate(outputs, window)?;
    let workspace = workspace_mut(outputs, &location)?;
    let container_index = workspace
        .find_container(window)
        .ok_or(GroupError::Floating)?;
    let container = workspace
        .containers
        .get_mut(container_index)
        .ok_or(GroupError::GroupGone)?;
    if container.windows.len() < 2 {
        return Err(GroupError::NotInGroup);
    }
    let focused = container.focused_window();
    container.focus_window(window);
    if forward {
        container.windows.swap_with_next();
    } else {
        container.windows.swap_with_previous();
    }
    if let Some(focused) = focused {
        container.focus_window(focused);
    }
    Ok(())
}

/// Merge the container of `window` into its next or previous neighbour
/// (wrapping). `window` becomes the group's visible tab.
pub fn merge(
    outputs: &mut Outputs,
    locks: &GroupLocks,
    window: WindowId,
    forward: bool,
) -> Result<GroupLocation, GroupError> {
    let location = locate(outputs, window)?;
    let workspace = workspace_mut(outputs, &location)?;
    let source_index = workspace
        .find_container(window)
        .ok_or(GroupError::Floating)?;
    let count = workspace.containers.len();
    if count < 2 {
        return Err(GroupError::NoNeighbor);
    }
    let target_index = if forward {
        (source_index + 1) % count
    } else if source_index == 0 {
        count - 1
    } else {
        source_index - 1
    };
    let target_id = workspace
        .containers
        .get(target_index)
        .map(|container| container.id)
        .ok_or(GroupError::TargetGone)?;
    if locks.contains(&location.key())
        || locks.contains(&(location.output, location.workspace_id, target_id))
    {
        return Err(GroupError::BothLocked);
    }

    let source = workspace
        .containers
        .remove_at(source_index)
        .ok_or(GroupError::SourceGone)?;
    let adjusted_target = if source_index < target_index {
        target_index - 1
    } else {
        target_index
    };
    let target = workspace
        .containers
        .get_mut(adjusted_target)
        .ok_or(GroupError::TargetGone)?;
    for moved in source.all_windows() {
        target.add_window(moved);
    }
    let selected_index = target
        .windows
        .elements()
        .iter()
        .position(|candidate| *candidate == window);
    if let Some(selected_index) = selected_index {
        target.windows.focus_at(selected_index);
    }
    workspace.containers.focus_at(adjusted_target);
    workspace.split_ratios.clear();
    Ok(location)
}

/// Take `window` out of its group into its own container, right after the
/// group, and focus it.
pub fn ungroup(
    outputs: &mut Outputs,
    locks: &GroupLocks,
    window: WindowId,
) -> Result<GroupLocation, GroupError> {
    let location = locate(outputs, window)?;
    if locks.contains(&location.key()) {
        return Err(GroupError::Locked);
    }
    let workspace = workspace_mut(outputs, &location)?;
    let container_index = workspace
        .find_container(window)
        .ok_or(GroupError::Floating)?;
    let container = workspace
        .containers
        .get_mut(container_index)
        .ok_or(GroupError::GroupGone)?;
    if container.windows.len() < 2 {
        return Err(GroupError::NotInGroup);
    }
    container.remove_window(window);
    workspace.add_window_at(window, container_index + 1);
    workspace.focus_container(container_index + 1);
    workspace.split_ratios.clear();
    Ok(location)
}

/// Merge every container of the visible workspace of
/// `output` into one group. Returns the window that stays visible.
pub fn stack_all(
    outputs: &mut Outputs,
    locks: &GroupLocks,
    output: OutputId,
) -> Result<Option<WindowId>, GroupError> {
    let workspace = outputs
        .get_focused_workspace_mut(output)
        .ok_or(GroupError::NoFocusedWorkspace)?;
    if workspace.containers.len() < 2 {
        return Err(GroupError::NothingToStack);
    }
    let workspace_id = workspace.id;
    if workspace
        .containers
        .iter()
        .any(|container| locks.contains(&(output, workspace_id, container.id)))
    {
        return Err(GroupError::UnlockAllFirst);
    }

    let target_index = workspace.containers.focused_idx();
    let focused = workspace
        .containers
        .get(target_index)
        .and_then(|container| container.focused_window());
    let mut target = workspace
        .containers
        .remove_at(target_index)
        .ok_or(GroupError::FocusedContainerGone)?;
    let windows: Vec<WindowId> = workspace
        .containers
        .iter()
        .flat_map(|container| container.all_windows())
        .collect();
    for window in windows {
        target.add_window(window);
    }
    if let Some(focused) = focused {
        target.focus_window(focused);
    }
    workspace.containers.clear();
    workspace.containers.push(target);
    workspace.containers.focus_at(0);
    workspace.split_ratios.clear();
    workspace.bsp_split_ratios.clear();
    Ok(focused)
}

/// Give every window of the focused group of
/// `output` its own container, keeping the tab order. Returns the window
/// that was visible and how many windows the group had.
pub fn unstack_all(
    outputs: &mut Outputs,
    locks: &GroupLocks,
    output: OutputId,
) -> Result<(Option<WindowId>, usize), GroupError> {
    let workspace = outputs
        .get_focused_workspace_mut(output)
        .ok_or(GroupError::NoFocusedWorkspace)?;
    let focused_index = workspace.containers.focused_idx();
    let container = workspace
        .containers
        .get(focused_index)
        .ok_or(GroupError::FocusedContainerGone)?;
    if container.windows.len() < 2 {
        return Err(GroupError::FocusedNotGroup);
    }
    if locks.contains(&(output, workspace.id, container.id)) {
        return Err(GroupError::UnlockBeforeUnstack);
    }

    let container = workspace
        .containers
        .remove_at(focused_index)
        .ok_or(GroupError::FocusedContainerGone)?;
    let focused = container.focused_window();
    let windows = container.all_windows();
    let focus_offset = focused
        .and_then(|focused| windows.iter().position(|window| *window == focused))
        .unwrap_or(0);
    for (offset, window) in windows.iter().enumerate() {
        workspace.add_window_at(*window, focused_index + offset);
    }
    workspace.focus_container(focused_index + focus_offset);
    workspace.split_ratios.clear();
    Ok((focused, windows.len()))
}

/// The stackbar of one group, for the backend to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct StackbarGroup {
    /// The group's visible window; the bar follows it.
    pub key: WindowId,
    /// The container's rect; the bar takes the top strip.
    pub rect: Rect,
    /// The tabs, in order.
    pub tabs: Vec<WindowId>,
    pub focused_tab: usize,
}

/// The stackbars a workspace should show: one per laid-out container with
/// two or more windows, none when the stackbar is disabled.
pub fn stackbars(workspace: &Workspace) -> Vec<StackbarGroup> {
    if !workspace.stackbar.enabled {
        return Vec::new();
    }
    workspace
        .containers
        .iter()
        .filter(|container| {
            container.windows.len() >= 2 && container.rect.width > 0 && container.rect.height > 0
        })
        .filter_map(|container| {
            container.focused_window().map(|key| StackbarGroup {
                key,
                rect: container.rect,
                tabs: container.all_windows(),
                focused_tab: container.windows.focused_idx(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::LayoutType;

    const A: OutputId = OutputId(1);
    const W1: WindowId = WindowId(1);
    const W2: WindowId = WindowId(2);
    const W3: WindowId = WindowId(3);

    /// Output A, one visible workspace with W1, W2, W3 in their own
    /// containers, W3 focused.
    fn three_windows() -> Outputs {
        let mut outputs = Outputs::new();
        outputs.add_test_monitor(A, "A");
        outputs.create_workspace(A, "one".into(), LayoutType::BSP);
        outputs.focus_workspace_at(A, 0);
        let workspace = outputs.get_focused_workspace_mut(A).unwrap();
        for window in [W1, W2, W3] {
            workspace.add_window(window, true);
        }
        workspace.focus_window(W3);
        outputs
    }

    fn containers(outputs: &Outputs) -> Vec<Vec<WindowId>> {
        outputs
            .get_focused_workspace(A)
            .unwrap()
            .containers
            .iter()
            .map(|container| container.all_windows())
            .collect()
    }

    #[test]
    fn stack_all_and_unstack_all_round_trip() {
        let mut outputs = three_windows();
        let locks = GroupLocks::new();
        assert_eq!(stack_all(&mut outputs, &locks, A), Ok(Some(W3)));
        assert_eq!(containers(&outputs).len(), 1);
        let ws = outputs.get_focused_workspace(A).unwrap();
        assert_eq!(
            ws.focused_window(),
            Some(W3),
            "the focused window stays visible"
        );
        assert_eq!(
            stack_all(&mut outputs, &locks, A),
            Err(GroupError::NothingToStack)
        );

        let (focused, count) = unstack_all(&mut outputs, &locks, A).unwrap();
        assert_eq!((focused, count), (Some(W3), 3));
        assert_eq!(containers(&outputs).len(), 3);
        assert_eq!(
            outputs.get_focused_workspace(A).unwrap().focused_window(),
            Some(W3)
        );
        assert_eq!(
            unstack_all(&mut outputs, &locks, A),
            Err(GroupError::FocusedNotGroup)
        );
    }

    #[test]
    fn merging_makes_the_window_the_visible_tab_and_ungrouping_splits_it_off() {
        let mut outputs = three_windows();
        let locks = GroupLocks::new();
        merge(&mut outputs, &locks, W1, true).unwrap();
        assert_eq!(containers(&outputs), vec![vec![W2, W1], vec![W3]]);
        assert_eq!(
            outputs.get_focused_workspace(A).unwrap().focused_window(),
            Some(W1)
        );
        assert_eq!(is_grouped(&outputs, W2), Ok(true));
        assert_eq!(is_grouped(&outputs, W3), Ok(false));

        ungroup(&mut outputs, &locks, W1).unwrap();
        assert_eq!(containers(&outputs), vec![vec![W2], vec![W1], vec![W3]]);
        assert_eq!(
            ungroup(&mut outputs, &locks, W1),
            Err(GroupError::NotInGroup)
        );
    }

    #[test]
    fn merging_backwards_from_the_first_container_wraps() {
        let mut outputs = three_windows();
        merge(&mut outputs, &GroupLocks::new(), W1, false).unwrap();
        assert_eq!(containers(&outputs), vec![vec![W2], vec![W3, W1]]);
    }

    #[test]
    fn tabs_cycle_focus_by_index_and_reorder() {
        let mut outputs = three_windows();
        let locks = GroupLocks::new();
        stack_all(&mut outputs, &locks, A).unwrap();
        // Tab order after stacking: W3 (focused container first), W1, W2.
        assert_eq!(containers(&outputs), vec![vec![W3, W1, W2]]);

        let change = cycle(&mut outputs, W3, true).unwrap();
        assert_eq!(change.selected, W1);
        assert_eq!(change.hidden, vec![W3, W2]);
        assert!(change.location.visible);
        assert_eq!(cycle(&mut outputs, W1, false).unwrap().selected, W3);

        assert_eq!(focus_tab(&mut outputs, W3, 2).unwrap().selected, W2);
        assert_eq!(
            focus_tab(&mut outputs, W3, 9).unwrap_err().to_string(),
            "Group tab index 9 does not exist; the group has 3 tab(s)"
        );

        move_tab(&mut outputs, W3, true).unwrap();
        assert_eq!(containers(&outputs), vec![vec![W1, W3, W2]]);
        assert_eq!(
            outputs.get_focused_workspace(A).unwrap().focused_window(),
            Some(W2),
            "reordering keeps the visible tab"
        );
    }

    #[test]
    fn locked_groups_refuse_membership_changes_and_locks_follow_a_moved_workspace() {
        let mut outputs = three_windows();
        let mut locks = GroupLocks::new();
        merge(&mut outputs, &locks, W1, true).unwrap();
        assert_eq!(toggle_lock(&outputs, &mut locks, W1), Ok(true));

        assert_eq!(ungroup(&mut outputs, &locks, W1), Err(GroupError::Locked));
        assert_eq!(
            merge(&mut outputs, &locks, W3, false),
            Err(GroupError::BothLocked)
        );
        assert_eq!(
            stack_all(&mut outputs, &locks, A),
            Err(GroupError::UnlockAllFirst)
        );
        outputs
            .get_focused_workspace_mut(A)
            .unwrap()
            .focus_window(W1);
        assert_eq!(
            unstack_all(&mut outputs, &locks, A),
            Err(GroupError::UnlockBeforeUnstack)
        );

        let key = *locks.iter().next().unwrap();
        move_workspace_locks(&mut locks, key.1, A, OutputId(2));
        assert!(locks.contains(&(OutputId(2), key.1, key.2)));

        let mut locks = GroupLocks::from([key]);
        assert_eq!(toggle_lock(&outputs, &mut locks, W2), Ok(false), "unlocked");
    }

    #[test]
    fn a_stackbar_is_described_for_each_laid_out_group() {
        let mut outputs = three_windows();
        merge(&mut outputs, &GroupLocks::new(), W1, true).unwrap();
        let workspace = outputs.get_focused_workspace_mut(A).unwrap();
        assert!(stackbars(workspace).is_empty(), "not laid out yet");

        workspace.stackbar.enabled = true;
        workspace.apply_layout(Rect::new(0, 0, 1920, 1040));
        let bars = stackbars(workspace);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].key, W1);
        assert_eq!(bars[0].tabs, vec![W2, W1]);
        assert_eq!(bars[0].focused_tab, 1);

        workspace.stackbar.enabled = false;
        assert!(stackbars(workspace).is_empty());
    }

    #[test]
    fn floating_and_unmanaged_windows_have_no_group() {
        let mut outputs = three_windows();
        outputs
            .get_focused_workspace_mut(A)
            .unwrap()
            .add_floating_window(WindowId(9));
        assert_eq!(locate(&outputs, WindowId(9)), Err(GroupError::Floating));
        assert_eq!(locate(&outputs, WindowId(8)), Err(GroupError::NotManaged));
    }
}
