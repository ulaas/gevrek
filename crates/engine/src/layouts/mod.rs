mod bsp;
mod centered_master;
mod columns;
mod dwindle;
mod grid;
mod master_stack;
mod monocle;
mod room;
mod rows;
mod scrolling;
mod spiral;

pub use bsp::{
    bsp_chain_rects, bsp_edge_split, bsp_resized_ratio, bsp_split_ratio, BspLayout,
    BSP_DEFAULT_SPLIT_RATIO,
};
pub use centered_master::CenteredMasterLayout;
pub use columns::ColumnsLayout;
pub use dwindle::{DwindleDirection, DwindleLayout};
pub use gevrek_model::layout::{LayoutAxis, LayoutType, Sizing};
pub use grid::GridLayout;
pub use master_stack::{MasterPosition, MasterStackLayout, NewWindowPlacement};
pub use monocle::MonocleLayout;
pub use room::make_room;
pub use rows::RowsLayout;
pub use scrolling::{
    column_in_the_middle as scrolling_column_in_the_middle, column_width as scrolling_column_width,
    first_column as scrolling_first_column, scrolled as scrolling_scrolled,
    set_whole_columns_only as set_scrolling_whole_columns_only,
    view_offset as scrolling_view_offset, whole_columns_only as scrolling_whole_columns_only,
    ScrollingLayout, DEFAULT_WIDTH as SCROLLING_DEFAULT_WIDTH,
    WIDTH_RANGE as SCROLLING_WIDTH_RANGE,
};
pub use spiral::{SpiralDirection, SpiralLayout, SpiralStart};

use super::container::{Container, Padding, Rect};
use serde::{Deserialize, Serialize};

/// The smallest a tile is made by the layouts that give each new window
/// a share of the last one's room (BSP, dwindle, the spiral), along the
/// side that is shared. With many windows those tiles came down to a few
/// pixels, one and none.
pub const SMALLEST_TILE: i32 = 120;

/// In how many rows so many windows share a room best (the smaller
/// side of a tile as large as it can be), and that smaller side.
fn best_rows(room: &Rect, windows: usize) -> (usize, i32) {
    let windows = windows.max(1);
    (1..=windows)
        .map(|rows| {
            let columns = windows.div_ceil(rows);
            let side = (room.width / columns as i32).min(room.height / rows as i32);
            (rows, side)
        })
        // (The first of the best: the fewest rows.)
        .fold((1, i32::MIN), |best, now| if now.1 > best.1 { now } else { best })
}

/// Whether a room is shared no further: into `kept`, which one window
/// has from then on, and `rest`, in which so many windows are still to
/// find their place, side by side (or one above the other). It is not
/// where one of the two would be smaller than a tile is made, nor where
/// the rest would not hold those windows even in equal parts.
pub(crate) fn shared_no_further(
    kept: &Rect,
    rest: &Rect,
    side_by_side: bool,
    still_to_come: usize,
) -> bool {
    let smaller = if side_by_side {
        kept.width.min(rest.width)
    } else {
        kept.height.min(rest.height)
    };
    smaller < SMALLEST_TILE || best_rows(rest, still_to_come).1 < SMALLEST_TILE
}

/// A room in so many equal parts, in rows: what the window whose room
/// can be shared no further and all that come after it get. (A row with
/// fewer windows than another has wider ones; the last of a row and the
/// last row take what the division leaves over.)
pub(crate) fn in_equal_parts(room: Rect, parts: usize) -> Vec<Rect> {
    let parts = parts.max(1);
    let rows = best_rows(&room, parts).0;
    let high = room.height / rows as i32;
    let mut tiles = Vec::with_capacity(parts);
    for row in 0..rows {
        // (The windows over go to the first rows.)
        let in_row = parts / rows + usize::from(row < parts % rows);
        let y = room.y + high * row as i32;
        let height = if row == rows - 1 {
            room.y + room.height - y
        } else {
            high
        };
        let wide = room.width / in_row.max(1) as i32;
        for column in 0..in_row {
            let x = room.x + wide * column as i32;
            let width = if column == in_row - 1 {
                room.x + room.width - x
            } else {
                wide
            };
            tiles.push(Rect::new(x, y, width, height));
        }
    }
    tiles
}

/// An edge of a window used by resize-edge operations. Mirrors the runtime `Direction` enum without
/// coupling the layout engine to the runtime crate layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Trait for layout algorithms
pub trait Layout {
    /// Calculate container positions for the given work area
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding);

    /// Calculate with custom split ratios (for layouts that support it)
    fn calculate_with_ratios(
        &self,
        work_area: Rect,
        containers: &mut [Container],
        padding: Padding,
        _ratios: &[f32],
    ) {
        // Default implementation ignores ratios
        self.calculate(work_area, containers, padding);
    }
}

/// Apply a layout to containers
pub fn apply_layout(
    layout_type: LayoutType,
    work_area: Rect,
    containers: &mut [Container],
    padding: Padding,
) {
    apply_layout_with_ratios(layout_type, work_area, containers, padding, &[])
}

/// Layout configuration passed from the workspace
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub master_ratio: f32,
    pub master_count: usize,
    pub master_position: MasterPosition,
    pub dwindle_direction: DwindleDirection,
    pub dwindle_preserve_split: bool,
    pub dwindle_split_history: Vec<DwindleDirection>,
    /// Per-split ratios for the BSP chain: element i is the share of the
    /// pre-split region kept by container i when container i+1 is created.
    /// Missing entries default to 0.5.
    pub bsp_split_ratios: Vec<f32>,
    /// The first column in view under the scrolling layout.
    pub scroll_first: usize,
    /// Where the view is on the strip, in pixels; `None`: whole columns
    /// only, from `scroll_first` on.
    pub scroll_offset: Option<i32>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            master_ratio: 0.55,
            master_count: 1,
            master_position: MasterPosition::Left,
            dwindle_direction: DwindleDirection::Alternate,
            dwindle_preserve_split: false,
            dwindle_split_history: Vec::new(),
            bsp_split_ratios: Vec::new(),
            scroll_first: 0,
            scroll_offset: None,
        }
    }
}

/// Apply a layout to containers with optional split ratios
pub fn apply_layout_with_ratios(
    layout_type: LayoutType,
    work_area: Rect,
    containers: &mut [Container],
    padding: Padding,
    ratios: &[f32],
) {
    apply_layout_configured(
        layout_type,
        work_area,
        containers,
        padding,
        ratios,
        LayoutConfig::default(),
    )
}

/// Apply a layout with full workspace configuration
pub fn apply_layout_configured(
    layout_type: LayoutType,
    work_area: Rect,
    containers: &mut [Container],
    padding: Padding,
    ratios: &[f32],
    config: LayoutConfig,
) {
    if containers.is_empty() {
        return;
    }

    match layout_type {
        LayoutType::BSP => {
            let layout = BspLayout::new();
            layout.calculate_with_ratios(work_area, containers, padding, &config.bsp_split_ratios);
        }
        LayoutType::Grid => {
            let layout = GridLayout::new();
            layout.calculate(work_area, containers, padding);
        }
        LayoutType::Columns => {
            let layout = ColumnsLayout::new();
            layout.calculate_with_ratios(work_area, containers, padding, ratios);
        }
        LayoutType::Rows => {
            let layout = RowsLayout::new();
            layout.calculate_with_ratios(work_area, containers, padding, ratios);
        }
        LayoutType::Monocle => {
            let layout = MonocleLayout::new();
            layout.calculate(work_area, containers, padding);
        }
        // Nothing is tiled: the workspace keeps no containers under this
        // layout (`Workspace::sync_floating_layout`).
        LayoutType::Floating => {}
        LayoutType::Scrolling => {
            let layout = match config.scroll_offset {
                Some(offset) => ScrollingLayout::at(offset),
                None => ScrollingLayout::new(config.scroll_first),
            };
            layout.calculate(work_area, containers, padding);
        }
        LayoutType::UltrawideVerticalStack => {
            let layout = UltrawideVerticalStackLayout::with_ratio(config.master_ratio);
            layout.calculate(work_area, containers, padding);
        }
        LayoutType::MasterStack => {
            // Use workspace master_ratio and master_count instead of hardcoded defaults
            let layout = MasterStackLayout::configured(
                config.master_ratio,
                config.master_count,
                config.master_position,
            );
            layout.calculate(work_area, containers, padding);
        }
        LayoutType::CenteredMaster => {
            // Use workspace master_ratio and master_count
            let layout = CenteredMasterLayout::configured(config.master_ratio, config.master_count);
            layout.calculate(work_area, containers, padding);
        }
        LayoutType::Spiral => {
            let layout = SpiralLayout::new();
            layout.calculate(work_area, containers, padding);
        }
        LayoutType::Dwindle => {
            let history = if config.dwindle_preserve_split {
                config.dwindle_split_history
            } else {
                Vec::new()
            };
            let layout = DwindleLayout::configured(
                config.dwindle_direction,
                config.master_ratio,
                true,
                history,
            );
            layout.calculate(work_area, containers, padding);
        }
    }
}

/// Ultrawide Vertical Stack: master window on left, stack on right
pub struct UltrawideVerticalStackLayout {
    master_ratio: f32,
}

impl UltrawideVerticalStackLayout {
    pub fn new() -> Self {
        Self { master_ratio: 0.6 }
    }

    pub fn with_ratio(master_ratio: f32) -> Self {
        Self {
            master_ratio: master_ratio.clamp(0.1, 0.9),
        }
    }
}

impl Default for UltrawideVerticalStackLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for UltrawideVerticalStackLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        if containers.is_empty() {
            return;
        }

        let work_area = work_area.with_padding(padding);

        if containers.len() == 1 {
            containers[0].set_rect(work_area);
            return;
        }

        // Split into master (left) and stack (right)
        let (master_rect, stack_rect) = work_area.split_horizontal(self.master_ratio);

        // First container is master
        containers[0].set_rect(master_rect);

        // Remaining containers stacked vertically on right
        let stack_containers = &mut containers[1..];
        let stack_count = stack_containers.len();
        let stack_height = stack_rect.height / stack_count as i32;

        for (i, container) in stack_containers.iter_mut().enumerate() {
            let y = stack_rect.y + (i as i32 * stack_height);
            let height = if i == stack_count - 1 {
                // Last container takes remaining space
                stack_rect.height - (i as i32 * stack_height)
            } else {
                stack_height
            };

            container.set_rect(Rect::new(stack_rect.x, y, stack_rect.width, height));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names_round_trip_through_config_syntax() {
        for layout in LayoutType::all() {
            assert_eq!(layout.config_name().parse::<LayoutType>().unwrap(), layout);
        }
        assert_eq!(
            "Centered Master".parse::<LayoutType>().unwrap(),
            LayoutType::CenteredMaster
        );
        assert!("unknown".parse::<LayoutType>().is_err());
    }

    /// The layouts that give each new window a share of the last one's
    /// room, with windows from one to forty on two displays.
    fn shared(layout: &dyn Layout, area: Rect, windows: usize) -> Vec<Rect> {
        let mut containers: Vec<Container> = (0..windows).map(Container::new).collect();
        layout.calculate(area, &mut containers, Padding::default());
        containers.iter().map(|container| container.rect).collect()
    }

    fn sharing_layouts() -> Vec<(&'static str, Box<dyn Layout>)> {
        vec![
            ("bsp", Box::new(BspLayout::new())),
            ("dwindle", Box::new(DwindleLayout::new())),
            (
                "dwindle, one way, 0.7",
                Box::new(DwindleLayout::configured(
                    DwindleDirection::Horizontal,
                    0.7,
                    true,
                    Vec::new(),
                )),
            ),
            (
                "dwindle, backwards",
                Box::new(DwindleLayout::configured(
                    DwindleDirection::Alternate,
                    0.3,
                    false,
                    Vec::new(),
                )),
            ),
            ("spiral", Box::new(SpiralLayout::new())),
            (
                "spiral, golden",
                Box::new(SpiralLayout::configured(
                    SpiralDirection::CounterClockwise,
                    SpiralStart::TopLeft,
                    true,
                )),
            ),
        ]
    }

    /// With many windows the tiles came down to a few pixels, one and
    /// none (nine windows, split one way, at 0.7: the last were 3, 1 and
    /// 0 wide). A room that can be shared no further without a tile
    /// smaller than `SMALLEST_TILE` is shared equally by the window that
    /// has it and all that come after; every tile has a size, the tiles
    /// fill the display and none lies on another.
    #[test]
    fn a_tile_is_not_made_smaller_than_the_smallest() {
        for area in [Rect::new(0, 0, 1920, 1080), Rect::new(-2560, 40, 2560, 1400)] {
            for (name, layout) in sharing_layouts() {
                for windows in 1..=40 {
                    let tiles = shared(layout.as_ref(), area, windows);
                    let told = format!("{name}, {windows} windows on {area:?}: {tiles:?}");
                    assert_eq!(tiles.len(), windows, "{told}");
                    let mut covered: i64 = 0;
                    for (index, tile) in tiles.iter().enumerate() {
                        assert!(tile.width > 0 && tile.height > 0, "{told}");
                        assert!(
                            tile.x >= area.x
                                && tile.y >= area.y
                                && tile.x + tile.width <= area.x + area.width
                                && tile.y + tile.height <= area.y + area.height,
                            "{told}"
                        );
                        covered += i64::from(tile.width) * i64::from(tile.height);
                        for other in &tiles[index + 1..] {
                            let apart = tile.x + tile.width <= other.x
                                || other.x + other.width <= tile.x
                                || tile.y + tile.height <= other.y
                                || other.y + other.height <= tile.y;
                            assert!(apart, "{told}");
                        }
                    }
                    assert_eq!(covered, i64::from(area.width) * i64::from(area.height), "{told}");
                    // As many windows as a person has on one workspace:
                    // none smaller than the smallest, either way.
                    if windows <= 9 {
                        assert!(
                            tiles
                                .iter()
                                .all(|tile| tile.width.min(tile.height) >= SMALLEST_TILE),
                            "{told}"
                        );
                    }
                }
            }
        }
    }

    /// Where every tile is large enough, nothing is other than it was.
    #[test]
    fn few_windows_are_laid_out_as_they_were() {
        let area = Rect::new(0, 0, 1920, 1080);
        let dwindle = shared(&DwindleLayout::new(), area, 5);
        assert_eq!(
            dwindle,
            [
                Rect::new(0, 0, 960, 1080),
                Rect::new(960, 0, 960, 540),
                Rect::new(960, 540, 480, 540),
                Rect::new(1440, 540, 480, 270),
                Rect::new(1440, 810, 480, 270),
            ]
        );
        assert_eq!(shared(&BspLayout::new(), area, 5), dwindle);
        // And the room that is shared equally is the last one's that
        // could not be shared: the tiles before it stay where they were,
        // and the rest are a grid in it.
        let many = shared(&DwindleLayout::new(), area, 12);
        assert_eq!(many[..4], dwindle[..4]);
        assert_eq!(many[4], Rect::new(1440, 810, 120, 135));
        assert_eq!(many[11], Rect::new(1800, 945, 120, 135));
        assert_eq!(in_equal_parts(Rect::new(10, 20, 100, 301), 3), [
            Rect::new(10, 20, 100, 100),
            Rect::new(10, 120, 100, 100),
            Rect::new(10, 220, 100, 101),
        ]);
    }
}
