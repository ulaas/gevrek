use super::Layout;
use crate::container::{Container, Padding, Rect};

/// Columns layout - arranges containers in vertical columns
pub struct ColumnsLayout;

impl ColumnsLayout {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ColumnsLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for ColumnsLayout {
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

        let mut current_x = work_area.x;

        for (idx, container) in containers.iter_mut().enumerate() {
            let width = if use_ratios {
                // Use the ratio for this container
                (work_area.width as f32 * ratios[idx]).round() as i32
            } else {
                // Equal sizing
                work_area.width / container_count as i32
            };

            // Last column gets remaining width to account for rounding
            let final_width = if idx == container_count - 1 {
                work_area.x + work_area.width - current_x
            } else {
                width
            };

            container.set_rect(Rect::new(
                current_x,
                work_area.y,
                final_width,
                work_area.height,
            ));
            current_x += final_width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_columns_layout() {
        let layout = ColumnsLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 900, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, Rect::new(0, 0, 300, 1000));
        assert_eq!(containers[1].rect, Rect::new(300, 0, 300, 1000));
        assert_eq!(containers[2].rect, Rect::new(600, 0, 300, 1000));
    }
}
