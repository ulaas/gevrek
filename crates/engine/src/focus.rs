//! Focus policy: what the WM does when a window takes focus, which window
//! gets focus back, focusing a managed window, and which output is active
//! (docs/PHASE2_PLAN.md, step 4.4).
//!
//! The rule flags live in `WindowState` (`stay_focused`, `no_focus`,
//! `no_initial_focus`, `no_follow_mouse`). Facts only the backend knows
//! (whether a window is visible, which window is in the foreground) come in
//! as arguments or closures.

use crate::focus_manager::FocusManager;
use crate::lifecycle::{Changes, World};
use crate::outputs::Outputs;
use crate::rule_state::WindowStateMap;
use crate::workspaces::{hide_windows, show_windows};
use gevrek_model::ids::{OutputId, WindowId};

/// What to do about a window that just took focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDecision {
    /// A visible stay-focused window takes focus back.
    Enforce(WindowId),
    /// The window never keeps focus (`no_focus` rule): focus goes to
    /// `restore`, or to the desktop when there is no candidate. The window
    /// is dropped from the focus history.
    Reject { restore: Option<WindowId> },
    /// The window must not take focus when it opens (`no_initial_focus`,
    /// one-shot): focus goes back to `restore` if there is one.
    SuppressInitial { restore: Option<WindowId> },
    /// The window keeps focus. `output` is its output when it is managed;
    /// the workspace focus already follows it.
    Accept { output: Option<OutputId> },
}

/// A window took focus (click, Alt-Tab, or the app activated itself).
/// `is_visible` tells whether a window exists and is visible.
pub fn on_focused(
    world: &mut World,
    history: &FocusManager,
    id: WindowId,
    is_visible: impl Fn(WindowId) -> bool,
) -> FocusDecision {
    // (A window that is itself to keep the keyboard keeps it: with two
    // such windows on screen each would else take it from the other, on
    // and on.)
    let enforced = if world.windows.get(id).stay_focused.enabled() {
        None
    } else {
        world
            .windows
            .ids_where(|state| state.stay_focused.enabled() && !state.minimized)
            .into_iter()
            .find(|candidate| *candidate != id && is_visible(*candidate))
    };
    if let Some(target) = enforced {
        return FocusDecision::Enforce(target);
    }
    if world.windows.get(id).rejects_focus() {
        world.windows.get_mut(id).initial_focus_suppression_pending = false;
        return FocusDecision::Reject {
            restore: previous_candidate(world.windows, history, id, &is_visible),
        };
    }
    if std::mem::take(&mut world.windows.get_mut(id).initial_focus_suppression_pending) {
        return FocusDecision::SuppressInitial {
            restore: previous_candidate(world.windows, history, id, &is_visible),
        };
    }
    FocusDecision::Accept {
        output: accept(world.outputs, id),
    }
}

/// The most recently focused window that can take focus back: not
/// `excluded`, visible, and not rejecting focus by rule.
pub fn previous_candidate(
    windows: &WindowStateMap,
    history: &FocusManager,
    excluded: WindowId,
    is_visible: impl Fn(WindowId) -> bool,
) -> Option<WindowId> {
    history.history().copied().find(|candidate| {
        *candidate != excluded && is_visible(*candidate) && !windows.get(*candidate).rejects_focus()
    })
}

/// Keep the workspace's focused container and tab in sync with the window
/// the user activated, so focus commands start from it. Only on the visible
/// workspace: a focus event from a window on a hidden workspace must not
/// silently retarget it. Returns the window's output if it is managed.
pub fn accept(outputs: &mut Outputs, id: WindowId) -> Option<OutputId> {
    let (output, index) = outputs.find_window(id)?;
    let visible = outputs
        .get_workspaces(output)
        .is_some_and(|workspaces| workspaces.focused_idx() == index);
    if visible {
        if let Some(workspace) = outputs.get_focused_workspace_mut(output) {
            workspace.focus_window(id);
        }
    }
    Some(output)
}

/// What a new window's focus rules ask for when it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialFocus {
    /// No rule: it may take focus.
    Allowed,
    /// It is not focused yet; its focus event will be suppressed once.
    Pending,
    /// It already has focus: give focus back now. `never` is set for
    /// `no_focus` windows, which also leave the focus history.
    SuppressNow { never: bool },
}

/// Arm the initial-focus suppression for a new window. The backend may
/// report focus before or after creation, so `is_active` says whether the
/// window already has focus.
pub fn arm_initial_suppression(
    windows: &mut WindowStateMap,
    id: WindowId,
    is_active: bool,
) -> InitialFocus {
    let never = windows.get(id).rejects_focus();
    if !windows.get(id).suppresses_initial_focus() && !never {
        return InitialFocus::Allowed;
    }
    windows.get_mut(id).initial_focus_suppression_pending = !is_active;
    if is_active {
        InitialFocus::SuppressNow { never }
    } else {
        InitialFocus::Pending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusError {
    RejectedByRule(WindowId),
    WorkspaceGone,
    NotInWorkspace,
}

impl std::fmt::Display for FocusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RejectedByRule(id) => write!(f, "Window {id:?} rejects focus by rule"),
            Self::WorkspaceGone => write!(f, "The target workspace disappeared"),
            Self::NotInWorkspace => write!(f, "The selected window is no longer in its workspace"),
        }
    }
}

impl std::error::Error for FocusError {}

/// Where a window the WM is about to focus lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusTarget {
    pub output: OutputId,
    pub workspace: usize,
    /// Its workspace is hidden: the caller switches to it first.
    pub needs_switch: bool,
}

/// Check that the WM may focus `id` and find it. `bypass_no_focus` skips the
/// `no_focus` rule for one explicit action.
/// `Ok(None)`: the window is not managed and can be focused as it is.
pub fn locate_target(
    world: &World,
    id: WindowId,
    bypass_no_focus: bool,
) -> Result<Option<FocusTarget>, FocusError> {
    if !bypass_no_focus && world.windows.get(id).rejects_focus() {
        return Err(FocusError::RejectedByRule(id));
    }
    Ok(world.outputs.find_window(id).map(|(output, workspace)| {
        let current = world
            .outputs
            .get_workspaces(output)
            .map(|workspaces| workspaces.focused_idx());
        FocusTarget {
            output,
            workspace,
            needs_switch: current != Some(workspace),
        }
    }))
}

/// Make `id` the focused window of the visible workspace of `output`: its
/// container's other tabs are hidden and it is shown.
pub fn bring_forward(
    world: &mut World,
    id: WindowId,
    output: OutputId,
    changes: &mut Changes,
) -> Result<(), FocusError> {
    let workspace = world
        .outputs
        .get_focused_workspace_mut(output)
        .ok_or(FocusError::WorkspaceGone)?;
    if !workspace.focus_window(id) {
        return Err(FocusError::NotInWorkspace);
    }
    let hidden_tabs = workspace
        .containers
        .iter()
        .find(|container| container.contains_window(id))
        .map(|container| container.non_focused_windows())
        .unwrap_or_default();
    hide_windows(world.windows, &hidden_tabs, changes);
    show_windows(world.windows, &[id], changes);
    Ok(())
}

/// The window that gets keyboard focus when `output` is focused: the
/// focused tiled window of its visible workspace, else the focused floating
/// one.
pub fn output_focus_target(outputs: &Outputs, output: OutputId) -> Option<WindowId> {
    let workspace = outputs.get_focused_workspace(output)?;
    workspace
        .focused_window()
        .or_else(|| workspace.floating_windows.focused().copied())
}

/// The window that takes the keyboard when the focused window of
/// `output`'s visible workspace is gone: the one used last among those
/// still on that workspace, else the workspace's own focused window.
/// `usable` says whether a window can be focused at all (it exists and is
/// shown); a window that rejects focus by rule is never chosen.
///
/// A system left to itself gives the focus to whatever is next in its own
/// order, which may be a window on another display.
pub fn successor(
    outputs: &Outputs,
    windows: &WindowStateMap,
    history: &FocusManager,
    output: OutputId,
    usable: impl Fn(WindowId) -> bool,
) -> Option<WindowId> {
    let visible = outputs.get_workspaces(output)?.focused_idx();
    let fits = |candidate: WindowId| {
        outputs.find_window(candidate) == Some((output, visible))
            && usable(candidate)
            && !windows.get(candidate).rejects_focus()
    };
    history
        .history()
        .copied()
        .find(|candidate| fits(*candidate))
        .or_else(|| output_focus_target(outputs, output).filter(|candidate| fits(*candidate)))
}

/// The floating windows that have to be put above again after `focused`
/// took the keyboard, in the order they lie in their workspace: floating
/// windows stay above the tiled ones, and a system that raises whatever
/// gets the keyboard (Windows) has just put a tiled window over them.
/// None when the window is floating itself, is not gevrek's, or is on a
/// workspace that is not shown.
pub fn floating_to_keep_above(outputs: &Outputs, focused: WindowId) -> Vec<WindowId> {
    let Some((output, index)) = outputs.find_window(focused) else {
        return Vec::new();
    };
    let Some(workspaces) = outputs.get_workspaces(output) else {
        return Vec::new();
    };
    if workspaces.focused_idx() != index {
        return Vec::new();
    }
    let Some(workspace) = workspaces.focused() else {
        return Vec::new();
    };
    if workspace
        .floating_windows
        .iter()
        .any(|window| *window == focused)
    {
        return Vec::new();
    }
    workspace.floating_windows.iter().copied().collect()
}

/// The output commands act on: the logically focused one while it is
/// attached, else the output of the foreground window (as a workspace
/// member, else by position), else the primary output.
pub fn active_output(
    outputs: &Outputs,
    focused_output: Option<OutputId>,
    foreground: impl FnOnce() -> Option<WindowId>,
    output_at_window: impl FnOnce(WindowId) -> Option<OutputId>,
) -> Option<OutputId> {
    focused_output
        .filter(|output| outputs.get_monitor(*output).is_some())
        .or_else(|| {
            let window = foreground()?;
            outputs
                .find_window(window)
                .map(|(output, _)| output)
                .or_else(|| output_at_window(window))
        })
        .or_else(|| outputs.get_primary_monitor_handle())
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
        history: FocusManager,
    }

    impl Fixture {
        fn new() -> Self {
            let mut outputs = Outputs::new();
            outputs.add_test_monitor(A, "A");
            outputs.add_test_monitor(B, "B");
            outputs.create_workspace(A, "one".into(), LayoutType::BSP);
            outputs.create_workspace(A, "two".into(), LayoutType::BSP);
            outputs.create_workspace(B, "b".into(), LayoutType::BSP);
            outputs.focus_workspace_at(A, 0);
            Self {
                outputs,
                windows: WindowStateMap::new(),
                history: FocusManager::new(),
            }
        }

        fn add(&mut self, output: OutputId, index: usize, id: WindowId) {
            let workspaces = self.outputs.get_workspaces_mut(output).unwrap();
            workspaces.get_mut(index).unwrap().add_window(id, true);
        }

        fn focused(&mut self, id: WindowId) -> FocusDecision {
            let decision = on_focused(
                &mut World {
                    outputs: &mut self.outputs,
                    windows: &mut self.windows,
                },
                &self.history,
                id,
                |_| true,
            );
            if matches!(decision, FocusDecision::Accept { .. }) {
                self.history.record_focus(id);
            }
            decision
        }
    }

    const W1: WindowId = WindowId(1);
    const W2: WindowId = WindowId(2);
    const W3: WindowId = WindowId(3);

    #[test]
    fn floating_windows_are_put_above_a_tiled_window_that_took_the_keyboard() {
        const F1: WindowId = WindowId(11);
        const F2: WindowId = WindowId(12);
        let mut f = Fixture::new();
        f.add(A, 0, W1);
        f.add(A, 1, W2);
        f.add(B, 0, W3);
        let workspaces = f.outputs.get_workspaces_mut(A).unwrap();
        workspaces.get_mut(0).unwrap().add_floating_window(F1);
        workspaces.get_mut(0).unwrap().add_floating_window(F2);
        // A tiled window of the workspace that is shown: its floating ones.
        assert_eq!(floating_to_keep_above(&f.outputs, W1), [F1, F2]);
        // A floating window took the keyboard: it is above already.
        assert!(floating_to_keep_above(&f.outputs, F1).is_empty());
        // A workspace that is not shown, another display's, nobody's window.
        assert!(floating_to_keep_above(&f.outputs, W2).is_empty());
        assert!(floating_to_keep_above(&f.outputs, W3).is_empty());
        assert!(floating_to_keep_above(&f.outputs, WindowId(99)).is_empty());
    }

    #[test]
    fn when_the_focused_window_goes_the_one_used_before_it_there_follows() {
        const W4: WindowId = WindowId(4);
        let mut f = Fixture::new();
        f.add(A, 0, W1);
        f.add(A, 0, W2);
        f.add(A, 1, W3); // a hidden workspace of the same output
        f.add(B, 0, W4); // the other display
        for id in [W1, W3, W4, W2] {
            f.history.record_focus(id);
        }
        let next = |f: &Fixture, usable: &dyn Fn(WindowId) -> bool| {
            successor(&f.outputs, &f.windows, &f.history, A, usable)
        };
        // W2 was focused and is gone: not W4 (used last, but elsewhere),
        // not W3 (hidden), but W1 on the same workspace.
        f.history.remove_window(W2);
        f.outputs
            .get_workspaces_mut(A)
            .unwrap()
            .get_mut(0)
            .unwrap()
            .remove_window(W2);
        assert_eq!(next(&f, &|_| true), Some(W1));
        // A window that cannot be focused, or must not be, is passed over.
        assert_eq!(next(&f, &|id| id != W1), None);
        f.windows.get_mut(W1).no_focus.static_value = Some(true);
        assert_eq!(next(&f, &|_| true), None);
        // The other display has its own answer.
        assert_eq!(
            successor(&f.outputs, &f.windows, &f.history, B, |_| true),
            Some(W4)
        );
    }

    #[test]
    fn a_window_never_focused_before_still_follows_a_closed_one() {
        let mut f = Fixture::new();
        f.add(A, 0, W1);
        // Nothing in the history: the workspace's own focused window.
        assert_eq!(
            successor(&f.outputs, &f.windows, &f.history, A, |_| true),
            Some(W1)
        );
        // An empty workspace has nobody to follow.
        assert_eq!(
            successor(&f.outputs, &f.windows, &f.history, B, |_| true),
            None
        );
    }

    #[test]
    fn an_ordinary_focus_is_accepted_and_the_workspace_follows() {
        let mut f = Fixture::new();
        f.add(A, 0, W1);
        f.add(A, 0, W2);
        assert_eq!(f.focused(W1), FocusDecision::Accept { output: Some(A) });
        assert_eq!(
            f.outputs.get_focused_workspace(A).unwrap().focused_window(),
            Some(W1)
        );
        assert_eq!(
            f.focused(WindowId(99)),
            FocusDecision::Accept { output: None }
        );
    }

    #[test]
    fn focus_on_a_hidden_workspace_does_not_retarget_it() {
        let mut f = Fixture::new();
        f.add(A, 1, W1);
        f.add(A, 1, W2);
        let before = f
            .outputs
            .get_workspaces(A)
            .unwrap()
            .get(1)
            .unwrap()
            .focused_window();
        let other = if before == Some(W1) { W2 } else { W1 };
        assert_eq!(f.focused(other), FocusDecision::Accept { output: Some(A) });
        let after = f
            .outputs
            .get_workspaces(A)
            .unwrap()
            .get(1)
            .unwrap()
            .focused_window();
        assert_eq!(after, before);
    }

    #[test]
    fn a_stay_focused_window_takes_focus_back() {
        let mut f = Fixture::new();
        f.windows.get_mut(W1).stay_focused.static_value = Some(true);
        assert_eq!(f.focused(W2), FocusDecision::Enforce(W1));
        assert_eq!(f.focused(W1), FocusDecision::Accept { output: None });
    }

    #[test]
    fn a_no_focus_window_hands_focus_to_the_previous_eligible_window() {
        let mut f = Fixture::new();
        f.focused(W1);
        f.focused(W2);
        f.windows.get_mut(W2).no_focus.static_value = Some(true);
        f.windows.get_mut(W3).no_focus.static_value = Some(true);
        // W2 is the most recent but rejects focus itself, so W1 gets it.
        assert_eq!(f.focused(W3), FocusDecision::Reject { restore: Some(W1) });

        // stay_focused wins over no_focus on the same window.
        f.windows.get_mut(W3).stay_focused.dynamic_value = Some(true);
        assert_eq!(f.focused(W3), FocusDecision::Accept { output: None });
    }

    #[test]
    fn initial_focus_is_suppressed_once_in_either_event_order() {
        let mut f = Fixture::new();
        f.focused(W1);
        f.windows.get_mut(W2).no_initial_focus.static_value = Some(true);

        // Created before focused: pending, then consumed by the focus event.
        assert_eq!(
            arm_initial_suppression(&mut f.windows, W2, false),
            InitialFocus::Pending
        );
        assert_eq!(
            f.focused(W2),
            FocusDecision::SuppressInitial { restore: Some(W1) }
        );
        assert_eq!(
            f.focused(W2),
            FocusDecision::Accept { output: None },
            "one-shot"
        );

        // Focused before created: suppressed right away, nothing pending.
        assert_eq!(
            arm_initial_suppression(&mut f.windows, W2, true),
            InitialFocus::SuppressNow { never: false }
        );
        assert!(!f.windows.get(W2).initial_focus_suppression_pending);
        assert_eq!(
            arm_initial_suppression(&mut f.windows, W3, true),
            InitialFocus::Allowed
        );
    }

    #[test]
    fn focusing_a_managed_window_finds_its_workspace_and_fronts_its_tab() {
        let mut f = Fixture::new();
        f.add(A, 1, W1);
        let ws = f.outputs.get_workspaces_mut(A).unwrap().get_mut(1).unwrap();
        ws.containers.focused_mut().unwrap().add_window(W2);
        ws.containers.focused_mut().unwrap().focus_window(W2);
        let mut world = World {
            outputs: &mut f.outputs,
            windows: &mut f.windows,
        };

        let target = locate_target(&world, W1, false).unwrap().unwrap();
        assert_eq!(
            (target.output, target.workspace, target.needs_switch),
            (A, 1, true)
        );
        world.outputs.focus_workspace_at(A, 1);

        let mut changes = Changes::default();
        bring_forward(&mut world, W1, A, &mut changes).unwrap();
        assert!(
            world.windows.get(W2).hidden_by_wm,
            "the other tab is hidden"
        );
        assert_eq!(output_focus_target(world.outputs, A), Some(W1));

        world.windows.get_mut(W1).no_focus.static_value = Some(true);
        assert_eq!(
            locate_target(&world, W1, false).unwrap_err().to_string(),
            "Window WindowId(1) rejects focus by rule"
        );
        assert!(
            locate_target(&world, W1, true).is_ok(),
            "force-focus bypasses the rule"
        );
    }

    #[test]
    fn the_active_output_prefers_logical_focus_then_the_foreground_window() {
        let mut f = Fixture::new();
        f.add(B, 0, W1);
        let o = &f.outputs;
        assert_eq!(active_output(o, Some(B), || None, |_| None), Some(B));
        // A detached output is ignored.
        assert_eq!(
            active_output(o, Some(OutputId(9)), || Some(W1), |_| None),
            Some(B)
        );
        assert_eq!(active_output(o, None, || Some(W2), |_| Some(B)), Some(B));
        assert_eq!(
            active_output(o, None, || None, |_| None),
            Some(A),
            "primary"
        );
    }
}
