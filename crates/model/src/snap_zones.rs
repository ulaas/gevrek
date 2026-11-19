//! Snap zones: layouts and settings.

use crate::geometry::Rect;
use serde::{Deserialize, Serialize};

/// A snap zone definition (relative to monitor work area)
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SnapZone {
    /// Unique identifier for this zone
    pub id: usize,
    /// Zone name for display
    pub name: String,
    /// Left position as percentage of monitor width (0.0 to 1.0)
    pub x_percent: f32,
    /// Top position as percentage of monitor height (0.0 to 1.0)
    pub y_percent: f32,
    /// Width as percentage of monitor width (0.0 to 1.0)
    pub width_percent: f32,
    /// Height as percentage of monitor height (0.0 to 1.0)
    pub height_percent: f32,
    /// Gap/margin in pixels (applied inside zone bounds)
    pub gap: i32,
    /// Priority for overlapping zones (higher = selected first when ambiguous)
    pub priority: i32,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl SnapZone {
    pub fn new(id: usize, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            extra: Default::default(),
            id,
            name: format!("Zone {}", id),
            x_percent: x.clamp(0.0, 1.0),
            y_percent: y.clamp(0.0, 1.0),
            width_percent: width.clamp(0.0, 1.0),
            height_percent: height.clamp(0.0, 1.0),
            gap: 8,
            priority: 0,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_gap(mut self, gap: i32) -> Self {
        self.gap = gap;
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Calculate the actual pixel rect for this zone on a given work area
    pub fn to_rect(&self, work_area: &Rect) -> Rect {
        let x = work_area.x + (work_area.width as f32 * self.x_percent) as i32;
        let y = work_area.y + (work_area.height as f32 * self.y_percent) as i32;
        let width = (work_area.width as f32 * self.width_percent) as i32;
        let height = (work_area.height as f32 * self.height_percent) as i32;

        // Apply gap
        // (A gap of any size out of a file: none of this overflows.)
        let both = self.gap.saturating_mul(2);
        Rect::new(
            x.saturating_add(self.gap),
            y.saturating_add(self.gap),
            width.saturating_sub(both).max(100),
            height.saturating_sub(both).max(100),
        )
    }

    /// Check if a point is inside this zone (using pixel rect)
    pub fn contains_point(&self, work_area: &Rect, x: i32, y: i32) -> bool {
        let rect = self.to_rect(work_area);
        x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
    }

    /// Check if a rect overlaps with this zone
    pub fn overlaps(&self, work_area: &Rect, other: &Rect) -> f32 {
        let rect = self.to_rect(work_area);

        let x1 = rect.x.max(other.x);
        let y1 = rect.y.max(other.y);
        let x2 = (rect.x + rect.width).min(other.x + other.width);
        let y2 = (rect.y + rect.height).min(other.y + other.height);

        if x2 <= x1 || y2 <= y1 {
            return 0.0;
        }

        let overlap_area = (x2 - x1) * (y2 - y1);
        let other_area = other.width * other.height;

        if other_area == 0 {
            return 0.0;
        }

        overlap_area as f32 / other_area as f32
    }
}

/// A layout preset containing multiple zones
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SnapZoneLayout {
    /// Layout identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Description of the layout
    pub description: String,
    /// Zones in this layout
    pub zones: Vec<SnapZone>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl SnapZoneLayout {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            extra: Default::default(),
            id: id.into(),
            name: name.into(),
            description: String::new(),
            zones: Vec::new(),
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn add_zone(mut self, zone: SnapZone) -> Self {
        self.zones.push(zone);
        self
    }
}

/// Configuration for zone snapping behavior
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SnapZoneConfig {
    /// Whether zone snapping is enabled
    pub enabled: bool,
    /// Show zone preview during window drag
    pub show_preview: bool,
    /// Highlight zones while dragging
    pub highlight_zones: bool,
    /// Minimum overlap percentage to trigger zone snap (0.0 to 1.0)
    pub snap_threshold: f32,
    /// Key to hold while dropping a window for it to snap to a zone (None =
    /// every drop snaps). Shift by default: zones cover the whole screen, so
    /// without a key every drag of a tiled window would end in a zone, and
    /// a window in a zone is floating.
    pub activation_modifier: Option<SnapZoneModifier>,
    /// Preview opacity (0.0 to 1.0)
    pub preview_opacity: f32,
    /// Preview color (RGB)
    pub preview_color: (u8, u8, u8),
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for SnapZoneConfig {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: true,
            show_preview: true,
            highlight_zones: true,
            snap_threshold: 0.3,
            activation_modifier: Some(SnapZoneModifier::Shift),
            preview_opacity: 0.3,
            preview_color: (0, 120, 215),
        }
    }
}

/// Modifier key for zone activation
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub enum SnapZoneModifier {
    Shift,
    Control,
    Alt,
    Win,
}

/// Preset zone layouts
pub mod presets {
    use super::*;

    /// Single full-screen zone
    pub fn focus() -> SnapZoneLayout {
        SnapZoneLayout::new("focus", "Focus")
            .with_description("Single window takes full screen")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 1.0, 1.0).with_name("Full"))
    }

    /// Two equal columns
    pub fn columns_2() -> SnapZoneLayout {
        SnapZoneLayout::new("columns_2", "Two Columns")
            .with_description("Two equal-width columns")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 0.5, 1.0).with_name("Left"))
            .add_zone(SnapZone::new(1, 0.5, 0.0, 0.5, 1.0).with_name("Right"))
    }

    /// Three equal columns
    pub fn columns_3() -> SnapZoneLayout {
        SnapZoneLayout::new("columns_3", "Three Columns")
            .with_description("Three equal-width columns")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 0.333, 1.0).with_name("Left"))
            .add_zone(SnapZone::new(1, 0.333, 0.0, 0.334, 1.0).with_name("Center"))
            .add_zone(SnapZone::new(2, 0.667, 0.0, 0.333, 1.0).with_name("Right"))
    }

    /// Two equal rows
    pub fn rows_2() -> SnapZoneLayout {
        SnapZoneLayout::new("rows_2", "Two Rows")
            .with_description("Two equal-height rows")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 1.0, 0.5).with_name("Top"))
            .add_zone(SnapZone::new(1, 0.0, 0.5, 1.0, 0.5).with_name("Bottom"))
    }

    /// Grid 2x2
    pub fn grid_2x2() -> SnapZoneLayout {
        SnapZoneLayout::new("grid_2x2", "2x2 Grid")
            .with_description("Four equal quadrants")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 0.5, 0.5).with_name("Top Left"))
            .add_zone(SnapZone::new(1, 0.5, 0.0, 0.5, 0.5).with_name("Top Right"))
            .add_zone(SnapZone::new(2, 0.0, 0.5, 0.5, 0.5).with_name("Bottom Left"))
            .add_zone(SnapZone::new(3, 0.5, 0.5, 0.5, 0.5).with_name("Bottom Right"))
    }

    /// Priority grid (large left, two stacked right)
    pub fn priority_grid() -> SnapZoneLayout {
        SnapZoneLayout::new("priority_grid", "Priority Grid")
            .with_description("Large main area with two side zones")
            .add_zone(
                SnapZone::new(0, 0.0, 0.0, 0.65, 1.0)
                    .with_name("Main")
                    .with_priority(1),
            )
            .add_zone(SnapZone::new(1, 0.65, 0.0, 0.35, 0.5).with_name("Top Right"))
            .add_zone(SnapZone::new(2, 0.65, 0.5, 0.35, 0.5).with_name("Bottom Right"))
    }

    /// Ultrawide layout (centered with side columns)
    pub fn ultrawide() -> SnapZoneLayout {
        SnapZoneLayout::new("ultrawide", "Ultrawide")
            .with_description("Centered main area for ultrawide monitors")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 0.2, 1.0).with_name("Left"))
            .add_zone(
                SnapZone::new(1, 0.2, 0.0, 0.6, 1.0)
                    .with_name("Center")
                    .with_priority(1),
            )
            .add_zone(SnapZone::new(2, 0.8, 0.0, 0.2, 1.0).with_name("Right"))
    }

    /// Master-stack style
    pub fn master_stack() -> SnapZoneLayout {
        SnapZoneLayout::new("master_stack", "Master Stack")
            .with_description("Large master on left, three stacked on right")
            .add_zone(
                SnapZone::new(0, 0.0, 0.0, 0.6, 1.0)
                    .with_name("Master")
                    .with_priority(1),
            )
            .add_zone(SnapZone::new(1, 0.6, 0.0, 0.4, 0.333).with_name("Stack 1"))
            .add_zone(SnapZone::new(2, 0.6, 0.333, 0.4, 0.334).with_name("Stack 2"))
            .add_zone(SnapZone::new(3, 0.6, 0.667, 0.4, 0.333).with_name("Stack 3"))
    }

    /// Rows of 3 (6 zones)
    pub fn rows_of_3() -> SnapZoneLayout {
        SnapZoneLayout::new("rows_of_3", "Rows of 3")
            .with_description("Two rows with three columns each")
            .add_zone(SnapZone::new(0, 0.0, 0.0, 0.333, 0.5).with_name("Top Left"))
            .add_zone(SnapZone::new(1, 0.333, 0.0, 0.334, 0.5).with_name("Top Center"))
            .add_zone(SnapZone::new(2, 0.667, 0.0, 0.333, 0.5).with_name("Top Right"))
            .add_zone(SnapZone::new(3, 0.0, 0.5, 0.333, 0.5).with_name("Bottom Left"))
            .add_zone(SnapZone::new(4, 0.333, 0.5, 0.334, 0.5).with_name("Bottom Center"))
            .add_zone(SnapZone::new(5, 0.667, 0.5, 0.333, 0.5).with_name("Bottom Right"))
    }

    /// Get all preset layouts
    pub fn all() -> Vec<SnapZoneLayout> {
        vec![
            focus(),
            columns_2(),
            columns_3(),
            rows_2(),
            grid_2x2(),
            priority_grid(),
            ultrawide(),
            master_stack(),
            rows_of_3(),
        ]
    }
}
