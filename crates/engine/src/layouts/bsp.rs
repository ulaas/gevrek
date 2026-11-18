use super::{EdgeDirection, Layout};
use crate::container::{Container, Padding, Rect};

/// Default share of the pre-split region kept by the existing container when
/// a new container is created.
pub const BSP_DEFAULT_SPLIT_RATIO: f32 = 0.5;

/// Binary Space Partitioning layout (Spiral/Dwindle style)
/// Each new window splits the previous window's space:
/// - 1 window: full space
/// - 2 windows: horizontal split (left/right)
/// - 3 windows: vertical split of window 2 (top/bottom)
/// - 4 windows: horizontal split of window 3 (left/right)
/// - etc., alternating horizontal/vertical
///
/// The splits form a chain: split `j` (1-based) divides the region that
/// container `j - 1` was about to occupy, keeping `ratios[j - 1]` of it for
/// container `j - 1` and giving the rest to the new container `j`. Odd splits
/// are horizontal (left/right), even splits vertical (top/bottom).
pub struct BspLayout;

impl BspLayout {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BspLayout {
    fn default() -> Self {
        Self::new()
    }
}

/// The ratio stored for split `j` (1-based), defaulting to an even split.
pub fn bsp_split_ratio(ratios: &[f32], split: usize) -> f32 {
    ratios
        .get(split.saturating_sub(1))
        .copied()
        .unwrap_or(BSP_DEFAULT_SPLIT_RATIO)
}

/// Split `j` (1-based) is horizontal when odd, vertical when even.
pub fn bsp_split_is_horizontal(split: usize) -> bool {
    split % 2 == 1
}

/// The pre-split regions of the BSP chain: element 0 is the work area,
/// element `i` is the region that is split to create container `i`.
pub fn bsp_chain_rects(work_area: Rect, count: usize, ratios: &[f32]) -> Vec<Rect> {
    let mut chain = Vec::with_capacity(count);
    if count == 0 {
        return chain;
    }
    chain.push(work_area);
    for split in 1..count {
        let previous = chain[split - 1];
        let ratio = bsp_split_ratio(ratios, split);
        let region = if bsp_split_is_horizontal(split) {
            previous.split_horizontal(ratio).1
        } else {
            previous.split_vertical(ratio).1
        };
        chain.push(region);
    }
    chain
}

/// The split (1-based) controlling `container`'s edge in `direction`, or
/// `None` when that edge sits on the workspace boundary and cannot move.
///
/// Container `k` lies in the second part of every split `1..=k` and in the
/// first part of split `k + 1` (when it exists). In this chain the right and
/// bottom edges of the newest containers coincide with the work-area edges,
/// so only the splits below can move each edge:
/// - Right/Down: split `k + 1`, when it exists and has the matching
///   orientation.
/// - Left/Up: the nearest split `<= k` with the matching orientation.
pub fn bsp_edge_split(count: usize, container: usize, direction: EdgeDirection) -> Option<usize> {
    if container >= count {
        return None;
    }
    match direction {
        EdgeDirection::Right => {
            let split = container + 1;
            (split < count && bsp_split_is_horizontal(split)).then_some(split)
        }
        EdgeDirection::Down => {
            let split = container + 1;
            (split < count && !bsp_split_is_horizontal(split)).then_some(split)
        }
        EdgeDirection::Left => {
            if container == 0 {
                None
            } else if bsp_split_is_horizontal(container) {
                Some(container)
            } else {
                Some(container - 1)
            }
        }
        EdgeDirection::Up => {
            if container < 2 {
                None
            } else if bsp_split_is_horizontal(container) {
                Some(container - 1)
            } else {
                Some(container)
            }
        }
    }
}

/// Compute the new ratio for the split controlling `container`'s edge in
/// `direction` after moving that edge by `delta_px` pixels (positive grows
/// the container). Returns `(split, new_ratio)` clamped to 0.1..=0.9, or
/// `None` when the edge cannot move (workspace boundary or degenerate area).
pub fn bsp_resized_ratio(
    work_area: Rect,
    count: usize,
    container: usize,
    ratios: &[f32],
    direction: EdgeDirection,
    delta_px: i32,
) -> Option<(usize, f32)> {
    let split = bsp_edge_split(count, container, direction)?;
    let chain = bsp_chain_rects(work_area, count, ratios);
    let parent = chain[split - 1];
    let extent = match direction {
        EdgeDirection::Left | EdgeDirection::Right => parent.width,
        EdgeDirection::Up | EdgeDirection::Down => parent.height,
    };
    if extent <= 0 {
        return None;
    }
    // Container `k` keeps the first part of split `k + 1` (Right/Down grow it
    // by increasing the ratio) and the second part of splits `1..=k`
    // (Left/Up grow it by decreasing the ratio of the shared boundary).
    let sign = match direction {
        EdgeDirection::Right | EdgeDirection::Down => 1.0,
        EdgeDirection::Left | EdgeDirection::Up => -1.0,
    };
    let ratio = bsp_split_ratio(ratios, split);
    let new_ratio = ratio + sign * delta_px as f32 / extent as f32;
    Some((split, new_ratio.clamp(0.1, 0.9)))
}

impl Layout for BspLayout {
    fn calculate(&self, work_area: Rect, containers: &mut [Container], padding: Padding) {
        self.calculate_with_ratios(work_area, containers, padding, &[]);
    }

    fn calculate_with_ratios(
        &self,
        work_area: Rect,
        containers: &mut [Container],
        padding: Padding,
        ratios: &[f32],
    ) {
        let work_area = work_area.with_padding(padding);
        let n = containers.len();

        if n == 0 {
            return;
        }

        // Base case: single container takes full space
        if n == 1 {
            containers[0].set_rect(work_area);
            log::trace!(
                "BSP: Container {} assigned rect ({}, {}) {}x{}",
                containers[0].id,
                work_area.x,
                work_area.y,
                work_area.width,
                work_area.height
            );
            return;
        }

        // Track rectangles for each window as we build the chain
        let mut rects = vec![work_area; n];

        // For each subsequent window, split the previous window's space
        for i in 1..n {
            let prev_rect = rects[i - 1];
            let ratio = bsp_split_ratio(ratios, i);
            // A room that can be shared no further without a tile too
            // small is shared equally by this window and all after it.
            let shared = if bsp_split_is_horizontal(i) {
                prev_rect.split_horizontal(ratio)
            } else {
                prev_rect.split_vertical(ratio)
            };
            if super::shared_no_further(&shared.0, &shared.1, bsp_split_is_horizontal(i), n - i) {
                let parts = super::in_equal_parts(prev_rect, n - (i - 1));
                rects[i - 1..].copy_from_slice(&parts);
                break;
            }

            if bsp_split_is_horizontal(i) {
                // Horizontal split (left/right)
                let (left, right) = prev_rect.split_horizontal(ratio);
                rects[i - 1] = left; // Previous window keeps the left share
                rects[i] = right; // New window takes the rest
                log::trace!(
                    "BSP: Window {} horizontal split at {:.2} - prev keeps left, new gets right",
                    i,
                    ratio
                );
            } else {
                // Vertical split (top/bottom)
                let (top, bottom) = prev_rect.split_vertical(ratio);
                rects[i - 1] = top; // Previous window keeps the top share
                rects[i] = bottom; // New window takes the rest
                log::trace!(
                    "BSP: Window {} vertical split at {:.2} - prev keeps top, new gets bottom",
                    i,
                    ratio
                );
            }
        }

        // Assign calculated rectangles to containers
        for (i, container) in containers.iter_mut().enumerate() {
            container.set_rect(rects[i]);
            log::trace!(
                "BSP: Container {} assigned rect ({}, {}) {}x{}",
                container.id,
                rects[i].x,
                rects[i].y,
                rects[i].width,
                rects[i].height
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bsp_single_container() {
        let layout = BspLayout::new();
        let mut containers = vec![Container::new(0)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        assert_eq!(containers[0].rect, work_area);
    }

    #[test]
    fn test_bsp_two_containers() {
        let layout = BspLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Should split horizontally - first window left half, second window right half
        assert_eq!(containers[0].rect.x, 0);
        assert_eq!(containers[0].rect.width, 500);
        assert_eq!(containers[1].rect.x, 500);
        assert_eq!(containers[1].rect.width, 500);
    }

    #[test]
    fn test_bsp_three_containers() {
        let layout = BspLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // First window: left half (500x1000)
        assert_eq!(containers[0].rect.x, 0);
        assert_eq!(containers[0].rect.width, 500);
        assert_eq!(containers[0].rect.height, 1000);

        // Second window: top right quarter (500x500)
        assert_eq!(containers[1].rect.x, 500);
        assert_eq!(containers[1].rect.y, 0);
        assert_eq!(containers[1].rect.width, 500);
        assert_eq!(containers[1].rect.height, 500);

        // Third window: bottom right quarter (500x500)
        assert_eq!(containers[2].rect.x, 500);
        assert_eq!(containers[2].rect.y, 500);
        assert_eq!(containers[2].rect.width, 500);
        assert_eq!(containers[2].rect.height, 500);
    }

    #[test]
    fn test_bsp_four_containers() {
        let layout = BspLayout::new();
        let mut containers = vec![
            Container::new(0),
            Container::new(1),
            Container::new(2),
            Container::new(3),
        ];
        let work_area = Rect::new(0, 0, 1000, 1000);

        layout.calculate(work_area, &mut containers, Padding::default());

        // Verify all containers have been assigned rectangles
        for container in &containers {
            assert!(container.rect.width > 0);
            assert!(container.rect.height > 0);
        }

        // Window 0: left half
        assert_eq!(containers[0].rect.width, 500);
        assert_eq!(containers[0].rect.height, 1000);

        // Window 1: top right quarter
        assert_eq!(containers[1].rect.x, 500);
        assert_eq!(containers[1].rect.y, 0);
        assert_eq!(containers[1].rect.height, 500);

        // Window 2: bottom right, left quarter (250x500)
        assert_eq!(containers[2].rect.x, 500);
        assert_eq!(containers[2].rect.y, 500);
        assert_eq!(containers[2].rect.width, 250);

        // Window 3: bottom right, right quarter (250x500)
        assert_eq!(containers[3].rect.x, 750);
        assert_eq!(containers[3].rect.y, 500);
        assert_eq!(containers[3].rect.width, 250);
    }

    #[test]
    fn bsp_ratios_change_split_positions() {
        let layout = BspLayout::new();
        let mut containers = vec![Container::new(0), Container::new(1), Container::new(2)];
        let work_area = Rect::new(0, 0, 1000, 1000);

        // Split 1 keeps 70% for container 0; split 2 keeps 25% for container 1.
        layout.calculate_with_ratios(work_area, &mut containers, Padding::default(), &[0.7, 0.25]);

        assert_eq!(containers[0].rect.width, 700);
        assert_eq!(containers[1].rect.x, 700);
        assert_eq!(containers[1].rect.height, 250);
        assert_eq!(containers[2].rect.y, 250);
        assert_eq!(containers[2].rect.height, 750);
    }

    #[test]
    fn bsp_chain_rects_tracks_pre_split_regions() {
        let chain = bsp_chain_rects(Rect::new(0, 0, 1000, 1000), 4, &[0.5, 0.5, 0.5]);
        assert_eq!(chain[0], Rect::new(0, 0, 1000, 1000));
        assert_eq!(chain[1], Rect::new(500, 0, 500, 1000));
        assert_eq!(chain[2], Rect::new(500, 500, 500, 500));
        assert_eq!(chain[3], Rect::new(750, 500, 250, 500));
    }

    #[test]
    fn bsp_edge_split_finds_the_controlling_split() {
        // Container 0: only its right edge (split 1) can move.
        assert_eq!(bsp_edge_split(4, 0, EdgeDirection::Right), Some(1));
        assert_eq!(bsp_edge_split(4, 0, EdgeDirection::Left), None);
        // Container 0's bottom edge is the work-area edge (split 1 is horizontal).
        assert_eq!(bsp_edge_split(4, 0, EdgeDirection::Down), None);
        assert_eq!(bsp_edge_split(4, 0, EdgeDirection::Up), None);

        // Container 1: right edge is the work-area edge (split 2 is vertical),
        // down edge is split 2, left edge is split 1, top is the work-area edge.
        assert_eq!(bsp_edge_split(4, 1, EdgeDirection::Right), None);
        assert_eq!(bsp_edge_split(4, 1, EdgeDirection::Down), Some(2));
        assert_eq!(bsp_edge_split(4, 1, EdgeDirection::Left), Some(1));
        assert_eq!(bsp_edge_split(4, 1, EdgeDirection::Up), None);

        // Container 2: right edge is split 3, left edge is split 1's boundary
        // (nearest horizontal split on its path), up edge is split 2, down is
        // the work-area edge.
        assert_eq!(bsp_edge_split(4, 2, EdgeDirection::Right), Some(3));
        assert_eq!(bsp_edge_split(4, 2, EdgeDirection::Left), Some(1));
        assert_eq!(bsp_edge_split(4, 2, EdgeDirection::Up), Some(2));
        assert_eq!(bsp_edge_split(4, 2, EdgeDirection::Down), None);

        // Newest container: no forward split exists.
        assert_eq!(bsp_edge_split(4, 3, EdgeDirection::Right), None);
        assert_eq!(bsp_edge_split(4, 3, EdgeDirection::Left), Some(3));
        assert_eq!(bsp_edge_split(4, 3, EdgeDirection::Up), Some(2));
        assert_eq!(bsp_edge_split(4, 3, EdgeDirection::Down), None);

        // Out-of-range container.
        assert_eq!(bsp_edge_split(4, 4, EdgeDirection::Right), None);
    }

    #[test]
    fn bsp_resized_ratio_grows_and_shrinks_the_shared_edge() {
        let work_area = Rect::new(0, 0, 1000, 1000);
        let ratios = [0.5, 0.5, 0.5];

        // Grow container 0's right edge by 100px: split 1 keeps more.
        let (split, ratio) =
            bsp_resized_ratio(work_area, 4, 0, &ratios, EdgeDirection::Right, 100).unwrap();
        assert_eq!(split, 1);
        assert!((ratio - 0.6).abs() < 1e-6);

        // Shrink it by 100px: split 1 keeps less.
        let (_, ratio) =
            bsp_resized_ratio(work_area, 4, 0, &ratios, EdgeDirection::Right, -100).unwrap();
        assert!((ratio - 0.4).abs() < 1e-6);

        // Grow container 1's left edge by 100px: the split-1 boundary moves
        // left, so the ratio container 0 keeps decreases.
        let (split, ratio) =
            bsp_resized_ratio(work_area, 4, 1, &ratios, EdgeDirection::Left, 100).unwrap();
        assert_eq!(split, 1);
        assert!((ratio - 0.4).abs() < 1e-6);

        // Grow container 1's down edge by 100px of its 1000px parent: split 2
        // keeps more height.
        let (split, ratio) =
            bsp_resized_ratio(work_area, 4, 1, &ratios, EdgeDirection::Down, 100).unwrap();
        assert_eq!(split, 2);
        assert!((ratio - 0.6).abs() < 1e-6);

        // Grow container 3's up edge by 100px: split 2 divides its 1000px
        // tall parent, so its boundary moves up and the ratio decreases.
        let (split, ratio) =
            bsp_resized_ratio(work_area, 4, 3, &ratios, EdgeDirection::Up, 100).unwrap();
        assert_eq!(split, 2);
        assert!((ratio - 0.4).abs() < 1e-6);

        // Boundary edges cannot move.
        assert!(bsp_resized_ratio(work_area, 4, 3, &ratios, EdgeDirection::Right, 100).is_none());
        assert!(bsp_resized_ratio(work_area, 4, 0, &ratios, EdgeDirection::Left, 100).is_none());
    }

    #[test]
    fn bsp_resized_ratio_clamps_extremes() {
        let work_area = Rect::new(0, 0, 1000, 1000);
        let ratios = [0.5];

        let (_, ratio) =
            bsp_resized_ratio(work_area, 2, 0, &ratios, EdgeDirection::Right, 5000).unwrap();
        assert!((ratio - 0.9).abs() < 1e-6);
        let (_, ratio) =
            bsp_resized_ratio(work_area, 2, 0, &ratios, EdgeDirection::Right, -5000).unwrap();
        assert!((ratio - 0.1).abs() < 1e-6);
    }
}
