use super::Layout;
use crate::container::{Container, Padding, Rect};
use serde::{Deserialize, Serialize};

/// Direction of the spiral
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpiralDirection {
    #[default]
    Clockwise,
    CounterClockwise,
}

/// Starting corner of the spiral
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpiralStart {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Spiral Layout (Fibonacci-style)
///
/// Creates a Fibonacci spiral pattern where each new window takes a
/// golden-ratio proportion of the remaining space.
///
/// Example (clockwise from top-left):
/// ```text
/// ┌──────────────┬─────────┐
/// │              │    2    │
/// │      1       ├────┬────┤
/// │              │ 4  │ 3  │
/// │              ├────┼────┤
/// │              │  5 │ 6  │
/// └──────────────┴────┴────┘
/// ```
pub struct SpiralLayout {
    /// Direction of the spiral
    pub direction: SpiralDirection,
    /// Starting corner
    pub start: SpiralStart,
    /// Use golden ratio (true) or equal split (false)
    pub use_golden_ratio: bool,
}

impl SpiralLayout {
    /// Golden ratio constant (~0.618)
    const PHI: f32 = 0.618_034;

    pub fn new() -> Self {
        Self {
            direction: SpiralDirection::Clockwise,
            start: SpiralStart::TopLeft,
            use_golden_ratio: true,
        }
    }

    pub fn clockwise() -> Self {
        Self {
            direction: SpiralDirection::Clockwise,
            ..Self::new()
        }
    }

    pub fn counter_clockwise() -> Self {
        Self {
            direction: SpiralDirection::CounterClockwise,
            ..Self::new()
        }
    }

    pub fn with_start(start: SpiralStart) -> Self {
        Self {
            start,
            ..Self::new()
        }
    }

    pub fn configured(direction: SpiralDirection, start: SpiralStart, golden: bool) -> Self {
        Self {
            direction,
            start,
            use_golden_ratio: golden,
        }
    }

    fn get_ratio(&self) -> f32 {
        if self.use_golden_ratio {
            Self::PHI
        } else {
            0.5
        }
    }

    /// Get the split pattern based on direction and iteration
    /// Returns (is_horizontal, take_first) where:
    /// - is_horizontal: true for left/right split, false for top/bottom
    /// - take_first: which part the new window takes (true = first/left/top)
    fn get_split_pattern(&self, iteration: usize) -> (bool, bool) {
        let phase = iteration % 4;

        match self.direction {
            SpiralDirection::Clockwise => match self.start {
                SpiralStart::TopLeft => match phase {
                    0 => (true, false),  // right
                    1 => (false, false), // bottom
                    2 => (true, true),   // left
                    3 => (false, true),  // top
                    _ => unreachable!(),
                },
                SpiralStart::TopRight => match phase {
                    0 => (false, false), // bottom
                    1 => (true, true),   // left
                    2 => (false, true),  // top
                    3 => (true, false),  // right
                    _ => unreachable!(),
                },
                SpiralStart::BottomRight => match phase {
                    0 => (true, true),   // left
                    1 => (false, true),  // top
                    2 => (true, false),  // right
                    3 => (false, false), // bottom
                    _ => unreachable!(),
                },
                SpiralStart::BottomLeft => match phase {
                    0 => (false, true),  // top
                    1 => (true, false),  // right
                    2 => (false, false), // bottom
                    3 => (true, true),   // left
                    _ => unreachable!(),
                },
            },
            SpiralDirection::CounterClockwise => match self.start {
                SpiralStart::TopLeft => match phase {
                    0 => (false, false), // bottom
                    1 => (true, false),  // right
                    2 => (false, true),  // top
                    3 => (true, true),   // left
                    _ => unreachable!(),
                },
                SpiralStart::TopRight => match phase {
                    0 => (true, true),   // left
                    1 => (false, false), // bottom
                    2 => (true, false),  // right
                    3 => (false, true),  // top
                    _ => unreachable!(),
                },
                SpiralStart::BottomRight => match phase {
                    0 => (false, true),  // top
                    1 => (true, true),   // left
                    2 => (false, false), // bottom
                    3 => (true, false),  // right
                    _ => unreachable!(),
                },
                SpiralStart::BottomLeft => match phase {
                    0 => (true, false),  // right
                    1 => (false, true),  // top
                    2 => (true, true),   // left
                    3 => (false, false), // bottom
                    _ => unreachable!(),
                },
            },
        }
    }
}

impl Default for SpiralLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for SpiralLayout {
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

        let ratio = self.get_ratio();
        let mut rects = vec![work_area; n];
        rects[0] = work_area;

        // Build spiral iteratively
        for i in 1..n {
            let prev_rect = rects[i - 1];
            let (is_horizontal, take_first) = self.get_split_pattern(i - 1);

            let (part1, part2) = if is_horizontal {
                prev_rect.split_horizontal(ratio)
            } else {
                prev_rect.split_vertical(ratio)
            };
            // A room that can be shared no further without a tile too
            // small is shared equally by this window and all after it.
            let (kept, rest) = if take_first {
                (&part2, &part1)
            } else {
                (&part1, &part2)
            };
            if super::shared_no_further(kept, rest, is_horizontal, n - i) {
                let parts = super::in_equal_parts(prev_rect, n - (i - 1));
                rects[i - 1..].copy_from_slice(&parts);
                break;
            }

            if take_first {
                // New window takes first part, previous keeps second
                rects[i] = part1;
                rects[i - 1] = part2;
            } else {
                // Previous keeps first part, new window takes second
                rects[i - 1] = part1;
                rects[i] = part2;
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
    fn test_spiral_single() {
        let layout = SpiralLayout::new();
        let mut containers = vec![Container::new(0)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, work_area);
    }

    #[test]
    fn test_spiral_two_windows() {
        let layout = SpiralLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // First window should be larger (golden ratio)
        assert!(containers[0].rect.width > containers[1].rect.width);
    }

    #[test]
    fn test_spiral_equal_split() {
        let layout = SpiralLayout::configured(
            SpiralDirection::Clockwise,
            SpiralStart::TopLeft,
            false, // equal split
        );
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Both windows should be roughly equal
        let diff = (containers[0].rect.width - containers[1].rect.width).abs();
        assert!(diff <= 1); // Allow for rounding
    }

    #[test]
    fn test_spiral_four_windows() {
        let layout = SpiralLayout::new();
        let mut containers = vec![
            Container::new(0),
            Container::new(1),
            Container::new(2),
            Container::new(3),
        ];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // All containers should have positive dimensions
        for container in &containers {
            assert!(container.rect.width > 0);
            assert!(container.rect.height > 0);
        }

        // First container should be largest
        let first_area = containers[0].rect.width * containers[0].rect.height;
        let last_area = containers[3].rect.width * containers[3].rect.height;
        assert!(first_area > last_area);
    }
}
