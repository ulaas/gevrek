use super::Layout;
use crate::container::{Container, Padding, Rect};
pub use gevrek_model::layout::{MasterPosition, NewWindowPlacement};

/// Master-Stack Layout
///
/// A classic tiling layout with a "master" area for the main window(s)
/// and a "stack" area for secondary windows.
///
/// Example with master on left (default):
/// ```text
/// ┌──────────┬──────┐
/// │          │  S1  │
/// │  Master  ├──────┤
/// │          │  S2  │
/// │          ├──────┤
/// │          │  S3  │
/// └──────────┴──────┘
/// ```
pub struct MasterStackLayout {
    /// Ratio of master area (0.1 to 0.9)
    pub master_ratio: f32,
    /// Number of windows in the master area
    pub master_count: usize,
    /// Position of the master area
    pub master_position: MasterPosition,
}

impl MasterStackLayout {
    pub fn new() -> Self {
        Self {
            master_ratio: 0.55,
            master_count: 1,
            master_position: MasterPosition::Left,
        }
    }

    pub fn with_ratio(ratio: f32) -> Self {
        Self {
            master_ratio: ratio.clamp(0.1, 0.9),
            ..Self::new()
        }
    }

    pub fn with_master_count(count: usize) -> Self {
        Self {
            master_count: count.max(1),
            ..Self::new()
        }
    }

    pub fn with_position(position: MasterPosition) -> Self {
        Self {
            master_position: position,
            ..Self::new()
        }
    }

    /// Full configuration
    pub fn configured(ratio: f32, count: usize, position: MasterPosition) -> Self {
        Self {
            master_ratio: ratio.clamp(0.1, 0.9),
            master_count: count.max(1),
            master_position: position,
        }
    }

    /// Adjust master ratio by delta (-0.05 to shrink, +0.05 to grow)
    pub fn adjust_ratio(&mut self, delta: f32) {
        self.master_ratio = (self.master_ratio + delta).clamp(0.1, 0.9);
    }

    /// Increment master count
    pub fn increment_master(&mut self) {
        self.master_count += 1;
    }

    /// Decrement master count (minimum 1)
    pub fn decrement_master(&mut self) {
        if self.master_count > 1 {
            self.master_count -= 1;
        }
    }
}

impl Default for MasterStackLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout for MasterStackLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        let work_area = work_area.with_padding(padding);
        let n = containers.len();

        if n == 0 {
            return;
        }

        // Single container takes full space
        if n == 1 {
            containers[0].set_rect(work_area);
            return;
        }

        // Determine how many windows go in master vs stack
        let master_n = self.master_count.min(n);
        let stack_n = n - master_n;

        // Split work area into master and stack regions
        let (master_rect, stack_rect) = match self.master_position {
            MasterPosition::Left => work_area.split_horizontal(self.master_ratio),
            MasterPosition::Right => {
                let (stack, master) = work_area.split_horizontal(1.0 - self.master_ratio);
                (master, stack)
            }
            MasterPosition::Top => work_area.split_vertical(self.master_ratio),
            MasterPosition::Bottom => {
                let (stack, master) = work_area.split_vertical(1.0 - self.master_ratio);
                (master, stack)
            }
        };

        // Layout master windows
        let is_horizontal_master = matches!(
            self.master_position,
            MasterPosition::Left | MasterPosition::Right
        );

        for (i, container) in containers[..master_n].iter_mut().enumerate() {
            if master_n == 1 {
                container.set_rect(master_rect);
            } else if is_horizontal_master {
                // Stack masters vertically within master area
                let height = master_rect.height / master_n as i32;
                let y = master_rect.y + (i as i32 * height);
                let h = if i == master_n - 1 {
                    master_rect.height - (i as i32 * height)
                } else {
                    height
                };
                container.set_rect(Rect::new(master_rect.x, y, master_rect.width, h));
            } else {
                // Stack masters horizontally within master area
                let width = master_rect.width / master_n as i32;
                let x = master_rect.x + (i as i32 * width);
                let w = if i == master_n - 1 {
                    master_rect.width - (i as i32 * width)
                } else {
                    width
                };
                container.set_rect(Rect::new(x, master_rect.y, w, master_rect.height));
            }
        }

        // Layout stack windows
        if stack_n > 0 {
            for (i, container) in containers[master_n..].iter_mut().enumerate() {
                if is_horizontal_master {
                    // Stack windows vertically
                    let height = stack_rect.height / stack_n as i32;
                    let y = stack_rect.y + (i as i32 * height);
                    let h = if i == stack_n - 1 {
                        stack_rect.height - (i as i32 * height)
                    } else {
                        height
                    };
                    container.set_rect(Rect::new(stack_rect.x, y, stack_rect.width, h));
                } else {
                    // Stack windows horizontally
                    let width = stack_rect.width / stack_n as i32;
                    let x = stack_rect.x + (i as i32 * width);
                    let w = if i == stack_n - 1 {
                        stack_rect.width - (i as i32 * width)
                    } else {
                        width
                    };
                    container.set_rect(Rect::new(x, stack_rect.y, w, stack_rect.height));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_master_stack_single() {
        let layout = MasterStackLayout::new();
        let mut containers = vec![Container::new(0)];
        let work_area = Rect::new(0, 0, 1920, 1080);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, work_area);
    }

    #[test]
    fn test_master_stack_two_windows() {
        let layout = MasterStackLayout::with_ratio(0.5);
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Master on left half
        assert_eq!(containers[0].rect.x, 0);
        assert_eq!(containers[0].rect.width, 500);

        // Stack on right half
        assert_eq!(containers[1].rect.x, 500);
        assert_eq!(containers[1].rect.width, 500);
    }

    #[test]
    fn test_master_stack_three_windows() {
        let layout = MasterStackLayout::with_ratio(0.5);
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Master takes left half
        assert_eq!(containers[0].rect.width, 500);
        assert_eq!(containers[0].rect.height, 1000);

        // Stack split vertically on right
        assert_eq!(containers[1].rect.x, 500);
        assert_eq!(containers[1].rect.height, 500);
        assert_eq!(containers[2].rect.x, 500);
        assert_eq!(containers[2].rect.y, 500);
    }

    #[test]
    fn test_master_stack_right_position() {
        let layout = MasterStackLayout::configured(0.5, 1, MasterPosition::Right);
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Master on right
        assert_eq!(containers[0].rect.x, 500);

        // Stack on left
        assert_eq!(containers[1].rect.x, 0);
    }

    #[test]
    fn test_master_stack_two_masters() {
        let layout = MasterStackLayout::configured(0.6, 2, MasterPosition::Left);
        let mut containers = vec![
            Container::new(0),
            Container::new(1),
            Container::new(2),
            Container::new(3),
        ];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Two masters stacked vertically on left
        assert_eq!(containers[0].rect.x, 0);
        assert_eq!(containers[0].rect.height, 500);
        assert_eq!(containers[1].rect.x, 0);
        assert_eq!(containers[1].rect.y, 500);

        // Two stack windows on right
        assert_eq!(containers[2].rect.x, 600);
        assert_eq!(containers[3].rect.x, 600);
    }
}
