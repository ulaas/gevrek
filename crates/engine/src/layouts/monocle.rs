use super::Layout;
use crate::container::{Container, Padding, Rect};

/// Monocle layout - shows only the focused container fullscreen
/// All other containers are hidden/minimized
pub struct MonocleLayout;

impl MonocleLayout {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MonocleLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for MonocleLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        if containers.is_empty() {
            return;
        }

        let work_area = work_area.with_padding(padding);

        // All containers get the full work area (only focused one will be shown)
        for container in containers.iter_mut() {
            container.set_rect(work_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monocle_layout() {
        let layout = MonocleLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // All containers should have the same rect (full screen)
        for container in &containers {
            assert_eq!(container.rect, work_area);
        }
    }
}
