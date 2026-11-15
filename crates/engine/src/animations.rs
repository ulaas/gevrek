use super::container::Rect;

/// Effects available to explicit `Animate Window` automation nodes.
///
/// Execution is handled by `RuntimeAnimationScheduler`; this enum only names
/// the requested visual effect. Every variant maps to a distinct rect or
/// opacity tween computed by [`start_rect`]. Effects that would require a
/// compositor (flip, rotate, zoom-blur, ...) are deliberately not offered:
/// `SetWindowPos` cannot transform window content, so naming them would be
/// false advertising.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationType {
    /// Opacity tween from transparent to the window's configured alpha.
    Fade,
    /// Rect slide in from the left.
    Slide,
    /// Rect slide in from the left.
    SlideLeft,
    /// Rect slide in from the right.
    SlideRight,
    /// Rect slide in from above.
    SlideUp,
    /// Rect slide in from below.
    SlideDown,
    /// Grow from 80% of the target size, centered on the target.
    Scale,
    /// Grow downward from a thin strip anchored at the target's top edge.
    Expand,
    /// Shrink from 125% of the target size, centered on the target.
    Contract,
}

/// Compute the rect a rect-tween style starts from so the scheduler can
/// interpolate it into `target`.
///
/// Returns `None` for [`AnimationType::Fade`], which tweens opacity instead
/// of geometry and therefore has no start rect.
pub fn start_rect(animation_type: AnimationType, target: Rect) -> Option<Rect> {
    match animation_type {
        AnimationType::Fade => None,
        AnimationType::Slide | AnimationType::SlideLeft => {
            Some(offset_rect(target, -target.width / 3, 0))
        }
        AnimationType::SlideRight => Some(offset_rect(target, target.width / 3, 0)),
        AnimationType::SlideUp => Some(offset_rect(target, 0, -target.height / 3)),
        AnimationType::SlideDown => Some(offset_rect(target, 0, target.height / 3)),
        AnimationType::Scale => Some(centered_scaled_rect(target, 80)),
        AnimationType::Expand => Some(Rect::new(
            target.x,
            target.y,
            target.width,
            (target.height / 5).max(1),
        )),
        AnimationType::Contract => Some(centered_scaled_rect(target, 125)),
    }
}

/// `rect` translated by (`dx`, `dy`) with its size unchanged.
pub fn offset_rect(rect: Rect, dx: i32, dy: i32) -> Rect {
    Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height)
}

/// `target` scaled to `percent` of its size, keeping the same center.
pub fn centered_scaled_rect(target: Rect, percent: u32) -> Rect {
    let width = ((target.width as i64 * percent as i64 / 100) as i32).max(1);
    let height = ((target.height as i64 * percent as i64 / 100) as i32).max(1);
    Rect::new(
        target.x + (target.width - width) / 2,
        target.y + (target.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation_scheduler::RuntimeAnimationScheduler;
    use crate::WindowHandle;
    use gevrek_model::settings::{AnimationChannelProfile, AnimationCurve, AnimationProfile};
    use std::time::{Duration, Instant};

    fn target() -> Rect {
        Rect::new(100, 50, 300, 200)
    }

    fn center(rect: Rect) -> (i32, i32) {
        (rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    fn linear_profile() -> AnimationProfile {
        AnimationProfile {
            window: AnimationChannelProfile::new(true, 100, AnimationCurve::Linear),
            ..AnimationProfile::default()
        }
    }

    #[test]
    fn fade_tweens_opacity_so_it_has_no_start_rect() {
        assert_eq!(start_rect(AnimationType::Fade, target()), None);
    }

    #[test]
    fn slide_variants_offset_along_one_axis_and_keep_size() {
        let target = target();
        let cases = [
            (AnimationType::Slide, -target.width / 3, 0),
            (AnimationType::SlideLeft, -target.width / 3, 0),
            (AnimationType::SlideRight, target.width / 3, 0),
            (AnimationType::SlideUp, 0, -target.height / 3),
            (AnimationType::SlideDown, 0, target.height / 3),
        ];
        for (style, dx, dy) in cases {
            let start = start_rect(style, target).expect("rect style has a start rect");
            assert_eq!(
                start,
                Rect::new(target.x + dx, target.y + dy, target.width, target.height),
                "{style:?} must slide along its axis without resizing",
            );
            assert_ne!(start, target, "{style:?} must actually move");
        }
    }

    #[test]
    fn scale_starts_smaller_and_centered_on_the_target() {
        let target = target();
        let start = start_rect(AnimationType::Scale, target).unwrap();
        assert_eq!(start, Rect::new(130, 70, 240, 160));
        assert!(start.width < target.width && start.height < target.height);
        assert_eq!(center(start), center(target));
    }

    #[test]
    fn contract_starts_larger_and_centered_on_the_target() {
        let target = target();
        let start = start_rect(AnimationType::Contract, target).unwrap();
        assert_eq!(start, Rect::new(63, 25, 375, 250));
        assert!(start.width > target.width && start.height > target.height);
        assert_eq!(center(start), center(target));
    }

    #[test]
    fn expand_grows_downward_from_the_top_edge() {
        let target = target();
        let start = start_rect(AnimationType::Expand, target).unwrap();
        assert_eq!(start, Rect::new(target.x, target.y, target.width, 40));
        assert!(start.height < target.height);
    }

    #[test]
    fn every_rect_style_has_a_distinct_start_rect() {
        let target = target();
        let styles = [
            AnimationType::Slide,
            AnimationType::SlideRight,
            AnimationType::SlideUp,
            AnimationType::SlideDown,
            AnimationType::Scale,
            AnimationType::Expand,
            AnimationType::Contract,
        ];
        for (index, style) in styles.iter().enumerate() {
            for other in &styles[index + 1..] {
                assert_ne!(
                    start_rect(*style, target),
                    start_rect(*other, target),
                    "{style:?} and {other:?} must not collapse to the same tween",
                );
            }
        }
    }

    #[test]
    fn tiny_targets_never_produce_zero_sized_start_rects() {
        let tiny = Rect::new(0, 0, 1, 1);
        for style in [
            AnimationType::Scale,
            AnimationType::Expand,
            AnimationType::Contract,
        ] {
            let start = start_rect(style, tiny).unwrap();
            assert!(start.width >= 1 && start.height >= 1, "{style:?}");
        }
    }

    #[test]
    fn scheduled_styles_pass_through_the_midpoint_and_land_on_target() {
        let target = target();
        let expected_midpoints = [
            (AnimationType::Slide, Rect::new(50, 50, 300, 200)),
            (AnimationType::SlideUp, Rect::new(100, 17, 300, 200)),
            (AnimationType::Scale, Rect::new(115, 60, 270, 180)),
            (AnimationType::Expand, Rect::new(100, 50, 300, 120)),
            (AnimationType::Contract, Rect::new(82, 38, 338, 225)),
        ];
        for (style, midpoint) in expected_midpoints {
            let started = Instant::now();
            let mut scheduler = RuntimeAnimationScheduler::new(linear_profile());
            let handle = WindowHandle(21);
            let start = start_rect(style, target).unwrap();
            assert!(scheduler.schedule_explicit_rect(handle, start, target, 0, started));

            let half = scheduler.tick(started + Duration::from_millis(50));
            assert_eq!(half.rectangles, vec![(handle, midpoint)], "{style:?}");

            let done = scheduler.tick(started + Duration::from_millis(100));
            assert_eq!(done.rectangles, vec![(handle, target)], "{style:?}");
        }
    }
}
