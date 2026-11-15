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
