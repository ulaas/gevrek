//! Rectangles and padding, in logical pixels.

use serde::{Deserialize, Serialize};

/// Rectangle representing position and size
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Split rectangle horizontally at the given ratio (0.0 to 1.0)
    pub fn split_horizontal(&self, ratio: f32) -> (Rect, Rect) {
        let split_width = (self.width as f32 * ratio) as i32;

        let left = Rect::new(self.x, self.y, split_width, self.height);
        let right = Rect::new(
            self.x + split_width,
            self.y,
            self.width - split_width,
            self.height,
        );

        (left, right)
    }

    /// Split rectangle vertically at the given ratio (0.0 to 1.0)
    pub fn split_vertical(&self, ratio: f32) -> (Rect, Rect) {
        let split_height = (self.height as f32 * ratio) as i32;

        let top = Rect::new(self.x, self.y, self.width, split_height);
        let bottom = Rect::new(
            self.x,
            self.y + split_height,
            self.width,
            self.height - split_height,
        );

        (top, bottom)
    }

    /// Apply padding to rectangle
    pub fn with_padding(&self, padding: Padding) -> Rect {
        Rect::new(
            self.x + padding.left,
            self.y + padding.top,
            (self.width - padding.left - padding.right).max(0),
            (self.height - padding.top - padding.bottom).max(0),
        )
    }

    /// Get center point
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    /// Check if point is inside rectangle
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// Get area
    pub fn area(&self) -> i32 {
        self.width * self.height
    }
}

impl Default for Rect {
    fn default() -> Self {
        Self::new(0, 0, 0, 0)
    }
}

/// Padding for containers and workspaces
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Padding {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Padding {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn uniform(value: i32) -> Self {
        Self::new(value, value, value, value)
    }

    pub fn symmetric(horizontal: i32, vertical: i32) -> Self {
        Self::new(horizontal, vertical, horizontal, vertical)
    }
}

impl Default for Padding {
    fn default() -> Self {
        Self::new(0, 0, 0, 0)
    }
}

/// A point, e.g. the pointer position.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
