//! Layout selection and options (the algorithms live in the engine).

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// An axis used by flip-layout and resize-axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutAxis {
    Horizontal,
    Vertical,
    HorizontalAndVertical,
}

impl LayoutAxis {
    pub fn config_name(&self) -> &'static str {
        match self {
            LayoutAxis::Horizontal => "horizontal",
            LayoutAxis::Vertical => "vertical",
            LayoutAxis::HorizontalAndVertical => "horizontal-and-vertical",
        }
    }

    /// The horizontal component of this axis, if any.
    pub fn includes_horizontal(&self) -> bool {
        matches!(
            self,
            LayoutAxis::Horizontal | LayoutAxis::HorizontalAndVertical
        )
    }

    /// The vertical component of this axis, if any.
    pub fn includes_vertical(&self) -> bool {
        matches!(
            self,
            LayoutAxis::Vertical | LayoutAxis::HorizontalAndVertical
        )
    }
}

impl FromStr for LayoutAxis {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        match normalized.as_str() {
            "horizontal" | "h" => Ok(Self::Horizontal),
            "vertical" | "v" => Ok(Self::Vertical),
            "horizontal-and-vertical" | "both" => Ok(Self::HorizontalAndVertical),
            _ => Err(format!(
                "Unknown axis '{value}'; use horizontal, vertical, or horizontal-and-vertical"
            )),
        }
    }
}

/// Grow/shrink direction for tiled resize commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sizing {
    Increase,
    Decrease,
}

impl Sizing {
    pub fn config_name(&self) -> &'static str {
        match self {
            Sizing::Increase => "increase",
            Sizing::Decrease => "decrease",
        }
    }

    /// Apply this sizing to a pixel delta, producing the signed delta.
    pub fn apply(&self, delta: i32) -> i32 {
        match self {
            Sizing::Increase => delta,
            Sizing::Decrease => -delta,
        }
    }
}

impl FromStr for Sizing {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "increase" | "inc" | "+" => Ok(Self::Increase),
            "decrease" | "dec" | "-" => Ok(Self::Decrease),
            _ => Err(format!(
                "Unknown sizing '{value}'; use increase or decrease"
            )),
        }
    }
}

/// Layout types supported by the window manager
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum LayoutType {
    /// Binary Space Partitioning
    BSP,
    /// Grid layout (evenly divided)
    Grid,
    /// Vertical columns
    Columns,
    /// Horizontal rows
    Rows,
    /// Single fullscreen window
    Monocle,
    /// Ultrawide vertical stack (master on left, stack on right)
    UltrawideVerticalStack,
    /// Master-Stack layout (master area with stack)
    MasterStack,
    /// Centered Master (master in center, stacks on sides)
    CenteredMaster,
    /// Spiral (Fibonacci) layout
    Spiral,
    /// Dwindle layout
    Dwindle,
    /// No tiling: every window of the workspace floats. One that opens
    /// there or is moved there is put in the middle of the display at its
    /// own size; the windows that were tiled when the layout was chosen
    /// stay where they are, and are tiled again when another is chosen.
    Floating,
    /// Columns side by side on a strip as long as it has to be, of which
    /// the display shows a part: a window that opens gets a column of its
    /// own and no other window changes its size; the view follows the
    /// keyboard.
    Scrolling,
}

impl LayoutType {
    pub fn all() -> Vec<LayoutType> {
        vec![
            LayoutType::BSP,
            LayoutType::Grid,
            LayoutType::Columns,
            LayoutType::Rows,
            LayoutType::Monocle,
            LayoutType::UltrawideVerticalStack,
            LayoutType::MasterStack,
            LayoutType::CenteredMaster,
            LayoutType::Spiral,
            LayoutType::Dwindle,
            LayoutType::Floating,
            LayoutType::Scrolling,
        ]
    }

    pub fn display_name(&self) -> &str {
        match self {
            LayoutType::BSP => "Binary Space Partition",
            LayoutType::Grid => "Grid",
            LayoutType::Columns => "Columns",
            LayoutType::Rows => "Rows",
            LayoutType::Monocle => "Monocle (Fullscreen)",
            LayoutType::UltrawideVerticalStack => "Ultrawide Vertical Stack",
            LayoutType::MasterStack => "Master-Stack",
            LayoutType::CenteredMaster => "Centered Master",
            LayoutType::Spiral => "Spiral (Fibonacci)",
            LayoutType::Dwindle => "Dwindle",
            LayoutType::Floating => "Floating",
            LayoutType::Scrolling => "Scrolling",
        }
    }

    /// Get a short description of the layout
    pub fn description(&self) -> &str {
        match self {
            LayoutType::BSP => "Recursive binary splits alternating horizontal/vertical",
            LayoutType::Grid => "Equal-sized grid of windows",
            LayoutType::Columns => "Vertical columns side by side",
            LayoutType::Rows => "Horizontal rows stacked",
            LayoutType::Monocle => "Single focused window fullscreen",
            LayoutType::UltrawideVerticalStack => "Large master on left, vertical stack on right",
            LayoutType::MasterStack => "Configurable master area with stacked secondaries",
            LayoutType::CenteredMaster => "Master centered with stacks on both sides",
            LayoutType::Spiral => "Golden ratio spiral pattern",
            LayoutType::Dwindle => "Corner dwindle pattern",
            LayoutType::Floating => "No tiling: windows float, new ones in the middle",
            LayoutType::Scrolling => "Columns on an endless strip; the view follows the keyboard",
        }
    }

    pub fn config_name(&self) -> &'static str {
        match self {
            LayoutType::BSP => "bsp",
            LayoutType::Grid => "grid",
            LayoutType::Columns => "columns",
            LayoutType::Rows => "rows",
            LayoutType::Monocle => "monocle",
            LayoutType::UltrawideVerticalStack => "ultrawide",
            LayoutType::MasterStack => "master",
            LayoutType::CenteredMaster => "centered",
            LayoutType::Spiral => "spiral",
            LayoutType::Dwindle => "dwindle",
            LayoutType::Floating => "floating",
            LayoutType::Scrolling => "scrolling",
        }
    }
}

impl FromStr for LayoutType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        match normalized.as_str() {
            "bsp" => Ok(Self::BSP),
            "grid" => Ok(Self::Grid),
            "columns" | "column" => Ok(Self::Columns),
            "rows" | "row" => Ok(Self::Rows),
            "monocle" => Ok(Self::Monocle),
            "ultrawide-vertical-stack" | "ultrawide" => Ok(Self::UltrawideVerticalStack),
            "master-stack" | "master" => Ok(Self::MasterStack),
            "centered-master" | "centered" => Ok(Self::CenteredMaster),
            "spiral" => Ok(Self::Spiral),
            "dwindle" => Ok(Self::Dwindle),
            "floating" | "float" => Ok(Self::Floating),
            "scrolling" | "scroll" => Ok(Self::Scrolling),
            _ => Err(format!(
                "Unknown layout '{value}'; use bsp, grid, columns, rows, monocle, ultrawide, master, centered, spiral, dwindle, floating, or scrolling"
            )),
        }
    }
}

/// Split direction preference for dwindle layout
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
pub enum DwindleDirection {
    /// Always alternate horizontal/vertical
    #[default]
    Alternate,
    /// Prefer horizontal splits
    Horizontal,
    /// Prefer vertical splits
    Vertical,
}

impl DwindleDirection {
    pub fn config_name(self) -> &'static str {
        match self {
            Self::Alternate => "alternate",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Alternate => "Alternate",
            Self::Horizontal => "Force horizontal",
            Self::Vertical => "Force vertical",
        }
    }
}

impl FromStr for DwindleDirection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "alternate" | "auto" => Ok(Self::Alternate),
            "horizontal" | "h" => Ok(Self::Horizontal),
            "vertical" | "v" => Ok(Self::Vertical),
            _ => Err(format!(
                "Unknown Dwindle split direction '{value}'; use alternate, horizontal, or vertical"
            )),
        }
    }
}

/// Position of the master area
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
pub enum MasterPosition {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

/// Where a newly tiled window enters a master-family workspace.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum NewWindowPlacement {
    /// Append after the existing stack, preserving the current master.
    #[default]
    StackEnd,
    /// Insert at index zero so the new window becomes the master.
    Master,
    /// Insert immediately after the currently focused container.
    AfterFocused,
}

/// How a workspace places a newly added tiled window.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WindowContainerBehaviour {
    /// Every new window gets its own container (the default column model).
    #[default]
    Column,
    /// New windows append to the currently focused container (auto-stack).
    Append,
}

impl WindowContainerBehaviour {
    pub fn toggled(self) -> Self {
        match self {
            Self::Column => Self::Append,
            Self::Append => Self::Column,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Column => "Column",
            Self::Append => "Append",
        }
    }
}

/// How big a workspace's windows are, where its layout lets them be
/// sized. Every size is a share of the display (`0.5`: half of it). The
/// master layouts and Dwindle are sized by the workspace's
/// `master_ratio` and `master_count`; Grid, Monocle and Floating have
/// nothing to size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WindowSizes {
    /// Scrolling: how wide a column is when its window opens, as a share
    /// of the display's width (0.1 to 1.0). Columns wider than half leave
    /// the next one partly in view, cut off at the display's edge, until
    /// the view moves along the strip.
    pub column_width: f32,
    /// The sizes of the first windows, by their place (the first window,
    /// the second, ...), each a share of the display (0.1 to 1.0).
    /// Scrolling: each column's width; a place not given takes
    /// `column_width`. Columns: each column's width; Rows: each row's
    /// height; the windows past those given share what is left evenly, and
    /// when the shares come to more or less than the whole display they
    /// are scaled to fill it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_place: Vec<f32>,
    /// BSP and Spiral: the share of its room a window keeps when the next
    /// window splits it (0.1 to 0.9). Not given: BSP halves, Spiral keeps
    /// the golden share.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<f32>,
}

impl Default for WindowSizes {
    fn default() -> Self {
        Self {
            column_width: Self::COLUMN_WIDTH,
            by_place: Vec::new(),
            split: None,
        }
    }
}

impl WindowSizes {
    /// The width of a scrolling column until it is given another.
    pub const COLUMN_WIDTH: f32 = 0.5;
    /// The least and the most share a window can be given.
    pub const RANGE: (f32, f32) = (0.1, 1.0);
    /// The least and the most share a window keeps when it is split.
    pub const SPLIT_RANGE: (f32, f32) = (0.1, 0.9);
    /// How many places can be given a size of their own.
    pub const MOST_PLACES: usize = 16;

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    fn within(share: f32, (least, most): (f32, f32), otherwise: f32) -> f32 {
        if share.is_finite() {
            share.clamp(least, most)
        } else {
            otherwise
        }
    }

    /// Scrolling: the width of the column at `place` (0: the first), as a
    /// share of the display's width.
    pub fn column_width_at(&self, place: usize) -> f32 {
        let default = Self::within(self.column_width, Self::RANGE, Self::COLUMN_WIDTH);
        self.by_place
            .get(place)
            .map_or(default, |share| Self::within(*share, Self::RANGE, default))
    }

    /// Columns and Rows: the share of each of `count` windows, coming to
    /// the whole display. Empty when no place is given a size (every
    /// window has as much as the others).
    pub fn shares(&self, count: usize) -> Vec<f32> {
        let mut shares: Vec<f32> = self
            .by_place
            .iter()
            .take(count)
            .map(|share| Self::within(*share, Self::RANGE, 1.0 / count as f32))
            .collect();
        if shares.is_empty() {
            return shares;
        }
        let rest = count - shares.len();
        if rest > 0 {
            let used: f32 = shares.iter().sum();
            // (What is left, or a little each where nothing is.)
            let each = ((1.0 - used) / rest as f32).max(Self::RANGE.0);
            shares.extend(std::iter::repeat_n(each, rest));
        }
        let sum: f32 = shares.iter().sum();
        for share in &mut shares {
            *share /= sum;
        }
        shares
    }

    /// BSP and Spiral: the share a window keeps when it is split, when one
    /// is given.
    pub fn split_share(&self) -> Option<f32> {
        self.split
            .filter(|share| share.is_finite())
            .map(|share| share.clamp(Self::SPLIT_RANGE.0, Self::SPLIT_RANGE.1))
    }
}

#[cfg(test)]
mod window_sizes_tests {
    use super::WindowSizes;

    fn close(one: &[f32], other: &[f32]) -> bool {
        one.len() == other.len() && one.iter().zip(other).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn a_scrolling_column_is_as_wide_as_its_place_says() {
        let sizes = WindowSizes {
            column_width: 0.4,
            by_place: vec![0.7],
            split: None,
        };
        assert_eq!(sizes.column_width_at(0), 0.7);
        assert_eq!(sizes.column_width_at(1), 0.4);
        assert_eq!(sizes.column_width_at(9), 0.4);
        assert_eq!(WindowSizes::default().column_width_at(0), 0.5);
        // Nothing past the range, and nothing that is not a number.
        let wild = WindowSizes {
            column_width: f32::NAN,
            by_place: vec![3.0, 0.0],
            split: Some(2.0),
        };
        assert_eq!(wild.column_width_at(0), 1.0);
        assert_eq!(wild.column_width_at(1), 0.1);
        assert_eq!(wild.column_width_at(2), 0.5);
        assert_eq!(wild.split_share(), Some(0.9));
    }

    #[test]
    fn columns_and_rows_share_the_whole_display() {
        assert!(WindowSizes::default().shares(3).is_empty());
        let sizes = WindowSizes {
            by_place: vec![0.6],
            ..WindowSizes::default()
        };
        // The first has its share, the others what is left.
        assert!(close(&sizes.shares(3), &[0.6, 0.2, 0.2]));
        assert!(close(&sizes.shares(1), &[1.0]));
        assert!(close(&sizes.shares(0), &[]));
        // More than the whole: scaled to fill it.
        let over = WindowSizes {
            by_place: vec![0.8, 0.8],
            ..WindowSizes::default()
        };
        assert!(close(&over.shares(2), &[0.5, 0.5]));
        let crowded = over.shares(3);
        assert!(crowded[2] > 0.0 && (crowded.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sizes_left_as_they_are_are_not_written() {
        let text = serde_json::to_string(&WindowSizes {
            column_width: 0.6,
            ..WindowSizes::default()
        })
        .unwrap();
        assert_eq!(text, r#"{"column_width":0.6}"#);
        let read: WindowSizes = serde_json::from_str("{}").unwrap();
        assert!(read.is_default());
    }
}
