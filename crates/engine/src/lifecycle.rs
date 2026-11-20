//! Window lifecycle: adoption, destruction, minimize and restore, and windows
//! the app hides itself (docs/PHASE2_PLAN.md, step 4.2).
//!
//! The functions work on borrowed state (`World`): the outputs with their
//! workspaces, and the per-window state. While the executor is being ported
//! the runtime owns both, so the Win32 executor and the engine call the same
//! code. Facts only the backend knows (which output a window is on, whether a
//! rule floats it) come in as arguments; the backend also filters out
//! windows the WM never manages before calling.
//!
//! Results: `Effect`s for the backend, and the outputs whose visible
//! workspace needs a layout pass (`Changes::relayout`).

use crate::contract::Effect;
use crate::outputs::Outputs;
use crate::rule_state::WindowStateMap;
use gevrek_model::ids::{OutputId, WindowId};
use std::collections::BTreeSet;

/// The state the lifecycle functions read and change.
pub struct World<'a> {
    pub outputs: &'a mut Outputs,
    pub windows: &'a mut WindowStateMap,
}

/// What a lifecycle step produced.
#[derive(Debug, Default, PartialEq)]
pub struct Changes {
    pub effects: Vec<Effect>,
    /// Outputs whose visible workspace must be laid out again. Laid out once
    /// per wake, so a burst of events on one output costs one pass.
    pub relayout: BTreeSet<OutputId>,
}

/// Where a new window goes, as far as the backend and the rules know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The output the window is on, or else the active output.
    pub output: Option<OutputId>,
    /// A rule floats the window.
    pub floating: bool,
    /// The workspace of `output` the window joins, where that is not the
    /// one the output shows: the workspace of the window it belongs to (a
    /// dialog of a window that is put away). It is hidden with it. `None`:
    /// the workspace the output shows.
    pub workspace: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adoption {
    /// Added to a workspace of `output`: the one it shows (`shown`), or one
    /// that is put away, and the window with it. `first` is true when the
    /// window was not known before, so its initial rules must run.
    Adopted {
        output: OutputId,
        first: bool,
        shown: bool,
    },
    /// Already a workspace member.
    AlreadyManaged,
    /// Released with `unmanage` for this session.
    SessionUnmanaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdoptError {
    NoOutput,
    NoActiveWorkspace,
}

impl std::fmt::Display for AdoptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoOutput => "Could not determine a monitor for the new window",
            Self::NoActiveWorkspace => "The monitor has no active workspace",
        })
    }
}

impl std::error::Error for AdoptError {}

/// Manage a window by default: a manageable window becomes managed even when
/// the automation graph does not route it. A floating rule keeps it
/// floating; windows released with `unmanage` stay unmanaged.
pub fn adopt(
    world: &mut World,
    id: WindowId,
    placement: Placement,
    changes: &mut Changes,
) -> Result<Adoption, AdoptError> {
    if world.outputs.find_window(id).is_some() {
        return Ok(Adoption::AlreadyManaged);
    }
    if world.windows.get(id).session_unmanaged {
        return Ok(Adoption::SessionUnmanaged);
    }
    let output = placement.output.ok_or(AdoptError::NoOutput)?;
    let workspaces = world
        .outputs
        .get_workspaces_mut(output)
        .ok_or(AdoptError::NoActiveWorkspace)?;
    let index = placement
        .workspace
        .filter(|index| *index < workspaces.len())
        .unwrap_or_else(|| workspaces.focused_idx());
    let shown = index == workspaces.focused_idx();
    let workspace = workspaces
        .get_mut(index)
        .ok_or(AdoptError::NoActiveWorkspace)?;
    if placement.floating {
        workspace.add_floating_window(id);
    } else {
        workspace.add_window(id, true);
    }
    let state = world.windows.get_mut(id);
    let first = !state.known;
    state.known = true;
    state.hidden_by_wm = !shown;
    // (A workspace that is put away is laid out when it is shown.)
    if shown {
        changes.relayout.insert(output);
    }
    changes
        .effects
        .push(Effect::SetVisible { id, visible: shown });
    Ok(Adoption::Adopted {
        output,
        first,
        shown,
    })
}

/// Where a destroyed window was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// Not a workspace member.
    NotManaged,
    /// Removed from the visible workspace of the output (queued for layout).
    Visible(OutputId),
    /// Removed from a hidden workspace, which the caller lays out now (the
    /// once-per-wake layout only covers visible workspaces).
    Hidden { output: OutputId, workspace: usize },
}

/// A window was destroyed: drop its state and its workspace membership.
pub fn destroyed(world: &mut World, id: WindowId, changes: &mut Changes) -> Removal {
    world.windows.forget(id);
    let Some((output, workspace)) = world.outputs.find_and_remove_window(id) else {
        return Removal::NotManaged;
    };
    let visible = world
        .outputs
        .get_workspaces(output)
        .is_some_and(|workspaces| workspaces.focused_idx() == workspace);
    if visible {
        changes.relayout.insert(output);
        Removal::Visible(output)
    } else {
        Removal::Hidden { output, workspace }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Minimize {
    /// The WM hid the window itself (workspace switch); nothing changes.
    HiddenByWm,
    /// A floating member or an unmanaged window: it keeps its place.
    Kept,
    /// A tiled window left the layout of `output` until it is restored.
    LeftLayout(OutputId),
}

/// A window was minimized. A tiled window leaves the layout until it is
/// restored; its workspace is remembered.
pub fn minimized(world: &mut World, id: WindowId, changes: &mut Changes) -> Minimize {
    if world.windows.get(id).hidden_by_wm {
        return Minimize::HiddenByWm;
    }
    world.windows.get_mut(id).minimized = true;
    let Some((output, index)) = world.outputs.find_window(id) else {
        return Minimize::Kept;
    };
    if world.outputs.is_window_floating(id) {
        return Minimize::Kept;
    }
    let workspace = world
        .outputs
        .get_workspaces(output)
        .and_then(|workspaces| workspaces.get(index));
    let workspace_id = workspace.map(|workspace| workspace.id);
    // (Where it stood among the tiled ones: the panel lists it there.)
    let place = workspace.and_then(|workspace| {
        workspace
            .containers
            .iter()
            .flat_map(|container| container.windows.iter().copied())
            .position(|window| window == id)
    });
    world.outputs.find_and_remove_window(id);
    if let Some(workspace_id) = workspace_id {
        let state = world.windows.get_mut(id);
        state.minimized_origin = Some(workspace_id);
        state.minimized_place = place;
    }
    changes.relayout.insert(output);
    Minimize::LeftLayout(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restore {
    /// Still a member (floating windows stay members while minimized).
    StillMember,
    /// Back in the visible workspace of `output`.
    Rejoined(OutputId),
    /// Never managed (e.g. minimized when the WM started): the caller adopts
    /// it with `adopt`, then calls `restored_settle`.
    NeedsAdoption,
    /// Released with `unmanage`; stays out.
    Unmanaged,
}

/// A window came back from minimized. A tiled window that left the layout
/// returns to its workspace if that one is visible, otherwise it joins the
/// visible workspace of `fallback` (the output it reappears on, or the
/// active one), never a hidden workspace, which would hide it again.
pub fn restored(world: &mut World, id: WindowId, fallback: Option<OutputId>) -> Restore {
    let state = world.windows.get_mut(id);
    state.minimized = false;
    state.hidden_by_wm = false;
    let origin = state.minimized_origin.take();
    state.minimized_place = None;
    if world.outputs.find_window(id).is_some() {
        return Restore::StillMember;
    }
    let Some(origin) = origin else {
        return if world.windows.get(id).session_unmanaged {
            Restore::Unmanaged
        } else {
            Restore::NeedsAdoption
        };
    };
    let visible_origin = world
        .outputs
        .find_workspace_by_id(origin)
        .filter(|(output, index)| {
            world
                .outputs
                .get_workspaces(*output)
                .is_some_and(|workspaces| workspaces.focused_idx() == *index)
        })
        .map(|(output, _)| output);
    let target = visible_origin.or(fallback);
    match target.and_then(|output| world.outputs.get_focused_workspace_mut(output)) {
        Some(workspace) => {
            workspace.add_window(id, true);
            log::debug!(
                "Restored window {:?} rejoined workspace '{}'",
                id,
                workspace.name
            );
            Restore::Rejoined(target.expect("a workspace was found for it"))
        }
        None => Restore::StillMember,
    }
}

/// After a restore (and any adoption): if the window is a member, queue its
/// output for layout. Returns the output and whether the window is tiled
/// (a tiled window must not stay maximized).
pub fn restored_settle(
    world: &mut World,
    id: WindowId,
    changes: &mut Changes,
) -> Option<(OutputId, bool)> {
    let (output, _) = world.outputs.find_window(id)?;
    changes.relayout.insert(output);
    Some((output, !world.outputs.is_window_floating(id)))
}

/// The app hid its own window (e.g. minimize to tray). It stops being
/// managed and leaves the layout; when the app shows it again it is adopted
/// like a new window. Returns the output whose layout it left.
pub fn unmapped(world: &mut World, id: WindowId, changes: &mut Changes) -> Option<OutputId> {
    let left = world
        .outputs
        .find_and_remove_window(id)
        .map(|(output, _)| output);
    if let Some(output) = left {
        changes.relayout.insert(output);
    }
    let state = world.windows.get_mut(id);
    state.known = false;
    state.minimized = false;
    state.minimized_origin = None;
    state.minimized_place = None;
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::LayoutType;

    struct Fixture {
        outputs: Outputs,
        windows: WindowStateMap,
    }

    const A: OutputId = OutputId(1);
    const W: WindowId = WindowId(10);

    impl Fixture {
        fn new() -> Self {
            let mut outputs = Outputs::new();
            outputs.add_test_monitor(A, "A");
            outputs.create_workspace(A, "one".into(), LayoutType::BSP);
            outputs.create_workspace(A, "two".into(), LayoutType::BSP);
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

        fn tiled_on(&self, workspace: usize) -> Vec<WindowId> {
            let ws = self
                .outputs
                .get_workspaces(A)
                .unwrap()
                .get(workspace)
                .unwrap();
            ws.containers.iter().flat_map(|c| c.all_windows()).collect()
        }
    }

    fn place() -> Placement {
        Placement {
            output: Some(A),
            floating: false,
            workspace: None,
        }
    }

    #[test]
    fn adoption_tiles_shows_and_reports_the_first_time_only() {
        let mut f = Fixture::new();
        let mut changes = Changes::default();
        let first = adopt(&mut f.world(), W, place(), &mut changes).unwrap();
        assert_eq!(
            first,
            Adoption::Adopted {
                output: A,
                first: true,
                shown: true
            }
        );
        assert_eq!(f.tiled_on(0), vec![W]);
        assert_eq!(
            changes.effects,
            vec![Effect::SetVisible {
                id: W,
                visible: true
            }]
        );
        assert!(changes.relayout.contains(&A));

        let again = adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();
        assert_eq!(again, Adoption::AlreadyManaged);
    }

    #[test]
    fn floating_and_unmanaged_and_nowhere_to_go() {
        let mut f = Fixture::new();
        let floating = Placement {
            floating: true,
            ..place()
        };
        adopt(&mut f.world(), W, floating, &mut Changes::default()).unwrap();
        assert!(f.outputs.is_window_floating(W));

        let other = WindowId(11);
        f.windows.get_mut(other).session_unmanaged = true;
        let result = adopt(&mut f.world(), other, place(), &mut Changes::default());
        assert_eq!(result, Ok(Adoption::SessionUnmanaged));

        let lost = Placement {
            output: None,
            floating: false,
            workspace: None,
        };
        let result = adopt(&mut f.world(), WindowId(12), lost, &mut Changes::default());
        assert_eq!(result, Err(AdoptError::NoOutput));
    }

    /// A window that joins a workspace that is put away (its parent's) is
    /// that workspace's, floating, and out of sight with it; the workspace
    /// the output shows is not laid out for it.
    #[test]
    fn adoption_into_a_workspace_that_is_put_away_hides_the_window() {
        let mut f = Fixture::new();
        let away = Placement {
            floating: true,
            workspace: Some(1),
            ..place()
        };
        let mut changes = Changes::default();
        let adopted = adopt(&mut f.world(), W, away, &mut changes).unwrap();
        assert_eq!(
            adopted,
            Adoption::Adopted {
                output: A,
                first: true,
                shown: false
            }
        );
        assert_eq!(f.outputs.find_window(W), Some((A, 1)));
        assert!(f.outputs.is_window_floating(W));
        assert!(f.windows.get(W).hidden_by_wm);
        assert_eq!(
            changes.effects,
            vec![Effect::SetVisible {
                id: W,
                visible: false
            }]
        );
        assert!(changes.relayout.is_empty());

        // The one the output shows, named: as when none is named.
        let here = Placement {
            workspace: Some(0),
            ..place()
        };
        let other = WindowId(11);
        let adopted = adopt(&mut f.world(), other, here, &mut Changes::default()).unwrap();
        assert!(matches!(adopted, Adoption::Adopted { shown: true, .. }));
        assert!(!f.windows.get(other).hidden_by_wm);
    }

    #[test]
    fn a_minimized_tiled_window_returns_to_its_workspace() {
        let mut f = Fixture::new();
        adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();

        let mut changes = Changes::default();
        assert_eq!(
            minimized(&mut f.world(), W, &mut changes),
            Minimize::LeftLayout(A)
        );
        assert!(f.tiled_on(0).is_empty());
        assert!(f.windows.get(W).minimized);

        assert_eq!(restored(&mut f.world(), W, None), Restore::Rejoined(A));
        assert_eq!(f.tiled_on(0), vec![W]);
        assert_eq!(f.windows.get(W).minimized_origin, None);
        assert_eq!(
            restored_settle(&mut f.world(), W, &mut changes),
            Some((A, true))
        );
    }

    #[test]
    fn a_restore_never_lands_on_a_hidden_workspace() {
        let mut f = Fixture::new();
        adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();
        minimized(&mut f.world(), W, &mut Changes::default());
        f.outputs.focus_workspace_at(A, 1);

        assert_eq!(restored(&mut f.world(), W, Some(A)), Restore::Rejoined(A));
        assert!(f.tiled_on(0).is_empty());
        assert_eq!(
            f.tiled_on(1),
            vec![W],
            "joins the visible workspace instead"
        );
    }

    #[test]
    fn minimizes_the_wm_caused_change_nothing() {
        let mut f = Fixture::new();
        adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();
        f.windows.get_mut(W).hidden_by_wm = true;
        assert_eq!(
            minimized(&mut f.world(), W, &mut Changes::default()),
            Minimize::HiddenByWm
        );
        assert_eq!(f.tiled_on(0), vec![W]);
    }

    #[test]
    fn restoring_a_window_never_managed_asks_for_adoption() {
        let mut f = Fixture::new();
        assert_eq!(restored(&mut f.world(), W, Some(A)), Restore::NeedsAdoption);
        f.windows.get_mut(W).session_unmanaged = true;
        assert_eq!(restored(&mut f.world(), W, Some(A)), Restore::Unmanaged);
    }

    #[test]
    fn an_app_unmap_forgets_adoption_so_a_new_show_adopts_again() {
        let mut f = Fixture::new();
        adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();
        assert_eq!(
            unmapped(&mut f.world(), W, &mut Changes::default()),
            Some(A)
        );
        assert!(!f.windows.get(W).known);
        let again = adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();
        assert_eq!(
            again,
            Adoption::Adopted {
                output: A,
                first: true,
                shown: true
            }
        );
    }

    #[test]
    fn destroying_reports_whether_the_workspace_was_visible() {
        let mut f = Fixture::new();
        adopt(&mut f.world(), W, place(), &mut Changes::default()).unwrap();
        let mut changes = Changes::default();
        assert_eq!(
            destroyed(&mut f.world(), W, &mut changes),
            Removal::Visible(A)
        );
        assert!(changes.relayout.contains(&A));

        let hidden = WindowId(11);
        adopt(&mut f.world(), hidden, place(), &mut Changes::default()).unwrap();
        f.outputs.focus_workspace_at(A, 1);
        assert_eq!(
            destroyed(&mut f.world(), hidden, &mut Changes::default()),
            Removal::Hidden {
                output: A,
                workspace: 0
            }
        );
        assert!(!f.windows.get(hidden).known);
    }
}
