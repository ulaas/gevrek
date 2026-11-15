use super::Layout;
use crate::container::{Container, Padding, Rect};
pub use gevrek_model::layout::DwindleDirection;

/// Dwindle Layout
///
/// Similar to BSP but with more predictable behavior. Each new window
/// splits the LAST window's space (not the previous window like BSP).
/// This creates a "dwindling" pattern where windows get progressively smaller.
///
/// Example with alternate splits:
/// ```text
/// ┌────────────┬───────────┐
/// │            │           │
/// │     1      │     2     │
/// │            ├─────┬─────┤
/// │            │  3  │  4  │
/// │            │     ├──┬──┤
/// │            │     │5 │6 │
/// └────────────┴─────┴──┴──┘
/// ```
///
/// Unlike BSP (which is spiral), dwindle always splits in a corner direction,
/// creating a "staircase" pattern.
pub struct DwindleLayout {
    /// Split direction preference
    pub direction: DwindleDirection,
    /// Split ratio (0.3 to 0.7, default 0.5)
    pub split_ratio: f32,
    /// Whether to grow right/bottom (true) or left/top (false)
    pub grow_forward: bool,
    /// Concrete directions retained by a workspace when preserve-split is on.
    pub split_history: Vec<DwindleDirection>,
}

impl DwindleLayout {
    pub fn new() -> Self {
        Self {
            direction: DwindleDirection::Alternate,
            split_ratio: 0.5,
            grow_forward: true,
            split_history: Vec::new(),
        }
    }

    pub fn with_ratio(ratio: f32) -> Self {
        Self {
            split_ratio: ratio.clamp(0.3, 0.7),
            ..Self::new()
        }
    }

    pub fn with_direction(direction: DwindleDirection) -> Self {
        Self {
            direction,
            ..Self::new()
        }
    }

    pub fn configured(
        direction: DwindleDirection,
        ratio: f32,
        grow_forward: bool,
        split_history: Vec<DwindleDirection>,
    ) -> Self {
        Self {
            direction,
            split_ratio: ratio.clamp(0.3, 0.7),
            grow_forward,
            split_history,
        }
    }

    fn should_split_horizontal(&self, depth: usize, _rect: &Rect) -> bool {
        if let Some(direction) = self.split_history.get(depth) {
            return matches!(direction, DwindleDirection::Horizontal);
        }
        match self.direction {
            DwindleDirection::Alternate => depth.is_multiple_of(2),
            DwindleDirection::Horizontal => true,
            DwindleDirection::Vertical => false,
        }
    }
}

impl Default for DwindleLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for DwindleLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        let work_area = work_area.with_padding(padding);
        let n = containers.len();

        if n == 0 {
            return;
        }

        if n == 1 {
            containers[0].set_rect(work_area);
            return;
        }

        // Dwindle: each new window splits from the last window's space
        // Unlike BSP which modifies the previous window, dwindle keeps
        // the previous windows stable and only modifies the last one

        let mut rects = Vec::with_capacity(n);
        rects.push(work_area);

        for i in 1..n {
            // Get the last rect and split it
            let last_rect = rects[i - 1];
            let split_h = self.should_split_horizontal(i - 1, &last_rect);

            let (first, second) = if split_h {
                last_rect.split_horizontal(self.split_ratio)
            } else {
                last_rect.split_vertical(self.split_ratio)
            };

            if self.grow_forward {
                // Previous window keeps first part, new window gets second
                rects[i - 1] = first;
                rects.push(second);
            } else {
                // New window gets first part, previous keeps second
                rects.push(first);
                rects[i - 1] = second;
            }
        }

        // Assign rectangles to containers
        for (i, container) in containers.iter_mut().enumerate() {
            container.set_rect(rects[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dwindle_single() {
        let layout = DwindleLayout::new();
        let mut containers = vec![Container::new(0)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, work_area);
    }

    #[test]
    fn test_dwindle_two_windows() {
        let layout = DwindleLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // First split is horizontal (alternating, depth 0)
        assert_eq!(containers[0].rect.width, 500);
        assert_eq!(containers[1].rect.width, 500);
        assert_eq!(containers[0].rect.x, 0);
        assert_eq!(containers[1].rect.x, 500);
    }

    #[test]
    fn test_dwindle_three_windows() {
        let layout = DwindleLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Window 0: left half (unchanged)
        assert_eq!(containers[0].rect.width, 500);
        assert_eq!(containers[0].rect.height, 1000);

        // Window 1: top-right quarter
        assert_eq!(containers[1].rect.x, 500);
        assert_eq!(containers[1].rect.height, 500);

        // Window 2: bottom-right quarter
        assert_eq!(containers[2].rect.x, 500);
        assert_eq!(containers[2].rect.y, 500);
    }

    #[test]
    fn test_dwindle_horizontal_only() {
        let layout = DwindleLayout::with_direction(DwindleDirection::Horizontal);
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // All windows should have full height (only horizontal splits)
        for container in &containers {
            assert_eq!(container.rect.height, 1000);
        }
    }

    #[test]
    fn test_dwindle_vertical_only() {
        let layout = DwindleLayout::with_direction(DwindleDirection::Vertical);
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // All windows should have full width (only vertical splits)
        for container in &containers {
            assert_eq!(container.rect.width, 1000);
        }
    }

    #[test]
    fn test_dwindle_custom_ratio() {
        let layout = DwindleLayout::with_ratio(0.6);
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // First window gets 60%, second gets 40%
        assert_eq!(containers[0].rect.width, 600);
        assert_eq!(containers[1].rect.width, 400);
    }

    #[test]
    fn preserved_history_overrides_current_policy() {
        let layout = DwindleLayout::configured(
            DwindleDirection::Horizontal,
            0.5,
            true,
            vec![DwindleDirection::Vertical, DwindleDirection::Horizontal],
        );
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        layout.calculate(
            Rect::new(0, 0, 1000, 1000),
            &mut containers,
            Padding::default(),
        );

        assert_eq!(containers[0].rect, Rect::new(0, 0, 1000, 500));
        assert_eq!(containers[1].rect, Rect::new(0, 500, 500, 500));
        assert_eq!(containers[2].rect, Rect::new(500, 500, 500, 500));
    }

    #[test]
    fn direction_names_round_trip() {
        for direction in [
            DwindleDirection::Alternate,
            DwindleDirection::Horizontal,
            DwindleDirection::Vertical,
        ] {
            assert_eq!(
                direction.config_name().parse::<DwindleDirection>().unwrap(),
                direction
            );
        }
        assert!("diagonal".parse::<DwindleDirection>().is_err());
    }
}
