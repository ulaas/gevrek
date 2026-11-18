//! Per-window policy state: one `WindowState` per window instead of a
//! separate map per property (docs/PHASE2_PLAN.md, step 4.1).
//!
//! Absent windows read as `WindowState::default()`, so "not in the map"
//! and "flag off" mean the same thing, as they did with the separate maps.
//! A window's state goes away in one call (`forget`) when it is destroyed.

use crate::rules::RuleResolution;
use gevrek_model::geometry::Rect;
use gevrek_model::ids::{OutputId, WindowId};
use gevrek_model::platform::windows::{SystemBackdrop, WindowCorner};
use gevrek_model::settings::{RuleColor, RuleMeasure};
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::time::Instant;

/// Executable, class and title a persistent floating size is stored under.
pub type PersistentSizeKey = (String, String, String);
/// Width and height from a rule (pixels or a share of the work area).
pub type RuleSize = (RuleMeasure, RuleMeasure);

/// A value set by window rules. Static rules are resolved once when the
/// window appears; dynamic rules are re-resolved when it changes and win.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleSetting<T> {
    pub static_value: Option<T>,
    pub dynamic_value: Option<T>,
}

impl<T> Default for RuleSetting<T> {
    fn default() -> Self {
        Self {
            static_value: None,
            dynamic_value: None,
        }
    }
}

impl<T> RuleSetting<T> {
    /// The dynamic value if a dynamic rule set one, else the static one.
    pub fn effective(&self) -> Option<&T> {
        self.dynamic_value.as_ref().or(self.static_value.as_ref())
    }

    /// The value of one source; `dynamic` picks which.
    pub fn source_mut(&mut self, dynamic: bool) -> &mut Option<T> {
        if dynamic {
            &mut self.dynamic_value
        } else {
            &mut self.static_value
        }
    }

    pub fn is_unset(&self) -> bool {
        self.static_value.is_none() && self.dynamic_value.is_none()
    }
}

impl RuleSetting<bool> {
    /// Whether the effective value is `true`.
    pub fn enabled(&self) -> bool {
        self.effective().copied().unwrap_or(false)
    }
}

/// A window the user is dragging or resizing.
#[derive(Debug, Clone, PartialEq)]
pub struct DragState {
    pub window: WindowId,
    pub last_move_time: Instant,
    pub original_monitor: Option<OutputId>,
    /// Size and position when the drag started.
    pub original_rect: Option<Rect>,
    /// The size changed, so this is a resize rather than a move.
    pub is_resizing: bool,
    pub last_known_rect: Option<Rect>,
}

/// What a window looked like before dynamic rules changed it, so it can be
/// put back when they stop matching.
#[derive(Debug, Clone, Copy)]
pub struct DynamicRuleBaseline {
    pub workspace: Option<(OutputId, usize)>,
    pub rect: Option<Rect>,
    pub managed: bool,
    pub floating: bool,
    pub always_on_top: bool,
    pub opacity: u8,
    pub maximized: bool,
    pub fullscreen: bool,
    pub border_color: RuleColor,
    pub caption_color: RuleColor,
    pub backdrop: SystemBackdrop,
    pub corners: WindowCorner,
}

/// The part of a window's look that rules set through backend effects, as
/// the backend reports it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedStyle {
    pub always_on_top: bool,
    pub border_color: RuleColor,
    pub caption_color: RuleColor,
    pub backdrop: SystemBackdrop,
    pub corners: WindowCorner,
}

/// The effects that bring a window's stacking and Windows-only look in line
/// with a rule resolution.
///
/// A value the rules set now is applied. A value the previous resolution
/// set and this one no longer does goes back to the `baseline` (what the
/// window looked like before dynamic rules touched it). Nothing is emitted
/// for a value the window already has.
pub fn style_effects(
    id: crate::WindowHandle,
    resolution: &RuleResolution,
    previous: Option<&RuleResolution>,
    baseline: Option<&DynamicRuleBaseline>,
    observed: ObservedStyle,
) -> Vec<crate::contract::Effect> {
    use crate::contract::{Effect, PlatformEffect, StackOp, WindowsEffect};
    // `was_set`: the previous resolution set this value.
    fn wanted<T: Copy>(now: Option<T>, was_set: bool, baseline: Option<T>) -> Option<T> {
        now.or(if was_set { baseline } else { None })
    }
    let mut effects = Vec::new();
    let always_on_top = wanted(
        resolution.always_on_top,
        previous.is_some_and(|previous| previous.always_on_top.is_some()),
        baseline.map(|baseline| baseline.always_on_top),
    );
    if let Some(enabled) = always_on_top.filter(|enabled| *enabled != observed.always_on_top) {
        effects.push(Effect::Stack {
            id,
            op: StackOp::AlwaysOnTop(enabled),
        });
    }
    let border_color = wanted(
        resolution.border_color,
        previous.is_some_and(|previous| previous.border_color.is_some()),
        baseline.map(|baseline| baseline.border_color),
    );
    if let Some(color) = border_color.filter(|color| *color != observed.border_color) {
        effects.push(Effect::Platform(PlatformEffect::Windows(
            WindowsEffect::NativeBorderColor { id, color },
        )));
    }
    let caption_color = wanted(
        resolution.caption_color,
        previous.is_some_and(|previous| previous.caption_color.is_some()),
        baseline.map(|baseline| baseline.caption_color),
    );
    if let Some(color) = caption_color.filter(|color| *color != observed.caption_color) {
        effects.push(Effect::Platform(PlatformEffect::Windows(
            WindowsEffect::CaptionColor { id, color },
        )));
    }
    let backdrop = wanted(
        resolution.backdrop,
        previous.is_some_and(|previous| previous.backdrop.is_some()),
        baseline.map(|baseline| baseline.backdrop),
    );
    if let Some(backdrop) = backdrop.filter(|backdrop| *backdrop != observed.backdrop) {
        effects.push(Effect::Platform(PlatformEffect::Windows(
            WindowsEffect::Backdrop { id, backdrop },
        )));
    }
    let corners = wanted(
        resolution.corners,
        previous.is_some_and(|previous| previous.corners.is_some()),
        baseline.map(|baseline| baseline.corners),
    );
    if let Some(corner) = corners.filter(|corner| *corner != observed.corners) {
        effects.push(Effect::Platform(PlatformEffect::Windows(
            WindowsEffect::Corner { id, corner },
        )));
    }
    effects
}

/// What the rules and the user's tag commands have recorded for one window:
/// its tags and the names and effects of the rules that matched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleMetadata {
    /// Tags from the static rules plus the ones added by command. Dynamic
    /// rules are resolved against these.
    pub base_tags: BTreeSet<String>,
    /// The base tags plus what the dynamic rules add or remove.
    pub effective_tags: BTreeSet<String>,
    pub static_rules: Vec<String>,
    pub dynamic_rules: Vec<String>,
    pub static_effects: Vec<String>,
    pub dynamic_effects: Vec<String>,
}

/// `RuleMetadata` for every window that has any.
#[derive(Debug, Clone, Default)]
pub struct RuleMetadataMap {
    windows: std::collections::HashMap<crate::WindowHandle, RuleMetadata>,
}

impl RuleMetadataMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The static rules were resolved: they define the base tags.
    pub fn apply_static(&mut self, id: crate::WindowHandle, resolution: &RuleResolution) {
        let metadata = self.windows.entry(id).or_default();
        metadata.base_tags = resolution.tags.clone();
        metadata.effective_tags = resolution.tags.clone();
        metadata.static_rules = resolution.matched_rules.clone();
        metadata.static_effects = resolution.effect_descriptions();
    }

    /// The dynamic rules were resolved: they define the effective tags.
    pub fn apply_dynamic(&mut self, id: crate::WindowHandle, resolution: &RuleResolution) {
        let metadata = self.windows.entry(id).or_default();
        metadata.effective_tags = resolution.tags.clone();
        metadata.dynamic_rules = resolution.matched_rules.clone();
        metadata.dynamic_effects = resolution.effect_descriptions();
    }

    pub fn tags(&self, id: crate::WindowHandle) -> BTreeSet<String> {
        self.windows
            .get(&id)
            .map(|metadata| metadata.effective_tags.clone())
            .unwrap_or_default()
    }

    pub fn base_tags(&self, id: crate::WindowHandle) -> BTreeSet<String> {
        self.windows
            .get(&id)
            .map(|metadata| metadata.base_tags.clone())
            .unwrap_or_default()
    }

    /// The names of the rules that matched, static ones first.
    pub fn matched_rules(&self, id: crate::WindowHandle) -> Vec<String> {
        self.windows.get(&id).map_or_else(Vec::new, |metadata| {
            metadata
                .static_rules
                .iter()
                .chain(&metadata.dynamic_rules)
                .cloned()
                .collect()
        })
    }

    /// What the matching rules do, static ones first.
    pub fn effects(&self, id: crate::WindowHandle) -> Vec<String> {
        self.windows.get(&id).map_or_else(Vec::new, |metadata| {
            metadata
                .static_effects
                .iter()
                .chain(&metadata.dynamic_effects)
                .cloned()
                .collect()
        })
    }

    /// Add a tag by command. Returns whether it was new; an empty tag is an
    /// error.
    pub fn add_tag(&mut self, id: crate::WindowHandle, tag: &str) -> Result<bool, String> {
        let tag = crate::rules::normalize_tag(tag);
        if tag.is_empty() {
            return Err("A window tag cannot be empty".to_string());
        }
        let metadata = self.windows.entry(id).or_default();
        let inserted = metadata.base_tags.insert(tag.clone());
        metadata.effective_tags.insert(tag);
        Ok(inserted)
    }

    /// Remove a tag. Returns whether the window had it as a base tag.
    pub fn remove_tag(&mut self, id: crate::WindowHandle, tag: &str) -> bool {
        let tag = crate::rules::normalize_tag(tag);
        let Some(metadata) = self.windows.get_mut(&id) else {
            return false;
        };
        let removed = metadata.base_tags.remove(&tag);
        metadata.effective_tags.remove(&tag);
        removed
    }

    /// Remove every tag. Returns how many base tags there were.
    pub fn clear_tags(&mut self, id: crate::WindowHandle) -> usize {
        let Some(metadata) = self.windows.get_mut(&id) else {
            return 0;
        };
        let removed = metadata.base_tags.len();
        metadata.base_tags.clear();
        metadata.effective_tags.clear();
        removed
    }

    /// Remove the tag if the window has it, add it otherwise. Returns
    /// whether the window has it afterwards.
    pub fn toggle_tag(&mut self, id: crate::WindowHandle, tag: &str) -> Result<bool, String> {
        let tag = crate::rules::normalize_tag(tag);
        if tag.is_empty() {
            return Err("A window tag cannot be empty".to_string());
        }
        if self.remove_tag(id, &tag) {
            Ok(false)
        } else {
            self.add_tag(id, &tag)?;
            Ok(true)
        }
    }

    /// The window is gone.
    pub fn forget(&mut self, id: crate::WindowHandle) {
        self.windows.remove(&id);
    }
}

/// The last dynamic rule resolution applied to a window, and its baseline.
#[derive(Debug, Clone)]
pub struct DynamicRuleState {
    pub baseline: DynamicRuleBaseline,
    pub resolution: RuleResolution,
}

/// How a window was before it was brought to the middle of a display
/// (`DispatchCommand::ToggleWindowCentered`): to be put back so.
#[derive(Debug, Clone, PartialEq)]
pub struct BeforeCentered {
    /// The workspace it was on, by its id.
    pub workspace: usize,
    pub floating: bool,
    /// Where it was, and how large.
    pub rect: Rect,
    /// Its place among the tiles, where it was tiled.
    pub place: Option<usize>,
}

/// Everything the WM tracks about one window beyond its place in the
/// layout.
#[derive(Debug, Clone, Default)]
pub struct WindowState {
    /// The window's creation has been handled (it is adopted).
    pub known: bool,
    /// Released from tiling, focus cycling and borders for this session
    ///.
    pub session_unmanaged: bool,
    /// Hidden by the WM (workspace switch, group tab, scratchpad); not
    /// restored when seen hidden.
    pub hidden_by_wm: bool,
    pub minimized: bool,
    /// Workspace (by id) a minimized tiled window left; it returns there on
    /// restore if that workspace is visible.
    pub minimized_origin: Option<usize>,
    pub drag: Option<DragState>,
    /// It was brought to the middle of a display, and was so before.
    pub centered_from: Option<BeforeCentered>,

    // ---- rules ----
    pub dynamic_rules: Option<DynamicRuleState>,
    /// Reclaim focus while visible.
    pub stay_focused: RuleSetting<bool>,
    /// Do not take focus when it opens.
    pub no_initial_focus: RuleSetting<bool>,
    /// Never take focus.
    pub no_focus: RuleSetting<bool>,
    /// Focus-follows-mouse skips it.
    pub no_follow_mouse: RuleSetting<bool>,
    /// A one-shot initial-focus suppression waits for the window's focus
    /// event (Windows may send Created and Focused in either order).
    pub initial_focus_suppression_pending: bool,

    // ---- size constraints of floating windows ----
    pub persistent_size: RuleSetting<bool>,
    pub persistent_size_key: Option<PersistentSizeKey>,
    /// The remembered size was applied once already.
    pub persistent_size_restored: bool,
    pub min_size: RuleSetting<RuleSize>,
    pub max_size: RuleSetting<RuleSize>,
    pub keep_aspect_ratio: RuleSetting<bool>,
    /// Width and height the aspect ratio is kept at.
    pub aspect_ratio_base: Option<(i32, i32)>,
    pub last_aspect_size: Option<(i32, i32)>,
}

impl WindowState {
    /// The window never keeps focus (`no_focus`, unless `stay_focused`).
    pub fn rejects_focus(&self) -> bool {
        self.no_focus.enabled() && !self.stay_focused.enabled()
    }

    /// The window must not take focus when it opens (`no_initial_focus`,
    /// unless `stay_focused`).
    pub fn suppresses_initial_focus(&self) -> bool {
        self.no_initial_focus.enabled() && !self.stay_focused.enabled()
    }

    /// Focus-follows-mouse may select the window.
    pub fn allows_follow_mouse(&self) -> bool {
        !self.rejects_focus() && !self.no_follow_mouse.enabled()
    }
}

/// The `WindowState` of every window that has one.
#[derive(Debug, Default)]
pub struct WindowStateMap {
    states: HashMap<WindowId, WindowState>,
    /// Returned for windows without an entry.
    absent: WindowState,
}

impl WindowStateMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The window's state; the default state if it has none.
    pub fn get(&self, id: WindowId) -> &WindowState {
        self.states.get(&id).unwrap_or(&self.absent)
    }

    /// The window's state, created if missing.
    pub fn get_mut(&mut self, id: WindowId) -> &mut WindowState {
        self.states.entry(id).or_default()
    }

    /// Drop everything about a window.
    pub fn forget(&mut self, id: WindowId) {
        self.states.remove(&id);
    }

    /// Keep only the windows for which `keep` returns true.
    pub fn retain(&mut self, mut keep: impl FnMut(WindowId, &WindowState) -> bool) {
        self.states.retain(|id, state| keep(*id, state));
    }

    /// Windows whose state matches `predicate`, in id order.
    pub fn ids_where(&self, predicate: impl Fn(&WindowState) -> bool) -> Vec<WindowId> {
        let mut ids: Vec<WindowId> = self
            .states
            .iter()
            .filter(|(_, state)| predicate(state))
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }

    pub fn iter(&self) -> impl Iterator<Item = (WindowId, &WindowState)> {
        self.states.iter().map(|(id, state)| (*id, state))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (WindowId, &mut WindowState)> {
        self.states.iter_mut().map(|(id, state)| (*id, state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dynamic_rule_value_overrides_the_static_one() {
        let mut setting = RuleSetting::default();
        assert!(!setting.enabled());
        *setting.source_mut(false) = Some(true);
        assert!(setting.enabled());
        *setting.source_mut(true) = Some(false);
        assert!(!setting.enabled(), "dynamic false beats static true");
        setting.dynamic_value = None;
        assert!(setting.enabled(), "back to the static value");
    }

    #[test]
    fn windows_without_state_read_as_default_and_forget_drops_everything() {
        let mut states = WindowStateMap::new();
        let id = WindowId(7);
        assert!(!states.get(id).known);

        let state = states.get_mut(id);
        state.known = true;
        state.minimized_origin = Some(3);
        state.no_focus.static_value = Some(true);
        assert!(states.get(id).no_focus.enabled());
        assert_eq!(states.ids_where(|s| s.known), vec![id]);

        states.forget(id);
        assert!(!states.get(id).known);
        assert_eq!(states.get(id).minimized_origin, None);
        assert!(states.iter().next().is_none());
    }
}

#[cfg(test)]
mod style_effect_tests {
    use super::*;
    use crate::contract::{Effect, PlatformEffect, StackOp, WindowsEffect};

    const OBSERVED: ObservedStyle = ObservedStyle {
        always_on_top: false,
        border_color: RuleColor::Default,
        caption_color: RuleColor::Default,
        backdrop: SystemBackdrop::Auto,
        corners: WindowCorner::Default,
    };

    fn baseline() -> DynamicRuleBaseline {
        DynamicRuleBaseline {
            workspace: None,
            rect: None,
            managed: true,
            floating: false,
            always_on_top: false,
            opacity: 255,
            maximized: false,
            fullscreen: false,
            border_color: RuleColor::Default,
            caption_color: RuleColor::Default,
            backdrop: SystemBackdrop::Auto,
            corners: WindowCorner::Default,
        }
    }

    #[test]
    fn a_rule_sets_only_what_differs_from_the_window() {
        let id = crate::WindowHandle(7);
        let resolution = RuleResolution {
            always_on_top: Some(true),
            backdrop: Some(SystemBackdrop::Mica),
            corners: Some(WindowCorner::Default),
            ..RuleResolution::default()
        };
        assert_eq!(
            style_effects(id, &resolution, None, None, OBSERVED),
            vec![
                Effect::Stack {
                    id,
                    op: StackOp::AlwaysOnTop(true)
                },
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::Backdrop {
                    id,
                    backdrop: SystemBackdrop::Mica
                })),
            ],
            "the corners already match, so nothing is sent for them"
        );
        assert!(style_effects(id, &RuleResolution::default(), None, None, OBSERVED).is_empty());
    }

    #[test]
    fn rule_colours_are_set_and_put_back() {
        let id = crate::WindowHandle(7);
        let red = RuleColor::Rgb { r: 200, g: 0, b: 0 };
        let rule = RuleResolution {
            border_color: Some(red),
            caption_color: Some(RuleColor::Hidden),
            ..RuleResolution::default()
        };
        assert_eq!(
            style_effects(id, &rule, None, None, OBSERVED),
            vec![
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::NativeBorderColor {
                    id,
                    color: red
                })),
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::CaptionColor {
                    id,
                    color: RuleColor::Hidden
                })),
            ]
        );
        // The rule stops matching: the border goes back, the caption (which
        // the window still shows as the baseline has it) is left alone.
        let now = ObservedStyle {
            border_color: red,
            ..OBSERVED
        };
        assert_eq!(
            style_effects(
                id,
                &RuleResolution::default(),
                Some(&rule),
                Some(&baseline()),
                now
            ),
            vec![Effect::Platform(PlatformEffect::Windows(
                WindowsEffect::NativeBorderColor {
                    id,
                    color: RuleColor::Default
                }
            ))]
        );
    }

    #[test]
    fn a_rule_that_stops_matching_puts_the_baseline_back() {
        let id = crate::WindowHandle(7);
        let previous = RuleResolution {
            always_on_top: Some(true),
            corners: Some(WindowCorner::Round),
            ..RuleResolution::default()
        };
        let now = ObservedStyle {
            always_on_top: true,
            corners: WindowCorner::Round,
            ..OBSERVED
        };
        let baseline = baseline();
        assert_eq!(
            style_effects(
                id,
                &RuleResolution::default(),
                Some(&previous),
                Some(&baseline),
                now
            ),
            vec![
                Effect::Stack {
                    id,
                    op: StackOp::AlwaysOnTop(false)
                },
                Effect::Platform(PlatformEffect::Windows(WindowsEffect::Corner {
                    id,
                    corner: WindowCorner::Default
                })),
            ],
            "the backdrop was never set by a rule, so it is left alone"
        );
        // Without a baseline there is nothing to go back to.
        assert!(
            style_effects(id, &RuleResolution::default(), Some(&previous), None, now).is_empty()
        );
    }
}

#[cfg(test)]
mod rule_metadata_tests {
    use super::*;

    fn resolution(tags: &[&str], rules: &[&str]) -> RuleResolution {
        RuleResolution {
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            matched_rules: rules.iter().map(|rule| rule.to_string()).collect(),
            ..RuleResolution::default()
        }
    }

    #[test]
    fn static_rules_set_the_base_and_dynamic_rules_the_effective_tags() {
        let id = crate::WindowHandle(3);
        let mut map = RuleMetadataMap::new();
        map.apply_static(id, &resolution(&["work"], &["editors"]));
        assert_eq!(map.tags(id), map.base_tags(id));
        map.apply_dynamic(id, &resolution(&["work", "urgent"], &["urgent titles"]));
        assert!(map.tags(id).contains("urgent"));
        assert!(!map.base_tags(id).contains("urgent"));
        assert_eq!(map.matched_rules(id), ["editors", "urgent titles"]);
        map.forget(id);
        assert!(map.tags(id).is_empty() && map.matched_rules(id).is_empty());
    }

    #[test]
    fn tag_commands_change_base_and_effective_tags() {
        let id = crate::WindowHandle(3);
        let mut map = RuleMetadataMap::new();
        assert_eq!(
            map.add_tag(id, "  Work "),
            Ok(true),
            "tags are trimmed and lower-cased"
        );
        assert_eq!(map.add_tag(id, "work"), Ok(false));
        assert!(map.add_tag(id, "   ").is_err());
        assert_eq!(map.toggle_tag(id, "music"), Ok(true));
        assert_eq!(map.toggle_tag(id, "MUSIC"), Ok(false));
        assert!(map.remove_tag(id, "work") && !map.remove_tag(id, "work"));
        map.add_tag(id, "a").unwrap();
        map.add_tag(id, "b").unwrap();
        assert_eq!(map.clear_tags(id), 2);
        assert!(map.tags(id).is_empty());
        assert_eq!(map.clear_tags(crate::WindowHandle(99)), 0);
    }
}
