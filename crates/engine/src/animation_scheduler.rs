use super::{Rect, WindowHandle};
use gevrek_model::settings::{AnimationChannelProfile, AnimationCurve, AnimationProfile};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionScope {
    Layout,
    Workspace,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderFrame {
    pub handle: WindowHandle,
    pub color: (u8, u8, u8),
    pub alpha: u8,
}

#[derive(Debug, Default)]
pub struct AnimationFrame {
    pub rectangles: Vec<(WindowHandle, Rect)>,
    pub borders: Vec<BorderFrame>,
    pub opacities: Vec<(WindowHandle, u8)>,
    pub close_windows: Vec<WindowHandle>,
}

#[derive(Debug, Clone)]
struct RectTransition {
    start: Rect,
    target: Rect,
    started_at: Instant,
    duration: Duration,
    curve: AnimationCurve,
    /// The speed the window already had when this movement began (x, y,
    /// width, height, in pixels a second): what a spring takes over from
    /// the movement it interrupts. Zero for a movement from rest and for
    /// every curve that is not a spring.
    velocity: [f32; 4],
}

impl RectTransition {
    /// Where the window is at `now`: x, y, width, height, not yet rounded.
    fn components(&self, now: Instant) -> [f32; 4] {
        let linear = progress(self.started_at, self.duration, now);
        let start = components_of(self.start);
        let target = components_of(self.target);
        let whole = self.duration.as_secs_f32();
        let mut at = [0.0; 4];
        for index in 0..4 {
            at[index] = match self.curve.spring_remaining(
                linear,
                start[index] - target[index],
                self.velocity[index] * whole,
            ) {
                Some(remaining) => target[index] + remaining,
                None => start[index] + (target[index] - start[index]) * self.curve.sample(linear),
            };
        }
        at
    }

    fn rect_at(&self, now: Instant) -> Rect {
        let [x, y, width, height] = self.components(now);
        Rect {
            x: x.round() as i32,
            y: y.round() as i32,
            width: (width.round() as i32).max(1),
            height: (height.round() as i32).max(1),
        }
    }

    /// How fast the window moves at `now`, in pixels a second; nothing
    /// before the movement began and once it is over.
    fn velocity_at(&self, now: Instant) -> [f32; 4] {
        let elapsed = now.saturating_duration_since(self.started_at);
        if elapsed.is_zero() || elapsed >= self.duration {
            return [0.0; 4];
        }
        let step = VELOCITY_STEP.min(elapsed);
        let before = self.components(now - step);
        let at = self.components(now);
        let seconds = step.as_secs_f32();
        std::array::from_fn(|index| (at[index] - before[index]) / seconds)
    }
}

/// The stretch of time a movement's speed is measured over.
const VELOCITY_STEP: Duration = Duration::from_millis(2);

fn components_of(rect: Rect) -> [f32; 4] {
    [
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    ]
}

#[derive(Debug, Clone)]
struct BorderTransition {
    start_color: (u8, u8, u8),
    target_color: (u8, u8, u8),
    start_alpha: u8,
    target_alpha: u8,
    started_at: Instant,
    duration: Duration,
    curve: AnimationCurve,
}

#[derive(Debug, Clone)]
struct OpacityTransition {
    start: u8,
    target: u8,
    started_at: Instant,
    duration: Duration,
    curve: AnimationCurve,
}

/// One cancellable scheduler for every runtime-owned transition.
///
/// Re-scheduling the same window replaces its existing transition. The caller
/// ticks the scheduler from the WM event loop, so animations never block the
/// loop and never create one thread per window.
#[derive(Debug, Clone)]
pub struct RuntimeAnimationScheduler {
    profile: AnimationProfile,
    rectangles: HashMap<WindowHandle, RectTransition>,
    borders: HashMap<WindowHandle, BorderTransition>,
    opacities: HashMap<WindowHandle, OpacityTransition>,
    recently_positioned: HashMap<WindowHandle, Instant>,
    /// Windows to close once their exit transitions finish. Kept apart from
    /// the transitions so a re-target (a layout pass, an opacity change) can
    /// never drop the close and leave a faded or shrunk window behind.
    pending_closes: HashSet<WindowHandle>,
}

impl RuntimeAnimationScheduler {
    pub fn new(profile: AnimationProfile) -> Self {
        Self {
            profile,
            rectangles: HashMap::new(),
            borders: HashMap::new(),
            opacities: HashMap::new(),
            recently_positioned: HashMap::new(),
            pending_closes: HashSet::new(),
        }
    }

    pub fn profile(&self) -> &AnimationProfile {
        &self.profile
    }

    pub fn set_profile(&mut self, profile: AnimationProfile) {
        self.profile = profile;
    }

    pub fn scope_enabled(&self, scope: TransitionScope) -> bool {
        self.profile.enabled && self.channel(scope).enabled
    }

    /// Whether windows are moved over time in this scope: its channel is
    /// on and movement is not reduced (`AnimationProfile::reduce_motion`).
    pub fn moves(&self, scope: TransitionScope) -> bool {
        self.scope_enabled(scope) && !self.profile.reduce_motion
    }

    pub fn border_enabled(&self) -> bool {
        self.profile.enabled && self.profile.border.enabled
    }

    fn channel(&self, scope: TransitionScope) -> &AnimationChannelProfile {
        match scope {
            TransitionScope::Layout => &self.profile.layout,
            TransitionScope::Workspace => &self.profile.workspace,
            TransitionScope::Explicit => &self.profile.window,
        }
    }

    /// Begin (or replace) the movement of a window. A spring that replaces
    /// a movement still under way takes over the speed the window has at
    /// this moment, so a window caught halfway is not stopped and started
    /// again: it swings round to where it is now wanted. The other curves
    /// are paths through time and start from rest, as they always did.
    fn begin_rect(
        &mut self,
        handle: WindowHandle,
        start: Rect,
        target: Rect,
        duration: Duration,
        curve: AnimationCurve,
        now: Instant,
    ) {
        let velocity = match (curve.spring_damping(), self.rectangles.get(&handle)) {
            (Some(_), Some(under_way)) => under_way.velocity_at(now),
            _ => [0.0; 4],
        };
        self.rectangles.insert(
            handle,
            RectTransition {
                start,
                target,
                started_at: now,
                duration,
                curve,
                velocity,
            },
        );
    }

    pub fn schedule_rect(
        &mut self,
        handle: WindowHandle,
        start: Rect,
        target: Rect,
        scope: TransitionScope,
        now: Instant,
    ) -> bool {
        if self.pending_closes.contains(&handle) {
            // The exit transition owns this window until it closes; report
            // the request as handled so callers do not snap it back.
            return true;
        }
        if !self.moves(scope) || start == target {
            self.rectangles.remove(&handle);
            return false;
        }

        let channel = self.channel(scope);
        let duration = Duration::from_millis(channel.duration_ms.max(16) as u64);
        let curve = channel.curve;
        self.begin_rect(handle, start, target, duration, curve, now);
        true
    }

    pub fn schedule_border(
        &mut self,
        handle: WindowHandle,
        start_color: (u8, u8, u8),
        target_color: (u8, u8, u8),
        start_alpha: u8,
        target_alpha: u8,
        now: Instant,
    ) -> bool {
        if !self.border_enabled() || (start_color == target_color && start_alpha == target_alpha) {
            self.borders.remove(&handle);
            return false;
        }

        self.borders.insert(
            handle,
            BorderTransition {
                start_color,
                target_color,
                start_alpha,
                target_alpha,
                started_at: now,
                duration: Duration::from_millis(self.profile.border.duration_ms.max(16) as u64),
                curve: self.profile.border.curve,
            },
        );
        true
    }

    pub fn schedule_explicit_rect(
        &mut self,
        handle: WindowHandle,
        start: Rect,
        target: Rect,
        duration_ms: u32,
        now: Instant,
    ) -> bool {
        if self.pending_closes.contains(&handle) {
            // The exit transition owns this window until it closes; report
            // the request as handled so callers do not snap it back.
            return true;
        }
        if !self.moves(TransitionScope::Explicit) || start == target {
            self.rectangles.remove(&handle);
            return false;
        }
        let duration = Duration::from_millis(explicit_duration(
            duration_ms,
            self.profile.window.duration_ms,
        ) as u64);
        let curve = self.profile.window.curve;
        self.begin_rect(handle, start, target, duration, curve, now);
        true
    }

    pub fn schedule_opacity(
        &mut self,
        handle: WindowHandle,
        start: u8,
        target: u8,
        duration_ms: u32,
        now: Instant,
    ) -> bool {
        if self.pending_closes.contains(&handle) {
            // The exit transition owns this window until it closes; report
            // the request as handled so callers do not snap it back.
            return true;
        }
        if !self.scope_enabled(TransitionScope::Explicit) || start == target {
            self.opacities.remove(&handle);
            return false;
        }
        self.opacities.insert(
            handle,
            OpacityTransition {
                start,
                target,
                started_at: now,
                duration: Duration::from_millis(explicit_duration(
                    duration_ms,
                    self.profile.window.duration_ms,
                ) as u64),
                curve: self.profile.window.curve,
            },
        );
        true
    }

    /// A window of a workspace that comes or goes fades, in the time and
    /// on the curve of the workspaces' channel.
    pub fn schedule_workspace_opacity(
        &mut self,
        handle: WindowHandle,
        start: u8,
        target: u8,
        now: Instant,
    ) -> bool {
        if self.pending_closes.contains(&handle) {
            return true;
        }
        if !self.scope_enabled(TransitionScope::Workspace) || start == target {
            self.opacities.remove(&handle);
            return false;
        }
        self.opacities.insert(
            handle,
            OpacityTransition {
                start,
                target,
                started_at: now,
                duration: Duration::from_millis(self.profile.workspace.duration_ms.max(16) as u64),
                curve: self.profile.workspace.curve,
            },
        );
        true
    }

    /// Whether a window's strength is on its way to another.
    pub fn opacity_running(&self, handle: WindowHandle) -> bool {
        self.opacities.contains_key(&handle)
    }

    pub fn schedule_exit_rect(
        &mut self,
        handle: WindowHandle,
        start: Rect,
        target: Rect,
        now: Instant,
    ) -> bool {
        if !self.moves(TransitionScope::Explicit) || start == target {
            self.rectangles.remove(&handle);
            return false;
        }
        let duration = Duration::from_millis(self.profile.window.duration_ms.max(16) as u64);
        let curve = self.profile.window.curve;
        self.begin_rect(handle, start, target, duration, curve, now);
        self.pending_closes.insert(handle);
        true
    }

    pub fn schedule_exit_opacity(&mut self, handle: WindowHandle, start: u8, now: Instant) -> bool {
        if !self.scope_enabled(TransitionScope::Explicit) || start == 0 {
            self.opacities.remove(&handle);
            return false;
        }
        self.opacities.insert(
            handle,
            OpacityTransition {
                start,
                target: 0,
                started_at: now,
                duration: Duration::from_millis(self.profile.window.duration_ms.max(16) as u64),
                curve: self.profile.window.curve,
            },
        );
        self.pending_closes.insert(handle);
        true
    }

    pub fn cancel_window(&mut self, handle: WindowHandle) {
        self.rectangles.remove(&handle);
        self.borders.remove(&handle);
        self.opacities.remove(&handle);
        self.recently_positioned.remove(&handle);
        self.pending_closes.remove(&handle);
    }

    pub fn active_count(&self) -> usize {
        self.rectangles.len() + self.borders.len() + self.opacities.len()
    }

    /// Where an in-flight geometry transition of `handle` will end.
    pub fn rect_target(&self, handle: WindowHandle) -> Option<Rect> {
        self.rectangles
            .get(&handle)
            .map(|transition| transition.target)
    }

    pub fn is_close_pending(&self, handle: WindowHandle) -> bool {
        self.pending_closes.contains(&handle)
    }

    pub fn is_position_event_suppressed(&self, handle: WindowHandle, now: Instant) -> bool {
        self.rectangles.contains_key(&handle)
            || self.recently_positioned.get(&handle).is_some_and(|last| {
                now.saturating_duration_since(*last) < Duration::from_millis(180)
            })
    }

    pub fn tick(&mut self, now: Instant) -> AnimationFrame {
        let mut frame = AnimationFrame::default();
        let mut completed_rectangles = Vec::new();
        let mut completed_borders = Vec::new();
        let mut completed_opacities = Vec::new();

        for (&handle, transition) in &self.rectangles {
            let linear = progress(transition.started_at, transition.duration, now);
            frame.rectangles.push((handle, transition.rect_at(now)));
            self.recently_positioned.insert(handle, now);
            if linear >= 1.0 {
                completed_rectangles.push(handle);
            }
        }

        for (&handle, transition) in &self.borders {
            let linear = progress(transition.started_at, transition.duration, now);
            let eased = transition.curve.sample(linear);
            frame.borders.push(BorderFrame {
                handle,
                color: (
                    lerp_u8(transition.start_color.0, transition.target_color.0, eased),
                    lerp_u8(transition.start_color.1, transition.target_color.1, eased),
                    lerp_u8(transition.start_color.2, transition.target_color.2, eased),
                ),
                alpha: lerp_u8(transition.start_alpha, transition.target_alpha, eased),
            });
            if linear >= 1.0 {
                completed_borders.push(handle);
            }
        }

        for (&handle, transition) in &self.opacities {
            let linear = progress(transition.started_at, transition.duration, now);
            let eased = transition.curve.sample(linear);
            frame
                .opacities
                .push((handle, lerp_u8(transition.start, transition.target, eased)));
            if linear >= 1.0 {
                completed_opacities.push(handle);
            }
        }

        for handle in completed_rectangles {
            self.rectangles.remove(&handle);
        }
        for handle in completed_borders {
            self.borders.remove(&handle);
        }
        for handle in completed_opacities {
            self.opacities.remove(&handle);
        }
        // Close once every exit transition of the window has finished (or
        // was removed, e.g. by a profile change).
        let rectangles = &self.rectangles;
        let opacities = &self.opacities;
        let close_windows = &mut frame.close_windows;
        self.pending_closes.retain(|handle| {
            let animating = rectangles.contains_key(handle) || opacities.contains_key(handle);
            if !animating {
                close_windows.push(*handle);
            }
            animating
        });
        frame.close_windows.sort_by_key(|handle| handle.0);
        self.recently_positioned
            .retain(|_, last| now.saturating_duration_since(*last) < Duration::from_millis(180));

        frame
    }
}

impl Default for RuntimeAnimationScheduler {
    fn default() -> Self {
        Self::new(AnimationProfile::default())
    }
}

fn progress(started_at: Instant, duration: Duration, now: Instant) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    (now.saturating_duration_since(started_at).as_secs_f32() / duration.as_secs_f32())
        .clamp(0.0, 1.0)
}

fn explicit_duration(requested_ms: u32, default_ms: u32) -> u32 {
    if requested_ms == 0 {
        default_ms
    } else {
        requested_ms
    }
    .clamp(16, 5000)
}

fn lerp_u8(start: u8, target: u8, progress: f32) -> u8 {
    (start as f32 + (target as f32 - start as f32) * progress)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::settings::AnimationCurve;

    fn profile() -> AnimationProfile {
        AnimationProfile {
            layout: AnimationChannelProfile::new(true, 100, AnimationCurve::Linear),
            workspace: AnimationChannelProfile::new(true, 100, AnimationCurve::Linear),
            border: AnimationChannelProfile::new(true, 100, AnimationCurve::Linear),
            window: AnimationChannelProfile::new(true, 100, AnimationCurve::Linear),
            ..AnimationProfile::default()
        }
    }

    #[test]
    fn layout_transition_reaches_exact_target() {
        let started = Instant::now();
        let mut scheduler = RuntimeAnimationScheduler::new(profile());
        let handle = WindowHandle(7);
        let start = Rect::new(0, 10, 100, 200);
        let target = Rect::new(200, 110, 300, 400);

        assert!(scheduler.schedule_rect(handle, start, target, TransitionScope::Layout, started,));
        let midpoint = scheduler.tick(started + Duration::from_millis(50));
        assert_eq!(
            midpoint.rectangles,
            vec![(handle, Rect::new(100, 60, 200, 300))]
        );

        let completed = scheduler.tick(started + Duration::from_millis(100));
        assert_eq!(completed.rectangles, vec![(handle, target)]);
        assert_eq!(scheduler.active_count(), 0);
        assert!(
            scheduler.is_position_event_suppressed(handle, started + Duration::from_millis(100))
        );
    }

    #[test]
    fn disabled_scope_snaps_instead_of_scheduling() {
        let mut disabled = profile();
        disabled.workspace.enabled = false;
        let mut scheduler = RuntimeAnimationScheduler::new(disabled);
        assert!(!scheduler.schedule_rect(
            WindowHandle(1),
            Rect::new(0, 0, 10, 10),
            Rect::new(20, 20, 10, 10),
            TransitionScope::Workspace,
            Instant::now(),
        ));
        assert_eq!(scheduler.active_count(), 0);
    }

    #[test]
    fn border_transition_uses_shared_curve_and_duration() {
        let started = Instant::now();
        let mut scheduler = RuntimeAnimationScheduler::new(profile());
        assert!(scheduler.schedule_border(
            WindowHandle(3),
            (0, 10, 20),
            (100, 110, 120),
            100,
            200,
            started,
        ));
        let frame = scheduler.tick(started + Duration::from_millis(50));
        assert_eq!(
            frame.borders,
            vec![BorderFrame {
                handle: WindowHandle(3),
                color: (50, 60, 70),
                alpha: 150,
            }]
        );
    }

    #[test]
    fn rectangle_scopes_use_independent_channel_timing() {
        let started = Instant::now();
        let mut scoped = profile();
        scoped.layout = AnimationChannelProfile::new(true, 100, AnimationCurve::Linear);
        scoped.workspace = AnimationChannelProfile::new(true, 200, AnimationCurve::Linear);
        let mut scheduler = RuntimeAnimationScheduler::new(scoped);
        let layout = WindowHandle(11);
        let workspace = WindowHandle(12);
        let start = Rect::new(0, 0, 100, 100);
        let target = Rect::new(100, 0, 100, 100);

        assert!(scheduler.schedule_rect(layout, start, target, TransitionScope::Layout, started,));
        assert!(scheduler.schedule_rect(
            workspace,
            start,
            target,
            TransitionScope::Workspace,
            started,
        ));
        let frame = scheduler.tick(started + Duration::from_millis(50));
        assert!(frame
            .rectangles
            .contains(&(layout, Rect::new(50, 0, 100, 100))));
        assert!(frame
            .rectangles
            .contains(&(workspace, Rect::new(25, 0, 100, 100))));
    }

    #[test]
    fn disabled_window_channel_rejects_explicit_effects() {
        let mut disabled = profile();
        disabled.window.enabled = false;
        let mut scheduler = RuntimeAnimationScheduler::new(disabled);
        assert!(!scheduler.schedule_opacity(WindowHandle(13), 0, 255, 0, Instant::now(),));
    }

    #[test]
    fn exit_transition_completes_with_one_deferred_close() {
        let started = Instant::now();
        let handle = WindowHandle(14);
        let mut scheduler = RuntimeAnimationScheduler::new(profile());
        assert!(scheduler.schedule_exit_opacity(handle, 200, started));
        assert!(scheduler.is_close_pending(handle));

        let midpoint = scheduler.tick(started + Duration::from_millis(50));
        assert!(midpoint.close_windows.is_empty());
        assert_eq!(midpoint.opacities, vec![(handle, 100)]);

        let mut disabled = profile();
        disabled.enabled = false;
        scheduler.set_profile(disabled);
        let completed = scheduler.tick(started + Duration::from_millis(100));
        assert_eq!(completed.close_windows, vec![handle]);
        assert_eq!(completed.opacities, vec![(handle, 0)]);
        assert!(!scheduler.is_close_pending(handle));
    }

    #[test]
    fn the_target_of_an_inflight_transition_is_known() {
        let started = Instant::now();
        let handle = WindowHandle(16);
        let mut scheduler = RuntimeAnimationScheduler::new(profile());
        assert_eq!(scheduler.rect_target(handle), None);
        let target = Rect::new(10, 10, 500, 400);
        assert!(scheduler.schedule_rect(
            handle,
            Rect::new(100, 100, 640, 480),
            target,
            TransitionScope::Layout,
            started,
        ));
        assert_eq!(scheduler.rect_target(handle), Some(target));
    }

    #[test]
    fn a_retarget_during_an_exit_never_loses_the_close() {
        let started = Instant::now();
        let handle = WindowHandle(15);
        let mut scheduler = RuntimeAnimationScheduler::new(profile());
        let start = Rect::new(0, 0, 400, 300);
        assert!(scheduler.schedule_exit_rect(handle, start, Rect::new(100, 0, 400, 300), started));

        // A layout pass and a focus-opacity change arrive mid-exit.
        assert!(scheduler.schedule_rect(
            handle,
            start,
            Rect::new(0, 0, 800, 600),
            TransitionScope::Layout,
            started,
        ));
        assert!(scheduler.schedule_opacity(handle, 255, 200, 0, started));
        assert!(scheduler.is_close_pending(handle));

        let completed = scheduler.tick(started + Duration::from_millis(100));
        assert_eq!(completed.close_windows, vec![handle]);
        assert_eq!(
            completed.rectangles,
            vec![(handle, Rect::new(100, 0, 400, 300))],
            "the exit transition must not be replaced"
        );
        assert!(!scheduler.is_close_pending(handle));
    }

    fn springy(curve: AnimationCurve) -> AnimationProfile {
        AnimationProfile {
            layout: AnimationChannelProfile::new(true, 200, curve),
            ..profile()
        }
    }

    /// The window's x, one frame after a movement to `target` that begins
    /// at 30 ms: in `interrupting` the window was on its way from 0 to
    /// 1000 until then, in the other it stood still at the same place.
    fn x_after_a_retarget(curve: AnimationCurve, target: i32) -> (i32, i32, i32) {
        let started = Instant::now();
        let handle = WindowHandle(21);
        let at = |x: i32| Rect::new(x, 0, 400, 300);
        let caught = started + Duration::from_millis(30);
        let next_frame = caught + Duration::from_millis(16);

        let mut interrupting = RuntimeAnimationScheduler::new(springy(curve));
        assert!(interrupting.schedule_rect(
            handle,
            at(0),
            at(1000),
            TransitionScope::Layout,
            started
        ));
        let halfway = interrupting.tick(caught).rectangles[0].1.x;
        assert!(halfway > 20 && halfway < 1000, "{halfway}");
        assert!(interrupting.schedule_rect(
            handle,
            at(halfway),
            at(target),
            TransitionScope::Layout,
            caught
        ));
        let carried = interrupting.tick(next_frame).rectangles[0].1.x;

        let mut from_rest = RuntimeAnimationScheduler::new(springy(curve));
        assert!(from_rest.schedule_rect(
            handle,
            at(halfway),
            at(target),
            TransitionScope::Layout,
            caught
        ));
        let rested = from_rest.tick(next_frame).rectangles[0].1.x;

        // Either way the window ends exactly where it is wanted, at rest.
        let end = caught + Duration::from_millis(200);
        assert_eq!(
            interrupting.tick(end).rectangles,
            vec![(handle, at(target))]
        );
        assert_eq!(interrupting.active_count(), 0);
        (halfway, carried, rested)
    }

    #[test]
    fn a_spring_that_interrupts_a_movement_goes_on_at_its_speed() {
        for curve in [AnimationCurve::Spring, AnimationCurve::SpringBouncy] {
            // Further along the same way: no stop in between.
            let (halfway, carried, rested) = x_after_a_retarget(curve, 1400);
            assert!(
                carried > rested + 20,
                "{curve:?}: {carried} with the speed carried, {rested} from rest (caught at {halfway})"
            );
            // Sent back where it came from: it does not turn on the spot,
            // it runs on a little first and then comes back.
            let (halfway, carried, rested) = x_after_a_retarget(curve, 0);
            assert!(rested < halfway, "{curve:?}: from rest it turns at once");
            assert!(
                carried > rested + 20,
                "{curve:?}: {carried} with the speed carried, {rested} from rest (caught at {halfway})"
            );
        }
    }

    #[test]
    fn the_other_curves_start_an_interrupting_movement_from_rest() {
        for curve in [
            AnimationCurve::Linear,
            AnimationCurve::EaseInOut,
            AnimationCurve::BackOut,
        ] {
            let (_, carried, rested) = x_after_a_retarget(curve, 1400);
            assert_eq!(carried, rested, "{curve:?}");
        }
    }

    #[test]
    fn a_movement_that_is_over_leaves_no_speed_behind() {
        let started = Instant::now();
        let handle = WindowHandle(22);
        let at = |x: i32| Rect::new(x, 0, 400, 300);
        let mut scheduler = RuntimeAnimationScheduler::new(springy(AnimationCurve::Spring));
        assert!(scheduler.schedule_rect(handle, at(0), at(1000), TransitionScope::Layout, started));
        // Scheduled again at the very moment it would end (before a tick
        // removed it): it is at rest by then.
        let over = started + Duration::from_millis(200);
        assert!(scheduler.schedule_rect(handle, at(1000), at(500), TransitionScope::Layout, over));
        let mut from_rest = RuntimeAnimationScheduler::new(springy(AnimationCurve::Spring));
        assert!(from_rest.schedule_rect(handle, at(1000), at(500), TransitionScope::Layout, over));
        let next = over + Duration::from_millis(16);
        assert_eq!(
            scheduler.tick(next).rectangles,
            from_rest.tick(next).rectangles
        );
    }

    #[test]
    fn with_motion_reduced_nothing_moves_but_what_fades_still_fades() {
        let started = Instant::now();
        let handle = WindowHandle(31);
        let mut still = profile();
        still.reduce_motion = true;
        let mut scheduler = RuntimeAnimationScheduler::new(still);
        let (here, there) = (Rect::new(0, 0, 400, 300), Rect::new(600, 0, 400, 300));
        // The caller is told to put the window in its place at once.
        for scope in [
            TransitionScope::Layout,
            TransitionScope::Workspace,
            TransitionScope::Explicit,
        ] {
            assert!(!scheduler.moves(scope));
            assert!(!scheduler.schedule_rect(handle, here, there, scope, started));
        }
        assert!(!scheduler.schedule_explicit_rect(handle, here, there, 0, started));
        assert!(!scheduler.schedule_exit_rect(handle, here, there, started));
        assert_eq!(scheduler.active_count(), 0);
        assert!(!scheduler.is_close_pending(handle));
        // Colour and opacity change in place, as before.
        assert!(scheduler.schedule_border(handle, (0, 0, 0), (255, 0, 0), 255, 255, started));
        assert!(scheduler.schedule_opacity(handle, 0, 255, 0, started));
        assert!(scheduler.schedule_exit_opacity(handle, 255, started));
        assert_eq!(scheduler.active_count(), 2);
    }

    #[test]
    fn explicit_opacity_transition_uses_requested_duration() {
        let started = Instant::now();
        let mut scheduler = RuntimeAnimationScheduler::new(profile());
        assert!(scheduler.schedule_opacity(WindowHandle(9), 0, 200, 200, started,));
        let frame = scheduler.tick(started + Duration::from_millis(50));
        assert_eq!(frame.opacities, vec![(WindowHandle(9), 50)]);
    }
}
