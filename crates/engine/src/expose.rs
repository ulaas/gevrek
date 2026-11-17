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

/// Between two pictures of an overview that shows the windows as
/// pictures of themselves ([`arrange`]).
pub const PICTURE_GAP: i32 = 36;
/// Under each picture: the room of its window's title.
pub const PICTURE_TITLE: i32 = 30;
/// The largest a picture gets, of the room the pictures have, each way.
/// (One window alone is not shown as large as it is: an overview is seen
/// to be one.)
pub const PICTURE_LARGEST: f64 = 0.72;

/// Where the pictures of windows go in an overview of them, in `area`:
/// every window as it looks (its own shape), all of them made smaller by
/// the same measure, so that what is large stays larger than what is
/// small; in rows, in the order they stand on the screen, the whole in
/// the middle of the area. The measure is the largest at which all of
/// them fit, and at which none is larger than [`PICTURE_LARGEST`] of the
/// area: the area says how large a window is shown, not the window.
///
/// `windows` are the windows with where they are; one without a size is
/// left out.
pub fn arrange(windows: &[(WindowId, Rect)], area: Rect) -> Moves {
    let mut ordered: Vec<(WindowId, Rect)> = windows
        .iter()
        .copied()
        .filter(|(_, rect)| rect.width > 0 && rect.height > 0)
        .collect();
    if ordered.is_empty() {
        return Vec::new();
    }
    // From the top, and from the left where two are as high.
    ordered.sort_by_key(|(window, rect)| {
        (
            rect.y + rect.height / 2,
            rect.x + rect.width / 2,
            window.0,
        )
    });
    let margin = ((area.width / 14).max(24), (area.height / 14).max(24));
    let room = Rect::new(
        area.x + margin.0,
        area.y + margin.1,
        (area.width - 2 * margin.0).max(1),
        (area.height - 2 * margin.1).max(1),
    );
    let total: f64 = ordered.iter().map(|(_, rect)| f64::from(rect.width)).sum();
    // The rows a number of rows makes: each as wide as the others, as
    // far as whole windows allow.
    let rows_of = |count: usize| -> Vec<Vec<usize>> {
        let each = total / count as f64;
        let mut rows = vec![Vec::new(); count];
        let mut before = 0.0;
        for (index, (_, rect)) in ordered.iter().enumerate() {
            let middle = before + f64::from(rect.width) / 2.0;
            rows[((middle / each) as usize).min(count - 1)].push(index);
            before += f64::from(rect.width);
        }
        rows.retain(|row| !row.is_empty());
        rows
    };
    // The measure at which these rows fit.
    let measure_of = |rows: &[Vec<usize>]| -> f64 {
        let across = rows
            .iter()
            .map(|row| {
                let width: f64 = row.iter().map(|at| f64::from(ordered[*at].1.width)).sum();
                f64::from(room.width - PICTURE_GAP * (row.len() as i32 - 1)) / width
            })
            .fold(f64::INFINITY, f64::min);
        let heights: f64 = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|at| f64::from(ordered[*at].1.height))
                    .fold(0.0, f64::max)
            })
            .sum();
        let count = rows.len() as i32;
        let down =
            f64::from(room.height - PICTURE_TITLE * count - PICTURE_GAP * (count - 1)) / heights;
        across.min(down)
    };
    let largest = ordered
        .iter()
        .map(|(_, rect)| {
            (PICTURE_LARGEST * f64::from(room.width) / f64::from(rect.width))
                .min(PICTURE_LARGEST * f64::from(room.height) / f64::from(rect.height))
        })
        .fold(1.0, f64::min);
    let mut best: Option<(f64, Vec<Vec<usize>>)> = None;
    for count in 1..=ordered.len() {
        let rows = rows_of(count);
        let measure = measure_of(&rows).min(largest);
        if best.as_ref().is_none_or(|(so_far, _)| measure > *so_far + 1e-9) {
            best = Some((measure, rows));
        }
    }
    let Some((measure, mut rows)) = best else {
        return Vec::new();
    };
    // (Very many windows on a small display: seen, however small.)
    let measure = measure.max(0.02);
    let small = |length: i32| ((f64::from(length) * measure).round() as i32).max(1);
    let row_height = |row: &[usize]| {
        row.iter()
            .map(|at| small(ordered[*at].1.height))
            .max()
            .unwrap_or(1)
    };
    let whole: i32 = rows
        .iter()
        .map(|row| row_height(row) + PICTURE_TITLE)
        .sum::<i32>()
        + PICTURE_GAP * (rows.len() as i32 - 1);
    let mut top = room.y + (room.height - whole) / 2;
    let mut places = Vec::with_capacity(ordered.len());
    for row in &mut rows {
        // In a row, from the left.
        row.sort_by_key(|at| {
            let rect = ordered[*at].1;
            (rect.x + rect.width / 2, rect.y, ordered[*at].0 .0)
        });
        let height = row_height(row);
        let width: i32 = row.iter().map(|at| small(ordered[*at].1.width)).sum::<i32>()
            + PICTURE_GAP * (row.len() as i32 - 1);
        let mut left = room.x + (room.width - width) / 2;
        for at in row.iter() {
            let (window, rect) = ordered[*at];
            let (wide, high) = (small(rect.width), small(rect.height));
            places.push((
                window,
                Rect::new(left, top + (height - high) / 2, wide, high),
            ));
            left += wide + PICTURE_GAP;
        }
        top += height + PICTURE_TITLE + PICTURE_GAP;
    }
    places
}

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
    let grid_rects = places(&windows, &original_rects, work_area);
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

/// Where the windows of an overview go: each at its own shape, all made
/// smaller by the same measure ([`arrange`]), from where they were when
/// the overview came up. Where one of them is not known to be anywhere
/// (no size to show it at), all of them are given the cells of a grid, as
/// before there was a measure.
fn places(
    windows: &[WindowId],
    where_they_were: &HashMap<WindowId, Rect>,
    work_area: Rect,
) -> HashMap<WindowId, Rect> {
    let known: Vec<(WindowId, Rect)> = windows
        .iter()
        .filter_map(|window| Some((*window, *where_they_were.get(window)?)))
        .collect();
    let arranged: HashMap<WindowId, Rect> = arrange(&known, work_area).into_iter().collect();
    if arranged.len() == windows.len() {
        arranged
    } else {
        ExposeState::calculate_grid(windows, work_area, GRID_PADDING, GRID_OUTER_PADDING)
    }
}

/// The picture among `places` to go to from the one at `from` with an
/// arrow key (`across` and `down`: -1, 0 or 1): the nearest one that lies
/// that way. `None` where none does.
pub fn towards(places: &[Rect], from: usize, across: i32, down: i32) -> Option<usize> {
    let middle = |rect: &Rect| (rect.x + rect.width / 2, rect.y + rect.height / 2);
    let (x, y) = middle(places.get(from)?);
    places
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != from)
        .filter_map(|(at, rect)| {
            let (there_x, there_y) = middle(rect);
            let (by_x, by_y) = (there_x - x, there_y - y);
            // How far it is that way, and how far beside the way.
            let (along, beside) = if across != 0 {
                (by_x * across, by_y.abs())
            } else {
                (by_y * down, by_x.abs())
            };
            // (That way at all, and more that way than beside it; a row
            // above or below counts whatever is beside.)
            let lies = along > 0 && (across == 0 || beside <= along);
            lies.then_some((along + 2 * beside, at))
        })
        .min()
        .map(|(_, at)| at)
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
    expose.grid_rects = places(&remaining, &expose.original_rects, work_area);
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
        // Three windows of 500 by 400, side by side at the top: shown at
        // the size they have (they fit), in a row, in the middle.
        for window in [W1, W2, W3] {
            let place = cell(&ws, window);
            assert_eq!((place.width, place.height), (500, 400));
            assert!(place.x >= AREA.x && place.x + place.width <= AREA.width);
        }
        assert!(cell(&ws, W1).x < cell(&ws, W2).x && cell(&ws, W2).x < cell(&ws, W3).x);
        assert_eq!(cell(&ws, W1).y, cell(&ws, W3).y);
        // A window that is not known to be anywhere: the grid, as before.
        let mut unknown = workspace();
        activate(&mut unknown, AREA, |window| {
            (window != W3).then(|| Rect::new(0, 0, 500, 400))
        })
        .unwrap();
        assert_eq!(cell(&unknown, W1), Rect::new(20, 20, 935, 495));
        assert_eq!(cell(&unknown, W3).y, 20 + 495 + 10);

        let restored = deactivate(&mut ws);
        assert_eq!(restored[0], (W1, Rect::new(100, 0, 500, 400)));
        assert!(!is_active(&ws));
        assert!(deactivate(&mut ws).is_empty());
    }

    fn within(inner: Rect, outer: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.width <= outer.x + outer.width
            && inner.y + inner.height <= outer.y + outer.height
    }

    #[test]
    fn pictures_keep_their_shape_and_all_fit_the_area() {
        // Two halves of the screen and a small window over them.
        let windows = [
            (W1, Rect::new(0, 0, 960, 1040)),
            (W2, Rect::new(960, 0, 960, 1040)),
            (W3, Rect::new(700, 300, 400, 300)),
        ];
        let places = arrange(&windows, AREA);
        assert_eq!(places.len(), 3);
        for (window, place) in &places {
            let was = windows.iter().find(|(id, _)| id == window).unwrap().1;
            assert!(within(*place, AREA), "{place:?} is not in the area");
            // Smaller, and of the shape it has.
            assert!(place.width < was.width && place.height < was.height);
            let shape = f64::from(was.width) / f64::from(was.height);
            let shown = f64::from(place.width) / f64::from(place.height);
            assert!((shape - shown).abs() < 0.02, "{was:?} is shown as {place:?}");
        }
        // By the same measure: the small one stays the smallest.
        let width = |window| places.iter().find(|(id, _)| *id == window).unwrap().1.width;
        assert!(width(W3) < width(W1));
        assert_eq!(width(W1), width(W2));
        // None over another.
        for (index, (_, one)) in places.iter().enumerate() {
            for (_, other) in &places[index + 1..] {
                let apart = one.x + one.width <= other.x
                    || other.x + other.width <= one.x
                    || one.y + one.height <= other.y
                    || other.y + other.height <= one.y;
                assert!(apart, "{one:?} and {other:?} are over each other");
            }
        }
    }

    #[test]
    fn the_area_says_how_large_a_picture_is() {
        // One window that fills the screen: shown well smaller than it is.
        let alone = arrange(&[(W1, AREA)], AREA);
        let place = alone[0].1;
        assert!(f64::from(place.width) <= f64::from(AREA.width) * PICTURE_LARGEST);
        assert!(f64::from(place.height) <= f64::from(AREA.height) * PICTURE_LARGEST);
        // In the middle of the area.
        let middle = place.x + place.width / 2;
        assert!((middle - AREA.width / 2).abs() <= 1);
        // A small window is not made larger than it is.
        let small = arrange(&[(W1, Rect::new(10, 10, 300, 200))], AREA);
        assert_eq!((small[0].1.width, small[0].1.height), (300, 200));
        // In half the area the same windows are shown half as large.
        let half = Rect::new(0, 0, 960, 520);
        let there = arrange(&[(W1, AREA)], half);
        assert!((there[0].1.width * 2 - place.width).abs() <= 4);
    }

    #[test]
    fn many_windows_are_shown_in_rows_from_where_they_stand() {
        let windows: Vec<(WindowId, Rect)> = (0..9)
            .map(|at| {
                (
                    WindowId(at + 1),
                    Rect::new((at as i32 % 3) * 640, (at as i32 / 3) * 340, 640, 340),
                )
            })
            .collect();
        let places = arrange(&windows, AREA);
        assert_eq!(places.len(), 9);
        assert!(places.iter().all(|(_, place)| within(*place, AREA)));
        let place = |window| places.iter().find(|(id, _)| *id == WindowId(window)).unwrap().1;
        // The one at the top left is shown at the top left, the one at
        // the bottom right at the bottom right.
        assert!(place(1).x < place(3).x && place(1).y < place(9).y);
        assert!(place(9).x > place(7).x);
        assert!(arrange(&[], AREA).is_empty());
        assert!(arrange(&[(W1, Rect::new(0, 0, 0, 10))], AREA).is_empty());
    }

    #[test]
    fn an_arrow_goes_to_the_nearest_picture_that_way() {
        // Two rows of two.
        let places = [
            Rect::new(0, 0, 100, 100),
            Rect::new(200, 0, 100, 100),
            Rect::new(0, 200, 100, 100),
            Rect::new(200, 200, 100, 100),
        ];
        assert_eq!(towards(&places, 0, 1, 0), Some(1));
        assert_eq!(towards(&places, 1, -1, 0), Some(0));
        assert_eq!(towards(&places, 0, 0, 1), Some(2));
        assert_eq!(towards(&places, 3, 0, -1), Some(1));
        // Nothing lies further that way.
        assert_eq!(towards(&places, 1, 1, 0), None);
        assert_eq!(towards(&places, 0, 0, -1), None);
        assert_eq!(towards(&places, 9, 1, 0), None);
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
        let centred = Rect::new((1920 - 500) / 2, (1040 - 400) / 2, 500, 400);
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

        let dragged = Rect::new(5, 5, 500, 400);
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
