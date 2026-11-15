use super::Layout;
use crate::container::{Container, Padding, Rect};

/// Rows layout - arranges containers in horizontal rows
pub struct RowsLayout;

impl RowsLayout {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RowsLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for RowsLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        // Use equal ratios when none provided
        self.calculate_with_ratios(work_area, containers, padding, &[]);
    }

    fn calculate_with_ratios(
        &self,
        work_area: Rect,
        containers: &mut [Container],
        padding: Padding,
        ratios: &[f32],
    ) {
        if containers.is_empty() {
            return;
        }

        let work_area = work_area.with_padding(padding);
        let container_count = containers.len();

        // Use provided ratios or fall back to equal sizing
        let use_ratios = ratios.len() == container_count;

        let mut current_y = work_area.y;

        for (idx, container) in containers.iter_mut().enumerate() {
            let height = if use_ratios {
                // Use the ratio for this container
                (work_area.height as f32 * ratios[idx]).round() as i32
            } else {
                // Equal sizing
                work_area.height / container_count as i32
            };

            // Last row gets remaining height to account for rounding
            let final_height = if idx == container_count - 1 {
                work_area.y + work_area.height - current_y
            } else {
                height
            };

            container.set_rect(Rect::new(
                work_area.x,
                current_y,
                work_area.width,
                final_height,
            ));
            current_y += final_height;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rows_layout() {
        let layout = RowsLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 900);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, Rect::new(0, 0, 1000, 300));
        assert_eq!(containers[1].rect, Rect::new(0, 300, 1000, 300));
        assert_eq!(containers[2].rect, Rect::new(0, 600, 1000, 300));
    }
}
