//! Size policy of floating windows: rule positions and sizes, minimum and
//! maximum size, fixed aspect ratio, and sizes remembered across sessions
//! (docs/PHASE2_PLAN.md, step 4.7).
//!
//! The per-window flags live in `WindowState`. The functions decide sizes;
//! the backend reads the current geometry and resizes the window.

use crate::rule_state::{PersistentSizeKey, RuleSize, WindowState};
use crate::rules::RulePosition;
use gevrek_model::geometry::Rect;
use std::collections::HashMap;

/// Where a rule puts a floating window: its size (from the rule, else the
/// size to restore, else the current one) and its position (coordinates in
/// the work area, centred, else the position to restore or the current one).
pub fn resolve_rule_rect(
    current: Rect,
    work_area: Rect,
    position: Option<&RulePosition>,
    size: Option<&RuleSize>,
    restore_position: Option<(i32, i32)>,
    restore_size: Option<(i32, i32)>,
) -> Rect {
    let (width, height) = size
        .map(|(width, height)| {
            (
                width.resolve(work_area.width).max(1),
                height.resolve(work_area.height).max(1),
            )
        })
        .or(restore_size)
        .unwrap_or((current.width, current.height));
    let (x, y) = match position {
        Some(RulePosition::Coordinates { x, y }) => (
            work_area.x + x.resolve(work_area.width),
            work_area.y + y.resolve(work_area.height),
        ),
        Some(RulePosition::Centered) => (
            work_area.x + (work_area.width - width) / 2,
            work_area.y + (work_area.height - height) / 2,
        ),
        None => restore_position.unwrap_or((current.x, current.y)),
    };
    Rect::new(x, y, width, height)
}

/// Clamp a floating window's size to its rule limits. A maximum below the
/// minimum is raised to it. With an aspect ratio (`width:height` base), the
/// axis the user changed more drives the scale; `previous_size` tells which.
pub fn constrain_rule_size(
    width: i32,
    height: i32,
    work_area: Rect,
    min_size: Option<&RuleSize>,
    max_size: Option<&RuleSize>,
    aspect_ratio: Option<(i32, i32)>,
    previous_size: Option<(i32, i32)>,
) -> (i32, i32) {
    let (min_width, min_height) = min_size
        .map(|(width, height)| {
            (
                width.resolve(work_area.width).max(1),
                height.resolve(work_area.height).max(1),
            )
        })
        .unwrap_or((1, 1));
    let (max_width, max_height) = max_size
        .map(|(width, height)| {
            (
                width.resolve(work_area.width).max(1),
                height.resolve(work_area.height).max(1),
            )
        })
        .unwrap_or((i32::MAX, i32::MAX));
    let max_width = max_width.max(min_width);
    let max_height = max_height.max(min_height);
    let Some((base_width, base_height)) =
        aspect_ratio.filter(|(width, height)| *width > 0 && *height > 0)
    else {
        return (
            width.clamp(min_width, max_width),
            height.clamp(min_height, max_height),
        );
    };

    let width_driven = previous_size.is_none_or(|(previous_width, previous_height)| {
        i64::from((width - previous_width).abs()) * i64::from(base_height)
            >= i64::from((height - previous_height).abs()) * i64::from(base_width)
    });
    let desired_scale = if width_driven {
        width as f64 / base_width as f64
    } else {
        height as f64 / base_height as f64
    };
    let min_scale =
        (min_width as f64 / base_width as f64).max(min_height as f64 / base_height as f64);
    let max_scale = (max_width as f64 / base_width as f64)
        .min(max_height as f64 / base_height as f64)
        .max(min_scale);
    let scale = desired_scale.clamp(min_scale, max_scale);
    (
        (base_width as f64 * scale).round().max(1.0) as i32,
        (base_height as f64 * scale).round().max(1.0) as i32,
    )
}

// ---- per-window constraints ---------------------------------------------------

/// Set the min/max size one rule source asks for (`None` clears it).
pub fn set_size_rules(
    state: &mut WindowState,
    dynamic: bool,
    min_size: Option<RuleSize>,
    max_size: Option<RuleSize>,
) {
    *state.min_size.source_mut(dynamic) = min_size;
    *state.max_size.source_mut(dynamic) = max_size;
}

/// Set the keep-aspect-ratio flag of one rule source. Returns whether the
/// ratio is kept now; if so the caller seeds it with the current size
/// (`seed_aspect_ratio`). When it is no longer kept, the base is dropped.
pub fn set_aspect_ratio_rule(
    state: &mut WindowState,
    dynamic: bool,
    setting: Option<bool>,
) -> bool {
    *state.keep_aspect_ratio.source_mut(dynamic) = setting;
    let kept = state.keep_aspect_ratio.enabled();
    if !kept {
        state.aspect_ratio_base = None;
        state.last_aspect_size = None;
    }
    kept
}

/// The size whose ratio is kept: the window's size when the rule first
/// applied. Later calls keep the existing base.
pub fn seed_aspect_ratio(state: &mut WindowState, current: (i32, i32)) {
    let size = (current.0.max(1), current.1.max(1));
    state.aspect_ratio_base.get_or_insert(size);
    state.last_aspect_size.get_or_insert(size);
}

/// The aspect ratio base, if the ratio is kept.
fn kept_aspect_ratio(state: &WindowState) -> Option<(i32, i32)> {
    state
        .keep_aspect_ratio
        .enabled()
        .then_some(state.aspect_ratio_base)
        .flatten()
}

/// Whether any size constraint applies to the window.
pub fn has_size_constraints(state: &WindowState) -> bool {
    state.min_size.effective().is_some()
        || state.max_size.effective().is_some()
        || kept_aspect_ratio(state).is_some()
}

/// The size a floating window may have, given the size it has now. Records
/// the result as the last size when the aspect ratio is kept, so the next
/// resize knows which axis the user changed.
pub fn constrained_size(
    state: &mut WindowState,
    work_area: Rect,
    width: i32,
    height: i32,
) -> (i32, i32) {
    let aspect_ratio = kept_aspect_ratio(state);
    let target = constrain_rule_size(
        width,
        height,
        work_area,
        state.min_size.effective(),
        state.max_size.effective(),
        aspect_ratio,
        state.last_aspect_size,
    );
    if aspect_ratio.is_some() {
        state.last_aspect_size = Some(target);
    }
    target
}

// ---- persistent sizes ------------------------------------------------------------

/// Executable, initial class and initial title, compared case-insensitively.
pub fn normalize_size_key(
    exe: &str,
    initial_class: &str,
    initial_title: &str,
) -> PersistentSizeKey {
    (
        exe.trim().to_ascii_lowercase(),
        initial_class.trim().to_ascii_lowercase(),
        initial_title.trim().to_ascii_lowercase(),
    )
}

/// Floating window sizes remembered across sessions, by window identity.
#[derive(Debug, Default)]
pub struct PersistentSizes {
    sizes: HashMap<PersistentSizeKey, (i32, i32)>,
}

impl PersistentSizes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the store with saved sizes (`exe, class, title, width,
    /// height`); empty sizes are dropped.
    pub fn load<'a>(
        &mut self,
        saved: impl IntoIterator<Item = (&'a str, &'a str, &'a str, i32, i32)>,
    ) {
        self.sizes = saved
            .into_iter()
            .filter(|(_, _, _, width, height)| *width > 0 && *height > 0)
            .map(|(exe, class, title, width, height)| {
                (normalize_size_key(exe, class, title), (width, height))
            })
            .collect();
    }

    /// Every remembered size, sorted by key.
    pub fn entries(&self) -> Vec<(PersistentSizeKey, (i32, i32))> {
        let mut entries: Vec<_> = self
            .sizes
            .iter()
            .map(|(key, size)| (key.clone(), *size))
            .collect();
        entries.sort();
        entries
    }

    pub fn get(&self, key: &PersistentSizeKey) -> Option<(i32, i32)> {
        self.sizes.get(key).copied()
    }
}

/// Set the persistent-size flag of one rule source. Returns the remembered
/// size to restore now: only for a floating window, and only once per
/// window. `key` identifies the window (asked for only when needed).
pub fn set_persistent_size_rule(
    state: &mut WindowState,
    sizes: &PersistentSizes,
    dynamic: bool,
    setting: Option<bool>,
    floating: bool,
    key: impl FnOnce() -> PersistentSizeKey,
) -> Option<(i32, i32)> {
    *state.persistent_size.source_mut(dynamic) = setting;
    if !state.persistent_size.enabled() {
        state.persistent_size_key = None;
        state.persistent_size_restored = false;
        return None;
    }
    let key = key();
    state.persistent_size_key = Some(key.clone());
    if !floating || std::mem::replace(&mut state.persistent_size_restored, true) {
        return None;
    }
    sizes.get(&key)
}

/// Remember a floating window's size if its rules ask for it. Returns true
/// when the stored size changed.
pub fn remember_size(
    state: &mut WindowState,
    sizes: &mut PersistentSizes,
    floating: bool,
    width: i32,
    height: i32,
    key: impl FnOnce() -> PersistentSizeKey,
) -> bool {
    if width <= 0 || height <= 0 || !state.persistent_size.enabled() || !floating {
        return false;
    }
    let key = state.persistent_size_key.get_or_insert_with(key).clone();
    sizes.sizes.insert(key, (width, height)) != Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::settings::RuleMeasure;

    const AREA: Rect = Rect {
        x: 100,
        y: 50,
        width: 1600,
        height: 900,
    };

    fn key() -> PersistentSizeKey {
        normalize_size_key(" App.EXE ", "Class", "Title")
    }

    #[test]
    fn min_and_max_clamp_and_a_low_max_yields_to_the_min() {
        let mut state = WindowState::default();
        assert!(!has_size_constraints(&state));
        set_size_rules(
            &mut state,
            false,
            Some((RuleMeasure::Pixels(400), RuleMeasure::Percent(30))),
            Some((RuleMeasure::Percent(75), RuleMeasure::Pixels(700))),
        );
        assert!(has_size_constraints(&state));
        assert_eq!(constrained_size(&mut state, AREA, 200, 1000), (400, 700));

        // A dynamic rule overrides the static one.
        set_size_rules(
            &mut state,
            true,
            Some((RuleMeasure::Pixels(900), RuleMeasure::Pixels(700))),
            None,
        );
        assert_eq!(constrained_size(&mut state, AREA, 500, 300), (900, 700));
    }

    #[test]
    fn a_kept_aspect_ratio_follows_the_axis_the_user_changed() {
        let mut state = WindowState::default();
        assert!(set_aspect_ratio_rule(&mut state, false, Some(true)));
        seed_aspect_ratio(&mut state, (800, 400));
        seed_aspect_ratio(&mut state, (10, 10));
        assert_eq!(
            state.aspect_ratio_base,
            Some((800, 400)),
            "the first size is the base"
        );

        // Width changed more: height follows.
        assert_eq!(constrained_size(&mut state, AREA, 1000, 410), (1000, 500));
        // Then height changed: width follows.
        assert_eq!(constrained_size(&mut state, AREA, 1000, 300), (600, 300));

        assert!(!set_aspect_ratio_rule(&mut state, false, None));
        assert_eq!(state.aspect_ratio_base, None);
        assert_eq!(constrained_size(&mut state, AREA, 123, 456), (123, 456));
    }

    #[test]
    fn a_remembered_size_is_restored_once_for_a_floating_window() {
        let mut sizes = PersistentSizes::new();
        sizes.load([
            ("app.exe", "class", "title", 640, 480),
            ("bad", "", "", 0, 10),
        ]);
        assert_eq!(sizes.entries().len(), 1);

        let mut state = WindowState::default();
        // Tiled: nothing to restore, and the one-shot is not used up.
        assert_eq!(
            set_persistent_size_rule(&mut state, &sizes, false, Some(true), false, key),
            None
        );
        assert_eq!(
            set_persistent_size_rule(&mut state, &sizes, false, Some(true), true, key),
            Some((640, 480))
        );
        assert_eq!(
            set_persistent_size_rule(&mut state, &sizes, false, Some(true), true, key),
            None
        );

        // Rule gone: the key and the one-shot reset.
        assert_eq!(
            set_persistent_size_rule(&mut state, &sizes, false, None, true, key),
            None
        );
        assert_eq!(state.persistent_size_key, None);
        assert!(!state.persistent_size_restored);
    }

    #[test]
    fn sizes_are_remembered_only_for_floating_windows_with_the_rule() {
        let mut sizes = PersistentSizes::new();
        let mut state = WindowState::default();
        assert!(
            !remember_size(&mut state, &mut sizes, true, 800, 600, key),
            "no rule"
        );
        state.persistent_size.static_value = Some(true);
        assert!(
            !remember_size(&mut state, &mut sizes, false, 800, 600, key),
            "tiled"
        );
        assert!(remember_size(&mut state, &mut sizes, true, 800, 600, key));
        assert!(
            !remember_size(&mut state, &mut sizes, true, 800, 600, key),
            "unchanged"
        );
        assert_eq!(sizes.get(&key()), Some((800, 600)));
    }

    #[test]
    fn rule_rects_resolve_size_then_position() {
        let current = Rect::new(10, 20, 300, 200);
        let size = (RuleMeasure::Percent(50), RuleMeasure::Pixels(450));
        let centered = resolve_rule_rect(
            current,
            AREA,
            Some(&RulePosition::Centered),
            Some(&size),
            None,
            None,
        );
        assert_eq!(centered, Rect::new(500, 275, 800, 450));
        let restored = resolve_rule_rect(current, AREA, None, None, Some((5, 6)), Some((70, 80)));
        assert_eq!(restored, Rect::new(5, 6, 70, 80));
        assert_eq!(
            resolve_rule_rect(current, AREA, None, None, None, None),
            current
        );
    }
}
