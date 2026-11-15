use super::ring::Ring;
use crate::WindowHandle;
pub use gevrek_model::geometry::{Padding, Rect};
use serde::{Deserialize, Serialize};

/// Represents a rectangular area that holds one or more windows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    /// Windows in this container
    pub windows: Ring<WindowHandle>,

    /// Position and size of this container
    pub rect: Rect,

    /// Percentage of parent space this container occupies (for resizing)
    pub layout_percentage: f32,

    /// ID for tracking
    pub id: usize,

    /// Under the scrolling layout: the share of the display's width this
    /// container has as a column.
    #[serde(default = "default_strip_width")]
    pub strip_width: f32,

    /// Under the scrolling layout: this column is out of view, and its
    /// windows are hidden. (Set by the layout at every pass.)
    #[serde(default)]
    pub offscreen: bool,
}

fn default_strip_width() -> f32 {
    crate::layouts::SCROLLING_DEFAULT_WIDTH
}

impl Container {
    pub fn new(id: usize) -> Self {
        Self {
            windows: Ring::new(),
            rect: Rect::default(),
            layout_percentage: 1.0,
            id,
            strip_width: default_strip_width(),
            offscreen: false,
        }
    }

    /// Add a window to this container
    pub fn add_window(&mut self, handle: WindowHandle) {
        self.windows.push(handle);
    }

    /// Remove a window from this container
    pub fn remove_window(&mut self, handle: WindowHandle) -> bool {
        let mut removed = false;
        let mut idx_to_remove = None;

        for (idx, win) in self.windows.iter().enumerate() {
            if *win == handle {
                idx_to_remove = Some(idx);
                break;
            }
        }

        if let Some(idx) = idx_to_remove {
            self.windows.remove_at(idx);
            removed = true;
        }

        removed
    }

    /// Get the focused window in this container
    pub fn focused_window(&self) -> Option<WindowHandle> {
        self.windows.focused().copied()
    }

    /// Focus a particular window already held by this container.
    pub fn focus_window(&mut self, handle: WindowHandle) -> bool {
        let Some(index) = self.windows.iter().position(|window| *window == handle) else {
            return false;
        };
        self.windows.focus_at(index)
    }

    /// Check if container contains a specific window
    pub fn contains_window(&self, handle: WindowHandle) -> bool {
        self.windows.iter().any(|w| *w == handle)
    }

    /// Get all windows in this container
    pub fn all_windows(&self) -> Vec<WindowHandle> {
        self.windows.iter().copied().collect()
    }

    /// Check if container is empty
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Set the container's rectangle
    pub fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }

    /// Get all non-focused windows (for hiding in stacked mode)
    pub fn non_focused_windows(&self) -> Vec<WindowHandle> {
        let focused = self.focused_window();
        self.windows
            .iter()
            .filter(|w| Some(**w) != focused)
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_split_horizontal() {
        let rect = Rect::new(0, 0, 100, 100);
        let (left, right) = rect.split_horizontal(0.5);

        assert_eq!(left, Rect::new(0, 0, 50, 100));
        assert_eq!(right, Rect::new(50, 0, 50, 100));
    }

    #[test]
    fn test_rect_split_vertical() {
        let rect = Rect::new(0, 0, 100, 100);
        let (top, bottom) = rect.split_vertical(0.5);

        assert_eq!(top, Rect::new(0, 0, 100, 50));
        assert_eq!(bottom, Rect::new(0, 50, 100, 50));
    }

    #[test]
    fn test_rect_with_padding() {
        let rect = Rect::new(0, 0, 100, 100);
        let padded = rect.with_padding(Padding::uniform(10));

        assert_eq!(padded, Rect::new(10, 10, 80, 80));
    }

    #[test]
    fn test_container_window_management() {
        let mut container = Container::new(0);
        let handle1 = WindowHandle(1);
        let handle2 = WindowHandle(2);

        container.add_window(handle1);
        container.add_window(handle2);

        assert_eq!(container.windows.len(), 2);
        assert!(container.contains_window(handle1));
        assert!(container.focus_window(handle1));
        assert_eq!(container.focused_window(), Some(handle1));
        assert!(!container.focus_window(WindowHandle(99)));

        container.remove_window(handle1);
        assert_eq!(container.windows.len(), 1);
        assert!(!container.contains_window(handle1));
    }
}
