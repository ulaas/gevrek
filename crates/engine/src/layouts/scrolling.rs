//! The scrolling layout: the windows stand side by side as columns on a
//! strip that is as long as it has to be, and the display shows a part of
//! it.
//!
//! - Every column keeps its own width (a share of the display's width, a
//!   half unless it was given another). A window that opens gets a column
//!   of its own beside the one with the keyboard, and no other window
//!   changes its size for it: the strip grows.
//! - The strip really scrolls: the view is a place on the strip counted in
//!   pixels (`view_offset`), not in columns. It moves no more than it has
//!   to: it stays where it is while the column with the keyboard is wholly
//!   in it, and moves just far enough to bring that column in when it is
//!   not. A column may stand partly in view, cut off at the display's
//!   edge, as on a strip of paper pulled past a window; and the view can
//!   be moved by itself, by any amount (`scrolled`).
//! - A column that is wholly out of view is marked (`Container::
//!   offscreen`); the window manager hides its windows. It keeps its true
//!   place on the strip, so that it comes in from where it is.
//! - Where the desktop's windows are the system's own and cannot be cut
//!   off at a display's edge (Windows: the half outside would lie on the
//!   display beside it), only whole columns are shown and the view moves
//!   by whole columns (`first_column`, `whole_columns_only`).

use super::Layout;
use crate::container::{Container, Padding, Rect};

/// The share of the display's width a column has until it is given another.
pub const DEFAULT_WIDTH: f32 = 0.5;

/// The least and the most of the display's width a column can have.
pub const WIDTH_RANGE: (f32, f32) = (0.1, 1.0);

/// The width of a column with this share of a room so wide, in pixels.
pub fn column_width(share: f32, room: i32) -> i32 {
    let share = if share.is_finite() {
        share.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1)
    } else {
        DEFAULT_WIDTH
    };
    ((room as f32 * share).round() as i32).clamp(1, room.max(1))
}

/// The first column of the view, after the keyboard went to the column
/// `focused`: where it was (`first`) while the focused column is still
/// wholly in view; the focused column itself when that is before the view;
/// else as few columns further on as bring it wholly into view. `widths`
/// are the columns' widths and `room` the view's, in pixels.
pub fn first_column(first: usize, focused: usize, widths: &[i32], room: i32) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let focused = focused.min(widths.len() - 1);
    let mut first = first.min(widths.len() - 1);
    if focused < first {
        return focused;
    }
    // (A column wider than the view is in view when it is the first.)
    while first < focused && widths[first..=focused].iter().sum::<i32>() > room {
        first += 1;
    }
    // With columns gone, the view does not begin further on than it must:
    // as many of the columns before it as still fit come back into it.
    while first > 0 && widths[first - 1..].iter().sum::<i32>() <= room {
        first -= 1;
    }
    first
}

/// How many columns from `first` on are wholly in a view so wide (the
/// first always is, however wide).
pub fn columns_in_view(first: usize, widths: &[i32], room: i32) -> usize {
    let mut used = 0;
    let mut count = 0;
    for width in widths.iter().skip(first) {
        if count > 0 && used + width > room {
            break;
        }
        used += width;
        count += 1;
    }
    count
}

/// Only whole columns can be shown here (the desktop cannot cut a window
/// off at a display's edge). Said once by the desktop's backend when it
/// starts; unless it is said, the strip scrolls freely.
static WHOLE_COLUMNS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_whole_columns_only(whole: bool) {
    WHOLE_COLUMNS.store(whole, std::sync::atomic::Ordering::Relaxed);
}

pub fn whole_columns_only() -> bool {
    WHOLE_COLUMNS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How long the strip is, and how far the view can be moved along it at
/// most (0 when everything fits).
pub fn furthest(widths: &[i32], room: i32) -> i32 {
    (widths.iter().sum::<i32>() - room).max(0)
}

/// Where the view is on the strip (how many pixels of the strip are to
/// its left), after the keyboard went to the column `focused`: where it
/// was while that column is wholly in view, else moved just far enough
/// to bring it in. A column wider than the view begins at the view's
/// left edge. The view never shows what is beyond the strip's ends.
pub fn view_offset(offset: i32, focused: usize, widths: &[i32], room: i32) -> i32 {
    if widths.is_empty() {
        return 0;
    }
    let focused = focused.min(widths.len() - 1);
    let left: i32 = widths[..focused].iter().sum();
    let right = left + widths[focused];
    let offset = if left < offset || widths[focused] >= room {
        left
    } else if right > offset + room {
        right - room
    } else {
        offset
    };
    offset.clamp(0, furthest(widths, room))
}

/// The view moved by itself by so many pixels (a wheel, fingers on a
/// touchpad), kept within the strip.
pub fn scrolled(offset: i32, by: i32, widths: &[i32], room: i32) -> i32 {
    offset.saturating_add(by).clamp(0, furthest(widths, room))
}

/// The column most of the view's middle lies on: the one that gets the
/// keyboard when the view was moved by itself and the column that had
/// it is out of view.
pub fn column_in_the_middle(offset: i32, widths: &[i32], room: i32) -> usize {
    let middle = offset + room / 2;
    let mut left = 0;
    for (index, width) in widths.iter().enumerate() {
        if middle < left + width {
            return index;
        }
        left += width;
    }
    widths.len().saturating_sub(1)
}

/// The scrolling layout with its view at a place on the strip.
pub struct ScrollingLayout {
    /// The first column in view (where only whole columns are shown).
    pub first: usize,
    /// How many pixels of the strip are left of the view; `None`: only
    /// whole columns are shown, from `first` on.
    pub offset: Option<i32>,
}

impl ScrollingLayout {
    /// Whole columns only, from this one on.
    pub fn new(first: usize) -> Self {
        Self {
            first,
            offset: None,
        }
    }

    /// The view at this place on the strip, columns cut off at its edges.
    pub fn at(offset: i32) -> Self {
        Self {
            first: 0,
            offset: Some(offset),
        }
    }
}

impl Layout for ScrollingLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        if containers.is_empty() {
            return;
        }
        let area = work_area.with_padding(padding);
        if let Some(offset) = self.offset {
            // Every column at its true place on the strip, seen from
            // where the view is. One that is wholly outside the work
            // area (not only outside the room between the gaps: a column
            // shows in the gap at the edge as it comes and goes) is out
            // of view.
            let mut left = area.x - offset;
            for container in containers.iter_mut() {
                let width = column_width(container.strip_width, area.width).min(area.width);
                container.set_rect(Rect::new(left, area.y, width, area.height));
                container.offscreen =
                    left + width <= work_area.x || left >= work_area.x + work_area.width;
                left += width;
            }
            return;
        }
        let widths: Vec<i32> = containers
            .iter()
            .map(|container| column_width(container.strip_width, area.width))
            .collect();
        let first = self.first.min(containers.len() - 1);
        let shown = columns_in_view(first, &widths, area.width);
        let mut x = area.x;
        for (index, container) in containers.iter_mut().enumerate() {
            let in_view = index >= first && index < first + shown;
            container.offscreen = !in_view;
            if in_view {
                // (A column wider than the view is as wide as the view.)
                let width = widths[index].min(area.width);
                container.set_rect(Rect::new(x, area.y, width, area.height));
                x += width;
            }
        }
        // A column out of view waits where it will come in: under the first
        // column of the view when it is before it, under the last when it
        // is after it. (It is hidden; it has a place so that it is there
        // at once when it is shown.)
        let first_rect = containers[first].rect;
        let last_rect = containers[first + shown - 1].rect;
        for (index, container) in containers.iter_mut().enumerate() {
            if container.offscreen {
                let beside = if index < first { first_rect } else { last_rect };
                container.set_rect(Rect::new(
                    beside.x,
                    beside.y,
                    widths[index].min(area.width),
                    beside.height,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(shares: &[f32]) -> Vec<Container> {
        shares
            .iter()
            .enumerate()
            .map(|(id, share)| {
                let mut container = Container::new(id);
                container.strip_width = *share;
                container
            })
            .collect()
    }

    #[test]
    fn a_new_window_changes_no_other_windows_size() {
        let area = Rect::new(0, 0, 2000, 1000);
        let mut two = columns(&[0.5, 0.5]);
        ScrollingLayout::new(0).calculate(area, &mut two, Padding::default());
        assert_eq!(two[0].rect, Rect::new(0, 0, 1000, 1000));
        assert_eq!(two[1].rect, Rect::new(1000, 0, 1000, 1000));
        // A third column: the two are as they were, and it is out of view.
        let mut three = columns(&[0.5, 0.5, 0.5]);
        ScrollingLayout::new(0).calculate(area, &mut three, Padding::default());
        assert_eq!(three[0].rect, two[0].rect);
        assert_eq!(three[1].rect, two[1].rect);
        assert!(!three[0].offscreen && !three[1].offscreen && three[2].offscreen);
        // One window has half the display, not all of it.
        let mut one = columns(&[0.5]);
        ScrollingLayout::new(0).calculate(area, &mut one, Padding::default());
        assert_eq!(one[0].rect, Rect::new(0, 0, 1000, 1000));
    }

    #[test]
    fn the_view_moves_by_whole_columns_and_no_more_than_it_must() {
        let widths = [1000, 1000, 1000, 1000];
        // The keyboard goes right: the view follows when it has to.
        assert_eq!(first_column(0, 0, &widths, 2000), 0);
        assert_eq!(first_column(0, 1, &widths, 2000), 0);
        assert_eq!(first_column(0, 2, &widths, 2000), 1);
        assert_eq!(first_column(1, 3, &widths, 2000), 2);
        // And left again: it stays while the column is in it.
        assert_eq!(first_column(2, 2, &widths, 2000), 2);
        assert_eq!(first_column(2, 1, &widths, 2000), 1);
        assert_eq!(first_column(1, 0, &widths, 2000), 0);
        // Columns of other widths: as few further on as bring it in.
        let mixed = [600, 1400, 800, 1000];
        assert_eq!(first_column(0, 2, &mixed, 2000), 2);
        assert_eq!(first_column(0, 3, &mixed, 2000), 2);
        // A column as wide as the view is in view alone.
        assert_eq!(first_column(0, 1, &[1000, 2000, 1000], 2000), 1);
        // Columns closed at the end: those before come back into view.
        assert_eq!(first_column(2, 0, &[1000], 2000), 0);
        assert_eq!(first_column(3, 2, &[1000, 1000, 1000], 2000), 1);
        assert_eq!(first_column(5, 0, &[], 2000), 0);
    }

    #[test]
    fn the_strip_scrolls_by_pixels_and_no_further_than_it_must() {
        let widths = [1000, 1000, 1000, 1000];
        // The keyboard goes right: the view follows when it has to, and
        // only as far as brings the column wholly in.
        assert_eq!(view_offset(0, 0, &widths, 2000), 0);
        assert_eq!(view_offset(0, 1, &widths, 2000), 0);
        assert_eq!(view_offset(0, 2, &widths, 2000), 1000);
        // Columns that do not fill the view evenly: a part of one stays in view.
        let narrow = [800, 800, 800, 800];
        assert_eq!(view_offset(0, 2, &narrow, 2000), 400);
        assert_eq!(
            view_offset(400, 1, &narrow, 2000),
            400,
            "still wholly in view: the view stays"
        );
        assert_eq!(view_offset(400, 0, &narrow, 2000), 0);
        assert_eq!(view_offset(0, 3, &narrow, 2000), 1200);
        // A column wider than the view begins at the view's left edge.
        assert_eq!(view_offset(0, 1, &[1000, 2600, 1000], 2000), 1000);
        // Columns closed: the view does not show what is beyond the strip's end.
        assert_eq!(view_offset(1200, 1, &[800, 800], 2000), 0);
        assert_eq!(view_offset(900, 0, &[], 2000), 0);
        // Moved by itself, within the strip.
        assert_eq!(scrolled(0, 350, &narrow, 2000), 350);
        assert_eq!(scrolled(350, -900, &narrow, 2000), 0);
        assert_eq!(scrolled(350, 9000, &narrow, 2000), 1200);
        assert_eq!(
            scrolled(0, 100, &[800, 800], 2000),
            0,
            "everything fits: nothing to scroll"
        );
        assert_eq!(column_in_the_middle(0, &narrow, 2000), 1);
        assert_eq!(column_in_the_middle(1200, &narrow, 2000), 2);
    }

    #[test]
    fn a_column_stands_partly_in_view_at_its_true_place() {
        let area = Rect::new(0, 0, 2000, 1000);
        let mut strip = columns(&[0.4, 0.4, 0.4, 0.4]);
        ScrollingLayout::at(400).calculate(area, &mut strip, Padding::default());
        // The first is half out at the left, the last wholly out at the right.
        assert_eq!(strip[0].rect, Rect::new(-400, 0, 800, 1000));
        assert_eq!(strip[1].rect, Rect::new(400, 0, 800, 1000));
        assert_eq!(strip[2].rect, Rect::new(1200, 0, 800, 1000));
        assert_eq!(strip[3].rect, Rect::new(2000, 0, 800, 1000));
        assert!(!strip[0].offscreen && !strip[1].offscreen && !strip[2].offscreen);
        assert!(strip[3].offscreen, "wholly outside: out of view");
        // No column changes its size as the view moves.
        let mut moved = columns(&[0.4, 0.4, 0.4, 0.4]);
        ScrollingLayout::at(1200).calculate(area, &mut moved, Padding::default());
        assert!(moved[0].offscreen && !moved[3].offscreen);
        for (one, other) in strip.iter().zip(&moved) {
            assert_eq!(
                (one.rect.width, one.rect.height),
                (other.rect.width, other.rect.height)
            );
            assert_eq!(one.rect.x - other.rect.x, 800);
        }
    }

    #[test]
    fn only_whole_columns_are_in_view() {
        assert_eq!(columns_in_view(0, &[1000, 1000, 1000], 2000), 2);
        // The third would stand half in view: it is not shown.
        assert_eq!(columns_in_view(0, &[800, 800, 800], 2000), 2);
        assert_eq!(columns_in_view(1, &[800, 800, 800], 2000), 2);
        // A column wider than the view is shown, alone.
        assert_eq!(columns_in_view(0, &[2400, 800], 2000), 1);
        let area = Rect::new(100, 50, 2000, 1000);
        let mut three = columns(&[0.4, 0.4, 0.4]);
        ScrollingLayout::new(1).calculate(area, &mut three, Padding::default());
        assert!(three[0].offscreen && !three[1].offscreen && !three[2].offscreen);
        assert_eq!(three[1].rect, Rect::new(100, 50, 800, 1000));
        assert_eq!(three[2].rect, Rect::new(900, 50, 800, 1000));
        // The one out of view waits under the first of the view.
        assert_eq!(three[0].rect.x, 100);
    }

    use crate::layouts::LayoutType;
    use crate::workspace::Workspace;
    use crate::workspaces::reveal_of;
    use crate::WindowHandle;

    /// A workspace with the scrolling layout, no padding and no gaps, and
    /// so many windows, each opened with the keyboard on the one before.
    /// A strip where only whole columns are shown (as on Windows): what
    /// the tests below this one are about.
    fn strip(windows: isize) -> Workspace {
        let mut workspace = free_strip(windows);
        workspace.scroll_whole_columns = true;
        workspace
    }

    /// A strip that scrolls freely.
    fn free_strip(windows: isize) -> Workspace {
        let mut workspace = Workspace::new(1, "strip".to_string());
        workspace.layout = LayoutType::Scrolling;
        workspace.padding = Padding::uniform(0);
        workspace.container_padding = Padding::uniform(0);
        for window in 1..=windows {
            workspace.add_window(WindowHandle(window), true);
            workspace.focus_window(WindowHandle(window));
        }
        workspace
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 2000,
        height: 1000,
    };

    fn in_view(workspace: &Workspace) -> Vec<isize> {
        workspace
            .containers
            .iter()
            .filter(|column| !column.offscreen)
            .filter_map(|column| column.focused_window())
            .map(|window| window.0)
            .collect()
    }

    #[test]
    fn the_view_follows_the_keyboard_along_the_strip() {
        let mut workspace = strip(4);
        workspace.apply_layout(AREA);
        // The keyboard is on the last: the last two are in view.
        assert_eq!(in_view(&workspace), [3, 4]);
        assert!(workspace.visibility_stale);
        assert!(workspace.out_of_view(WindowHandle(1)));
        // What is out of view is hidden when the workspace is shown.
        let reveal = reveal_of(&workspace);
        assert_eq!(reveal.shown, [WindowHandle(3), WindowHandle(4)]);
        assert_eq!(reveal.hidden, [WindowHandle(1), WindowHandle(2)]);
        // One to the left: still in view, nothing moves.
        workspace.visibility_stale = false;
        workspace.focus_window(WindowHandle(3));
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [3, 4]);
        assert!(!workspace.visibility_stale);
        // One more: the view moves by one column.
        workspace.focus_window(WindowHandle(2));
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [2, 3]);
        assert!(workspace.visibility_stale);
        assert_eq!(
            workspace.containers.get(1).unwrap().rect,
            Rect::new(0, 0, 1000, 1000)
        );
    }

    #[test]
    fn a_free_strip_follows_the_keyboard_and_can_be_looked_along() {
        // Three columns of half the display each: two fit.
        let mut workspace = free_strip(3);
        workspace.apply_layout(AREA);
        // The keyboard is on the third: the first is wholly out at the left.
        assert_eq!(workspace.scroll_offset, 1000);
        assert_eq!(in_view(&workspace), [2, 3]);
        assert_eq!(workspace.containers.get(0).unwrap().rect.x, -1000);
        // Looked along by 300 pixels to the left: a part of the first
        // comes in, and nothing changes its size.
        assert!(workspace.scroll_view(-300, AREA));
        workspace.apply_layout(AREA);
        assert_eq!(workspace.scroll_offset, 700);
        assert_eq!(in_view(&workspace), [1, 2, 3]);
        assert_eq!(
            workspace.containers.get(0).unwrap().rect,
            Rect::new(-700, 0, 1000, 1000)
        );
        assert_eq!(
            workspace.containers.get(2).unwrap().rect,
            Rect::new(1300, 0, 1000, 1000)
        );
        // The view stays where it was put while the keyboard stays...
        workspace.apply_layout(AREA);
        assert_eq!(workspace.scroll_offset, 700);
        // ...and follows again when the keyboard goes to another column.
        workspace.focus_window(WindowHandle(1));
        workspace.apply_layout(AREA);
        assert_eq!(workspace.scroll_offset, 0);
        assert_eq!(in_view(&workspace), [1, 2]);
        // Not beyond the strip's ends.
        assert!(!workspace.scroll_view(-500, AREA));
        assert!(workspace.scroll_view(5000, AREA));
        assert_eq!(workspace.scroll_offset, 1000);
        // A strip of whole columns is not moved by pixels.
        let mut whole = strip(3);
        whole.apply_layout(AREA);
        assert!(!whole.scroll_view(300, AREA));
    }

    #[test]
    fn a_new_window_has_its_column_beside_the_one_with_the_keyboard() {
        let mut workspace = strip(3);
        workspace.focus_window(WindowHandle(1));
        workspace.add_window(WindowHandle(9), true);
        let order: Vec<isize> = workspace
            .containers
            .iter()
            .filter_map(|column| column.focused_window())
            .map(|window| window.0)
            .collect();
        assert_eq!(order, [1, 9, 2, 3]);
        // The keyboard stays where it was until the window takes it.
        assert_eq!(workspace.focused_window(), Some(WindowHandle(1)));
    }

    #[test]
    fn a_column_is_as_wide_as_its_window_was_made() {
        let mut workspace = strip(3);
        workspace.focus_window(WindowHandle(1));
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [1, 2]);
        // The first is made wider: the second no longer fits beside it,
        // and keeps its own width for when it is in view again.
        assert!(workspace.set_column_width(0, 1400, AREA));
        assert!(!workspace.set_column_width(0, 1400, AREA));
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [1]);
        assert_eq!(workspace.containers.get(0).unwrap().rect.width, 1400);
        workspace.focus_window(WindowHandle(2));
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [2, 3]);
        assert_eq!(workspace.containers.get(1).unwrap().rect.width, 1000);
        // With gaps, the gap round the window is the column's too: the
        // window keeps the width it was given.
        workspace.container_padding = Padding::uniform(10);
        assert!(workspace.set_column_width(1, 790, AREA));
        workspace.apply_layout(AREA);
        assert_eq!(workspace.containers.get(1).unwrap().rect.width, 790);
    }

    use gevrek_model::layout::WindowSizes;

    #[test]
    fn a_column_opens_as_wide_as_the_configuration_says() {
        let mut workspace = free_strip(0);
        workspace.set_sizes(&WindowSizes {
            column_width: 0.4,
            by_place: vec![0.7],
            split: None,
        });
        for window in 1..=3 {
            workspace.add_window(WindowHandle(window), true);
            workspace.focus_window(WindowHandle(window));
        }
        workspace.focus_window(WindowHandle(1));
        workspace.apply_layout(AREA);
        let widths: Vec<i32> = workspace
            .containers
            .iter()
            .map(|column| column.rect.width)
            .collect();
        assert_eq!(widths, [1400, 800, 800]);
        // The first is wider than half: the second stands cut off at the
        // display's edge, the third is out of view.
        assert_eq!(in_view(&workspace), [1, 2]);
        assert_eq!(
            workspace.containers.get(1).unwrap().rect,
            Rect::new(1400, 0, 800, 1000)
        );
        // Sizes changed in the configuration are given to the columns
        // there are, those made wider by hand too.
        assert!(workspace.set_column_width(2, 1200, AREA));
        assert!(workspace.set_sizes(&WindowSizes::default()));
        assert!(!workspace.set_sizes(&WindowSizes::default()));
        workspace.apply_layout(AREA);
        assert!(workspace
            .containers
            .iter()
            .all(|column| column.rect.width == 1000));
    }

    #[test]
    fn columns_rows_and_splits_start_from_the_configured_shares() {
        let mut workspace = free_strip(3);
        workspace.layout = LayoutType::Columns;
        workspace.set_sizes(&WindowSizes {
            by_place: vec![0.6],
            ..WindowSizes::default()
        });
        workspace.apply_layout(AREA);
        let widths: Vec<i32> = workspace
            .containers
            .iter()
            .map(|column| column.rect.width)
            .collect();
        assert_eq!(widths, [1200, 400, 400]);
        workspace.layout = LayoutType::Rows;
        workspace.apply_layout(AREA);
        assert_eq!(workspace.containers.get(0).unwrap().rect.height, 600);
        // BSP: the first window keeps the share it is given at the split.
        let mut bsp = free_strip(2);
        bsp.layout = LayoutType::BSP;
        bsp.set_sizes(&WindowSizes {
            split: Some(0.7),
            ..WindowSizes::default()
        });
        bsp.apply_layout(AREA);
        assert_eq!(bsp.containers.get(0).unwrap().rect.width, 1400);
    }

    #[test]
    fn under_another_layout_nothing_is_out_of_view() {
        let mut workspace = strip(4);
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [3, 4]);
        workspace.visibility_stale = false;
        workspace.layout = LayoutType::Columns;
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [1, 2, 3, 4]);
        assert!(workspace.visibility_stale);
        // Nor with tiling off.
        workspace.layout = LayoutType::Scrolling;
        workspace.apply_layout(AREA);
        workspace.visibility_stale = false;
        workspace.tiling_enabled = false;
        workspace.apply_layout(AREA);
        assert_eq!(in_view(&workspace), [1, 2, 3, 4]);
        assert!(workspace.visibility_stale);
    }

    #[test]
    fn a_columns_width_is_a_share_of_the_display() {
        assert_eq!(column_width(0.5, 2000), 1000);
        assert_eq!(column_width(1.0 / 3.0, 1800), 600);
        // Not less than a tenth, not more than all, and never nothing.
        assert_eq!(column_width(0.01, 2000), 200);
        assert_eq!(column_width(7.0, 2000), 2000);
        assert_eq!(column_width(f32::NAN, 2000), 1000);
        assert_eq!(column_width(0.5, 0), 1);
    }
}
