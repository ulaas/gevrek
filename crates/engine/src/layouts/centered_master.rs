use super::Layout;
use crate::container::{Container, Padding, Rect};

/// Centered Master Layout
///
/// Places the master window(s) in the center with stacks on both sides.
/// Ideal for ultrawide monitors where you want the main content centered.
///
/// Example with 5 windows:
/// ```text
/// ┌────┬──────────┬────┐
/// │ L1 │          │ R1 │
/// ├────┤  Master  ├────┤
/// │ L2 │          │ R2 │
/// └────┴──────────┴────┘
/// ```
pub struct CenteredMasterLayout {
    /// Ratio of the center/master area (0.3 to 0.8)
    pub center_ratio: f32,
    /// Number of windows in the master/center area
    pub master_count: usize,
}

impl CenteredMasterLayout {
    pub fn new() -> Self {
        Self {
            center_ratio: 0.5,
            master_count: 1,
        }
    }

    pub fn with_ratio(ratio: f32) -> Self {
        Self {
            center_ratio: ratio.clamp(0.3, 0.8),
            master_count: 1,
        }
    }

    pub fn with_master_count(count: usize) -> Self {
        Self {
            center_ratio: 0.5,
            master_count: count.max(1),
        }
    }

    pub fn configured(ratio: f32, count: usize) -> Self {
        Self {
            center_ratio: ratio.clamp(0.3, 0.8),
            master_count: count.max(1),
        }
    }

    /// Adjust center ratio
    pub fn adjust_ratio(&mut self, delta: f32) {
        self.center_ratio = (self.center_ratio + delta).clamp(0.3, 0.8);
    }
}

impl Default for CenteredMasterLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for CenteredMasterLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        let work_area = work_area.with_padding(padding);
        let n = containers.len();

        if n == 0 {
            return;
        }

        // Single window takes full space
        if n == 1 {
            containers[0].set_rect(work_area);
            return;
        }

        // Two windows: split horizontally
        if n == 2 {
            let (left, right) = work_area.split_horizontal(0.5);
            containers[0].set_rect(left);
            containers[1].set_rect(right);
            return;
        }

        // Calculate regions
        let _side_ratio = (1.0 - self.center_ratio) / 2.0;
        let center_width = (work_area.width as f32 * self.center_ratio) as i32;
        let side_width = (work_area.width - center_width) / 2;

        let left_rect = Rect::new(work_area.x, work_area.y, side_width, work_area.height);
        let center_rect = Rect::new(
            work_area.x + side_width,
            work_area.y,
            center_width,
            work_area.height,
        );
        let right_rect = Rect::new(
            work_area.x + side_width + center_width,
            work_area.y,
            work_area.width - side_width - center_width,
            work_area.height,
        );

        // Determine window distribution
        let master_n = self.master_count.min(n);
        let stack_n = n - master_n;

        // Distribute stack windows between left and right
        let left_n = stack_n / 2;
        let right_n = stack_n - left_n;

        // Layout master windows in center
        for (i, container) in containers[..master_n].iter_mut().enumerate() {
            if master_n == 1 {
                container.set_rect(center_rect);
            } else {
                let height = center_rect.height / master_n as i32;
                let y = center_rect.y + (i as i32 * height);
                let h = if i == master_n - 1 {
                    center_rect.height - (i as i32 * height)
                } else {
                    height
                };
                container.set_rect(Rect::new(center_rect.x, y, center_rect.width, h));
            }
        }

        // Layout left stack
        if left_n > 0 {
            let height = left_rect.height / left_n as i32;
            for (i, container) in containers[master_n..master_n + left_n]
                .iter_mut()
                .enumerate()
            {
                let y = left_rect.y + (i as i32 * height);
                let h = if i == left_n - 1 {
                    left_rect.height - (i as i32 * height)
                } else {
                    height
                };
                container.set_rect(Rect::new(left_rect.x, y, left_rect.width, h));
            }
        }

        // Layout right stack
        if right_n > 0 {
            let height = right_rect.height / right_n as i32;
            for (i, container) in containers[master_n + left_n..].iter_mut().enumerate() {
                let y = right_rect.y + (i as i32 * height);
                let h = if i == right_n - 1 {
                    right_rect.height - (i as i32 * height)
                } else {
                    height
                };
                container.set_rect(Rect::new(right_rect.x, y, right_rect.width, h));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_centered_master_single() {
        let layout = CenteredMasterLayout::new();
        let mut containers = vec![Container::new(0)];
        let work_area = Rect::new(0, 0, 1920, 1080);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, work_area);
    }

    #[test]
    fn test_centered_master_three_windows() {
        let layout = CenteredMasterLayout::with_ratio(0.5);
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Master in center
        assert!(containers[0].rect.x > 0);
        assert!(containers[0].rect.x < 500);

        // Left and right stacks
        assert_eq!(containers[1].rect.x, 0);
        assert!(containers[2].rect.x > 500);
    }

    #[test]
    fn test_centered_master_five_windows() {
        let layout = CenteredMasterLayout::with_ratio(0.5);
        let mut containers = vec![
            Container::new(0),
            Container::new(1),
            Container::new(2),
            Container::new(3),
            Container::new(4),
        ];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Master in center
        let center_x = containers[0].rect.x;
        assert!(center_x > 200 && center_x < 300);

        // Two windows on left stack
        assert_eq!(containers[1].rect.x, 0);
        assert_eq!(containers[2].rect.x, 0);
        assert!(containers[2].rect.y > containers[1].rect.y);

        // Two windows on right stack
        assert!(containers[3].rect.x > 700);
        assert!(containers[4].rect.x > 700);
    }
}
