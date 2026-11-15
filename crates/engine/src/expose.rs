//! Expose: a window overview. The workspace's windows are arranged in a
//! grid, the focused one is promoted to the centre, and leaving restores
//! every window (docs/PHASE2_PLAN.md, step 4.8).
//!
//! The functions update `Workspace::expose_state` and return the moves to
//! make (`Moves`); the backend reads actual window rects and applies them.

use crate::workspace::{ExposeState, Workspace};
use gevrek_model::geometry::Rect;
use gevrek_model::ids::WindowId;
use std::collections::HashMap;

/// Gap between grid cells.
pub const GRID_PADDING: i32 = 10;
/// Gap between the grid and the work area edge.
pub const GRID_OUTER_PADDING: i32 = 20;

/// Windows to move and where.
pub type Moves = Vec<(WindowId, Rect)>;

pub fn is_active(workspace: &Workspace) -> bool {
    workspace.expose_state.is_some()
}

/// Enter expose: remember where every window is (`current` reads it) and
/// arrange them in a grid. Returns `None` when the workspace has no windows
/// (expose does not start).
pub fn activate(
    workspace: &mut Workspace,
    work_area: Rect,
    current: impl Fn(WindowId) -> Option<Rect>,
) -> Option<Moves> {
    let windows = workspace.all_windows();
    if windows.is_empty() {
        return None;
    }
    let original_rects: HashMap<WindowId, Rect> = windows
        .iter()
        .filter_map(|window| Some((*window, current(*window)?)))
        .collect();
    let grid_rects =
        ExposeState::calculate_grid(&windows, work_area, GRID_PADDING, GRID_OUTER_PADDING);
    let moves = windows
        .iter()
        .filter_map(|window| Some((*window, *grid_rects.get(window)?)))
        .collect();
    workspace.expose_state = Some(ExposeState {
        original_rects,
        grid_rects,
        promoted_window: None,
        promoted_rect: None,
        promoted_rects: HashMap::new(),
    });
    Some(moves)
}

/// Leave expose: every window goes back to where it was. Empty when expose
/// was not active.
pub fn deactivate(workspace: &mut Workspace) -> Moves {
    let Some(expose) = workspace.expose_state.take() else {
        return Vec::new();
    };
    let mut moves: Moves = expose.original_rects.into_iter().collect();
    moves.sort_by_key(|(window, _)| *window);
    moves
}

#[derive(Debug, Clone, PartialEq)]
pub enum AfterDestroy {
    /// Expose is not active on the workspace.
    NotActive,
    /// The last window is gone: expose ended.
    Ended,
    /// The remaining windows were arranged again (the promoted one stays).
    Regrid(Moves),
}

/// A window of the workspace was destroyed while expose is active. Without
/// a work area the grid is left as it is.
pub fn window_destroyed(
    workspace: &mut Workspace,
    window: WindowId,
    work_area: Option<Rect>,
) -> AfterDestroy {
    let Some(expose) = workspace.expose_state.as_mut() else {
        return AfterDestroy::NotActive;
    };
    expose.original_rects.remove(&window);
    expose.grid_rects.remove(&window);
    if expose.promoted_window == Some(window) {
        expose.promoted_window = None;
        expose.promoted_rect = None;
    }
    if expose.grid_rects.is_empty() {
        workspace.expose_state = None;
        return AfterDestroy::Ended;
    }
    let Some(work_area) = work_area else {
        return AfterDestroy::Regrid(Vec::new());
    };
    let mut remaining: Vec<WindowId> = expose.grid_rects.keys().copied().collect();
    remaining.sort();
    expose.grid_rects =
        ExposeState::calculate_grid(&remaining, work_area, GRID_PADDING, GRID_OUTER_PADDING);
    let moves = remaining
        .iter()
        .filter(|window| expose.promoted_window != Some(**window))
        .filter_map(|window| Some((*window, *expose.grid_rects.get(window)?)))
        .collect();
    AfterDestroy::Regrid(moves)
}

/// A window of the grid took focus: it is promoted to the centre and the
/// previously promoted one goes back to its cell. `actual_rect` reads a
/// window's current rect; the demoted window's size is remembered so it
/// comes back at that size. Returns `None` when nothing changes (expose not
/// active, the window is already promoted, or it is not in the grid). After
/// `Some`, the caller raises the window.
pub fn promote(
    workspace: &mut Workspace,
    window: WindowId,
    work_area: Option<Rect>,
    actual_rect: impl FnOnce(WindowId) -> Option<Rect>,
) -> Option<Moves> {
    let expose = workspace.expose_state.as_mut()?;
    if expose.promoted_window == Some(window) || !expose.grid_rects.contains_key(&window) {
        return None;
    }
    let mut moves = Vec::new();
    if let Some(previous) = expose.promoted_window {
        if let Some(actual) = actual_rect(previous) {
            expose.promoted_rects.insert(previous, actual);
        }
        if let Some(grid) = expose.grid_rects.get(&previous) {
            moves.push((previous, *grid));
        }
    }
    if let Some(area) = work_area {
        // The size it had when promoted before, else its cell size.
        let (width, height) = expose
            .promoted_rects
            .get(&window)
            .or_else(|| expose.grid_rects.get(&window))
            .map(|rect| (rect.width, rect.height))
            .unwrap_or((area.width / 3, area.height / 3));
        let promoted = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        moves.push((window, promoted));
        expose.promoted_window = Some(window);
        expose.promoted_rect = Some(promoted);
    }
    Some(moves)
}

/// A window moved or was resized during expose. The promoted window is kept
/// centred at its new size; any other window snaps back to its cell.
/// `actual` reads the window's current rect. Returns the correction, if one
/// is needed.
pub fn correct_move(
    workspace: &mut Workspace,
    window: WindowId,
    work_area: Option<Rect>,
    actual: impl FnOnce() -> Option<Rect>,
) -> Option<(WindowId, Rect)> {
    let expose = workspace.expose_state.as_mut()?;
    if expose.promoted_window == Some(window) {
        let area = work_area?;
        let actual = actual()?;
        let centered = Rect::new(
            area.x + (area.width - actual.width) / 2,
            area.y + (area.height - actual.height) / 2,
            actual.width,
            actual.height,
        );
        if actual.x == centered.x && actual.y == centered.y {
            return None;
        }
        expose.promoted_rect = Some(centered);
        expose.promoted_rects.insert(window, centered);
        Some((window, centered))
    } else {
        let grid = *expose.grid_rects.get(&window)?;
        (actual()? != grid).then_some((window, grid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    const W1: WindowId = WindowId(1);
    const W2: WindowId = WindowId(2);
    const W3: WindowId = WindowId(3);

    fn workspace() -> Workspace {
        let mut workspace = Workspace::new(0, "one".into());
        for window in [W1, W2, W3] {
            workspace.add_window(window, true);
        }
        workspace
    }

    fn start(workspace: &mut Workspace) -> Moves {
        activate(workspace, AREA, |window| {
            Some(Rect::new(window.0 as i32 * 100, 0, 500, 400))
        })
        .unwrap()
    }

    fn cell(workspace: &Workspace, window: WindowId) -> Rect {
        workspace.expose_state.as_ref().unwrap().grid_rects[&window]
    }

    #[test]
    fn entering_arranges_a_grid_and_leaving_restores_every_window() {
        let mut ws = workspace();
        let moves = start(&mut ws);
        assert_eq!(moves.len(), 3);
        assert!(is_active(&ws));
        // Three windows: a 2 x 2 grid inside the outer padding.
        assert_eq!(cell(&ws, W1), Rect::new(20, 20, 935, 495));
        assert_eq!(cell(&ws, W3).y, 20 + 495 + 10);

        let restored = deactivate(&mut ws);
        assert_eq!(restored[0], (W1, Rect::new(100, 0, 500, 400)));
        assert!(!is_active(&ws));
        assert!(deactivate(&mut ws).is_empty());
    }

    #[test]
    fn an_empty_workspace_does_not_enter_expose() {
        let mut ws = Workspace::new(0, "empty".into());
        assert_eq!(activate(&mut ws, AREA, |_| None), None);
        assert!(!is_active(&ws));
    }

    #[test]
    fn focus_promotes_to_the_centre_and_demotes_the_previous_window() {
        let mut ws = workspace();
        start(&mut ws);
        let first = promote(&mut ws, W1, Some(AREA), |_| None).unwrap();
        let centred = Rect::new((1920 - 935) / 2, (1040 - 495) / 2, 935, 495);
        assert_eq!(first, vec![(W1, centred)]);
        assert_eq!(
            promote(&mut ws, W1, Some(AREA), |_| None),
            None,
            "already promoted"
        );

        // W1 was resized by the user while promoted; that size is remembered.
        let resized = Rect::new(300, 200, 1200, 700);
        let second = promote(&mut ws, W2, Some(AREA), |_| Some(resized)).unwrap();
        assert_eq!(second[0], (W1, cell(&ws, W1)), "back to its cell");
        assert_eq!(second[1].0, W2);

        let again = promote(&mut ws, W1, Some(AREA), |_| None).unwrap();
        let w1 = again.iter().find(|(window, _)| *window == W1).unwrap().1;
        assert_eq!(
            (w1.width, w1.height),
            (1200, 700),
            "its promoted size comes back"
        );
        assert_eq!(promote(&mut ws, WindowId(9), Some(AREA), |_| None), None);
    }

    #[test]
    fn a_moved_cell_snaps_back_and_the_promoted_window_recentres() {
        let mut ws = workspace();
        start(&mut ws);
        promote(&mut ws, W1, Some(AREA), |_| None);

        let dragged = Rect::new(5, 5, 935, 495);
        assert_eq!(
            correct_move(&mut ws, W2, Some(AREA), || Some(dragged)),
            Some((W2, cell(&ws, W2)))
        );
        let in_place = cell(&ws, W2);
        assert_eq!(
            correct_move(&mut ws, W2, Some(AREA), || Some(in_place)),
            None
        );

        let resized = Rect::new(0, 0, 1000, 600);
        assert_eq!(
            correct_move(&mut ws, W1, Some(AREA), || Some(resized)),
            Some((W1, Rect::new(460, 220, 1000, 600)))
        );
    }

    #[test]
    fn destroying_windows_regrids_then_ends_expose() {
        let mut ws = workspace();
        start(&mut ws);
        promote(&mut ws, W1, Some(AREA), |_| None);

        let AfterDestroy::Regrid(moves) = window_destroyed(&mut ws, W3, Some(AREA)) else {
            panic!("expected a new grid");
        };
        assert_eq!(moves.len(), 1, "the promoted window stays in the centre");
        assert_eq!(moves[0].0, W2);

        assert_eq!(
            window_destroyed(&mut ws, W1, Some(AREA)).clone(),
            AfterDestroy::Regrid(vec![(W2, cell(&ws, W2))])
        );
        assert_eq!(
            window_destroyed(&mut ws, W2, Some(AREA)),
            AfterDestroy::Ended
        );
        assert_eq!(
            window_destroyed(&mut ws, W2, Some(AREA)),
            AfterDestroy::NotActive
        );
    }
}
