use super::Layout;
use crate::container::{Container, Padding, Rect};

/// Grid layout - evenly divides space into a grid
pub struct GridLayout;

impl GridLayout {
    pub fn new() -> Self {
        Self
    }

    /// Calculate optimal grid dimensions for n containers
    fn calculate_grid_dimensions(count: usize) -> (usize, usize) {
        if count == 0 {
            return (0, 0);
        }

        let sqrt = (count as f64).sqrt();
        let cols = sqrt.ceil() as usize;
        let rows = (count as f64 / cols as f64).ceil() as usize;

        (cols, rows)
    }
}

impl Default for GridLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for GridLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        if containers.is_empty() {
            return;
        }

        let work_area = work_area.with_padding(padding);

        let (cols, rows) = Self::calculate_grid_dimensions(containers.len());
        let n = containers.len();

        let cell_height = work_area.height / rows as i32;

        for (idx, container) in containers.iter_mut().enumerate() {
            let row = idx / cols;

            // Calculate how many items are in THIS row
            // Last row may have fewer items than cols
            let items_in_row = if row == rows - 1 {
                n - (row * cols) // remaining items
            } else {
                cols
            };

            let col = idx % cols;
            let row_cell_width = work_area.width / items_in_row as i32;

            let x = work_area.x + (col as i32 * row_cell_width);
            let y = work_area.y + (row as i32 * cell_height);

            // Last column in this row gets remaining width (handles rounding)
            let width = if col == items_in_row - 1 {
                work_area.width - (col as i32 * row_cell_width)
            } else {
                row_cell_width
            };

            // Last row gets remaining height
            let height = if row == rows - 1 {
                work_area.height - (row as i32 * cell_height)
            } else {
                cell_height
            };

            container.set_rect(Rect::new(x, y, width, height));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_dimensions() {
        assert_eq!(GridLayout::calculate_grid_dimensions(1), (1, 1));
        assert_eq!(GridLayout::calculate_grid_dimensions(2), (2, 1));
        assert_eq!(GridLayout::calculate_grid_dimensions(3), (2, 2));
        assert_eq!(GridLayout::calculate_grid_dimensions(4), (2, 2));
        assert_eq!(GridLayout::calculate_grid_dimensions(5), (3, 2));
        assert_eq!(GridLayout::calculate_grid_dimensions(9), (3, 3));
    }

    #[test]
    fn test_grid_layout() {
        let layout = GridLayout::new();
        let mut containers = vec![
            Container::new(0),
            Container::new(1),
            Container::new(2),
            Container::new(3),
        ];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // 4 containers should be 2x2 grid
        assert_eq!(containers[0].rect, Rect::new(0, 0, 500, 500));
        assert_eq!(containers[1].rect, Rect::new(500, 0, 500, 500));
        assert_eq!(containers[2].rect, Rect::new(0, 500, 500, 500));
        assert_eq!(containers[3].rect, Rect::new(500, 500, 500, 500));
    }
}
