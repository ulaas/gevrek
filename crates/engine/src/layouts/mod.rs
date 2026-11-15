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
}
