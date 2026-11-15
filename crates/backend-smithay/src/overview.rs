//! The overview of every workspace (`DispatchCommand::WorkspaceOverview`):
//! each display shows all of its workspaces at once, every one a small
//! picture of itself with its windows in it, live. A press on a window
//! goes to its workspace and gives it the keyboard; a press on a
//! workspace's empty room goes to that workspace; Escape, or the command
//! again, puts the overview away.
//!
//! Nothing is asked of any program: a window keeps its size and its
//! place, and only where its picture is drawn changes (`WlWindow::overview`,
//! as in the overview of one workspace). The windows of workspaces that
//! are not shown are hidden as ever, for keyboard and pointer; they are
//! drawn while this is up (`WlWindow::in_overview`).

use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::{Desktop, Rect, WindowHandle};
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;

/// How much room there is between the pictures of two workspaces, and
/// around them all.
const GAP: i32 = 28;
/// How high the line with a workspace's name is, above its picture.
pub const LABEL: i32 = 38;

/// One workspace in the overview.
pub struct Cell {
    pub output: MonitorHandle,
    /// Its place among its display's workspaces.
    pub index: usize,
    /// It is the one its display shows.
    pub shown: bool,
    /// Where its picture is, on the desktop.
    pub rect: Rect,
    /// Its windows, bottom to top, each with where its picture is.
    pub windows: Vec<(WindowHandle, Rect)>,
    /// Its name, drawn.
    pub label: MemoryRenderBuffer,
    pub label_size: (i32, i32),
    /// The names its ground and its frame are drawn under.
    pub ids: [smithay::backend::renderer::element::Id; 5],
}

/// The overview while it is up.
pub struct WorkspaceOverview {
    pub cells: Vec<Cell>,
    /// How many windows there were when it came up: with another number
    /// it is out of date, and put away.
    windows: usize,
}

/// Where the pictures of `count` workspaces go in `area`: a grid as near
/// to square as the count allows, every picture of the display's shape
/// (`screen`), with a line for its name above it.
pub fn cells(area: Rect, screen: Rect, count: usize) -> Vec<Rect> {
    if count == 0 || screen.width <= 0 || screen.height <= 0 {
        return Vec::new();
    }
    let columns = (1..=count)
        .find(|columns| columns * columns >= count)
        .unwrap_or(1);
    let rows = count.div_ceil(columns);
    let (columns_i, rows_i) = (columns as i32, rows as i32);
    let room_w = (area.width - GAP * (columns_i + 1)) / columns_i;
    let room_h = (area.height - GAP * (rows_i + 1)) / rows_i - LABEL;
    if room_w <= 0 || room_h <= 0 {
        return Vec::new();
    }
    // The display's shape, as large as the room allows.
    let scale = (f64::from(room_w) / f64::from(screen.width))
        .min(f64::from(room_h) / f64::from(screen.height));
    let width = (f64::from(screen.width) * scale).floor() as i32;
    let height = (f64::from(screen.height) * scale).floor() as i32;
    // The grid as a whole stands in the middle of the area.
    let grid_w = columns_i * width + (columns_i - 1) * GAP;
    let grid_h = rows_i * (height + LABEL) + (rows_i - 1) * GAP;
    let left = area.x + (area.width - grid_w) / 2;
    let top = area.y + (area.height - grid_h) / 2;
    (0..count)
        .map(|index| {
            let (column, row) = ((index % columns) as i32, (index / columns) as i32);
            Rect::new(
                left + column * (width + GAP),
                top + row * (height + LABEL + GAP) + LABEL,
                width,
                height,
            )
        })
        .collect()
}

/// Where the picture of something at `rect` on the display `screen` is,
/// in that display's small picture at `cell`.
pub fn small(rect: Rect, screen: Rect, cell: Rect) -> Rect {
    let scale = f64::from(cell.width) / f64::from(screen.width.max(1));
    let part = |value: i32| (f64::from(value) * scale).round() as i32;
    Rect::new(
        cell.x + part(rect.x - screen.x),
        cell.y + part(rect.y - screen.y),
        part(rect.width).max(1),
        part(rect.height).max(1),
    )
}

/// A workspace's name as a small picture: light letters on a dark ground,
/// in the accent's colour for the workspace that is shown.
fn label(name: &str, width: i32, colour: (u8, u8, u8)) -> (MemoryRenderBuffer, (i32, i32)) {
    let (width, height) = (width.clamp(40, 600), LABEL - 4);
    let fill = (22u8, 22u8, 28u8);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        *pixel = [fill.2, fill.1, fill.0, 255];
    }
    if let Some(font) = crate::stackbar::title_font() {
        crate::stackbar::write_title(
            &mut pixels,
            width,
            height,
            8,
            width - 8,
            fill,
            colour,
            name,
            font,
        );
    }
    let buffer = MemoryRenderBuffer::from_slice(
        &pixels,
        smithay::backend::allocator::Fourcc::Argb8888,
        (width, height),
        1,
        smithay::utils::Transform::Normal,
        None,
    );
    (buffer, (width, height))
}

impl State {
    /// The overview of every workspace comes up, or is put away.
    pub fn toggle_workspace_overview(&mut self) -> anyhow::Result<()> {
        if self.workspace_overview.is_some() {
            self.leave_workspace_overview(None);
            return Ok(());
        }
        // (The overview of one workspace has the desktop then.)
        anyhow::ensure!(
            self.desktop.decorations_visible,
            "Another overview is up already"
        );
        anyhow::ensure!(!self.locked, "The session is locked");
        let accent = self.core.wallpaper_accent.unwrap_or((64, 224, 208));
        let mut cells_all = Vec::new();
        let outputs: Vec<MonitorHandle> = self
            .core
            .monitor_manager
            .get_all_monitors_ordered()
            .into_iter()
            .map(|monitor| monitor.handle)
            .collect();
        for output in outputs {
            let Some(screen) = self.desktop.output_rectangle(output) else {
                continue;
            };
            let area = self
                .core
                .monitor_manager
                .get_adjusted_work_area(output, &self.desktop)
                .unwrap_or(screen);
            let views = self.core.executor.workspaces_laid_out(
                &mut self.desktop,
                &mut self.core.monitor_manager,
                output,
            );
            let places = cells(area, screen, views.len());
            for (view, rect) in views.into_iter().zip(places) {
                let colour = if view.shown { accent } else { (214, 214, 222) };
                let (label, label_size) = label(&view.name, rect.width, colour);
                // Bottom to top, as they lie on one another.
                let mut windows: Vec<(WindowHandle, Rect)> = view
                    .windows
                    .iter()
                    .map(|(id, place)| (*id, small(*place, screen, rect)))
                    .collect();
                let order = &self.desktop.stacking;
                windows.sort_by_key(|(id, _)| order.iter().position(|other| other == id));
                cells_all.push(Cell {
                    output,
                    index: view.index,
                    shown: view.shown,
                    rect,
                    windows,
                    label,
                    label_size,
                    ids: std::array::from_fn(|_| smithay::backend::renderer::element::Id::new()),
                });
            }
        }
        anyhow::ensure!(!cells_all.is_empty(), "There is no workspace to show");
        // The borders and tab strips rest, and the wallpaper goes soft
        // behind the pictures, as in the overview of one workspace.
        self.desktop.overview_all = true;
        self.desktop
            .apply(&gevrek_engine::Effect::DecorationsVisible(false))?;
        let now = std::time::Instant::now();
        for cell in &cells_all {
            for (id, picture) in &cell.windows {
                let from = self
                    .desktop
                    .shown(*id)
                    .then(|| self.desktop.windows.get(id).map(|window| window.rect))
                    .flatten();
                // A window that is shown goes to its picture's place; one
                // of a workspace that is not shown is there at once.
                let animated = from.is_some_and(|from| {
                    self.desktop.animations.schedule_rect(
                        *id,
                        from,
                        *picture,
                        gevrek_engine::animation_scheduler::TransitionScope::Layout,
                        now,
                    )
                });
                if let Some(window) = self.desktop.windows.get_mut(id) {
                    window.overview = Some(*picture);
                    window.in_overview = true;
                    window.drawn = if animated { from } else { None };
                }
            }
        }
        let windows = self.desktop.windows.values().filter(|w| w.alive).count();
        self.workspace_overview = Some(WorkspaceOverview {
            cells: cells_all,
            windows,
        });
        self.desktop.needs_redraw = true;
        Ok(())
    }

    /// The overview is put away; with a choice, the chosen workspace is
    /// shown (and the chosen window given the keyboard) as it goes.
    pub fn leave_workspace_overview(
        &mut self,
        choice: Option<(MonitorHandle, usize, Option<WindowHandle>)>,
    ) {
        let Some(overview) = self.workspace_overview.take() else {
            return;
        };
        // Where every picture was: the windows that are shown afterwards
        // grow from there to their places.
        let pictures: Vec<(WindowHandle, Rect)> = overview
            .cells
            .iter()
            .flat_map(|cell| cell.windows.iter().copied())
            .collect();
        for window in self.desktop.windows.values_mut() {
            window.in_overview = false;
        }
        self.desktop.overview_all = false;
        if let Err(error) = self
            .desktop
            .apply(&gevrek_engine::Effect::DecorationsVisible(true))
        {
            log::debug!("leaving the overview: {error}");
        }
        if let Some((output, index, window)) = choice {
            // (The workspace does not slide in: it grows from its picture.)
            self.desktop.plain_workspace_change = true;
            let shown = self.core.executor.show_workspace(
                &mut self.desktop,
                &mut self.core.monitor_manager,
                output,
                index,
            );
            self.desktop.plain_workspace_change = false;
            if let Err(error) = shown {
                log::warn!("the chosen workspace is not shown: {error}");
            }
            self.settle();
            if let Some(window) = window.filter(|id| self.desktop.shown(*id)) {
                self.focus_window(window);
            }
        }
        let now = std::time::Instant::now();
        for (id, picture) in pictures {
            if !self.desktop.shown(id) {
                continue;
            }
            let Some(rect) = self.desktop.windows.get(&id).map(|window| window.rect) else {
                continue;
            };
            let animated = self.desktop.animations.schedule_rect(
                id,
                picture,
                rect,
                gevrek_engine::animation_scheduler::TransitionScope::Layout,
                now,
            );
            if let Some(window) = self.desktop.windows.get_mut(&id) {
                window.drawn = animated.then_some(picture);
            }
        }
        self.desktop.needs_redraw = true;
        self.settle();
    }

    /// A press while the overview is up: on a window's picture, on a
    /// workspace's, or beside them all (which puts the overview away and
    /// changes nothing). Returns whether the overview was up.
    pub fn workspace_overview_press(&mut self, x: i32, y: i32) -> bool {
        let Some(overview) = &self.workspace_overview else {
            return false;
        };
        let inside = |rect: &Rect| {
            x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
        };
        let choice = overview
            .cells
            .iter()
            .find(|cell| {
                // (The name above a picture belongs to it.)
                inside(&Rect::new(
                    cell.rect.x,
                    cell.rect.y - LABEL,
                    cell.rect.width,
                    cell.rect.height + LABEL,
                ))
            })
            .map(|cell| {
                let window = cell
                    .windows
                    .iter()
                    .rev()
                    .find(|(_, picture)| inside(picture))
                    .map(|(id, _)| *id);
                (cell.output, cell.index, window)
            });
        self.leave_workspace_overview(choice);
        true
    }

    /// Whether the overview is out of date (a window came or went, a
    /// workspace was shown by other means): then it is put away.
    pub fn workspace_overview_is_stale(&self) -> bool {
        let Some(overview) = &self.workspace_overview else {
            return false;
        };
        let windows = self.desktop.windows.values().filter(|w| w.alive).count();
        windows != overview.windows
            || overview.cells.iter().any(|cell| {
                self.core
                    .monitor_manager
                    .get_workspaces(cell.output)
                    .is_none_or(|workspaces| (workspaces.focused_idx() == cell.index) != cell.shown)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_of_the_workspaces_stand_in_a_grid_of_the_displays_shape() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let area = Rect::new(0, 40, 1920, 1040);
        let one = cells(area, screen, 1);
        assert_eq!(one.len(), 1);
        let four = cells(area, screen, 4);
        assert_eq!(four.len(), 4);
        // Two by two, each of the display's shape, none over another,
        // all inside the area, with room for a name above each.
        for rect in &four {
            assert!(
                (rect.width * 1080 - rect.height * 1920).abs() <= 1920,
                "{rect:?}"
            );
            assert!(rect.x >= area.x && rect.x + rect.width <= area.x + area.width);
            assert!(rect.y - LABEL >= area.y && rect.y + rect.height <= area.y + area.height);
        }
        assert_eq!(four[0].y, four[1].y);
        assert_eq!(four[0].x, four[2].x);
        assert!(four[1].x >= four[0].x + four[0].width + GAP);
        assert!(four[2].y >= four[0].y + four[0].height + GAP + LABEL);
        // Five: three columns, two rows; nine: three by three.
        let five = cells(area, screen, 5);
        assert_eq!(five[2].y, five[0].y);
        assert!(five[3].y > five[0].y);
        assert_eq!(cells(area, screen, 9).len(), 9);
        assert!(cells(area, screen, 0).is_empty());
        assert!(cells(Rect::new(0, 0, 20, 20), screen, 4).is_empty());
    }

    #[test]
    fn a_windows_picture_is_where_the_window_is_in_its_workspaces_picture() {
        let screen = Rect::new(1920, 0, 1920, 1080);
        let cell = Rect::new(2000, 100, 480, 270);
        // The whole display: the whole picture.
        assert_eq!(small(screen, screen, cell), cell);
        // The right half.
        assert_eq!(
            small(Rect::new(2880, 0, 960, 1080), screen, cell),
            Rect::new(2240, 100, 240, 270)
        );
        // Never nothing.
        let dot = small(Rect::new(1920, 0, 1, 1), screen, cell);
        assert_eq!((dot.width, dot.height), (1, 1));
    }
}
