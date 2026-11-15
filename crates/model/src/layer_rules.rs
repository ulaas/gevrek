//! Rules for the desktop's own surfaces and those of other shells: a
//! panel, a launcher, a notification, whatever a program shows on a layer
//! of the desktop instead of in a window. Such a surface says what it is
//! by a name (its namespace: `gevrek-panel`, `waybar`, `rofi`); a rule
//! names the surfaces it is for by that.

use serde::{Deserialize, Serialize};

/// What surfaces of a name are given.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LayerRule {
    /// The name of the surfaces it is for; a `*` stands for any letters
    /// (`gevrek-*`, `*bar`). Upper and lower case are the same.
    pub namespace: String,
    /// How solid the surface is drawn, 0 (not seen) to 255 (as it draws
    /// itself). Not set: as it draws itself.
    pub opacity: Option<u8>,
    /// Behind the surface the wallpaper is shown blurred (for a surface
    /// that is drawn less than solid: what shows through is soft, and
    /// what stands on the surface can be read). It is the wallpaper that
    /// is blurred, not windows that lie behind the surface: for a panel,
    /// which has its own room on the display.
    pub blur: bool,
    /// The corners of the surface are cut round, by so many pixels. Not
    /// set, or 0: as it draws itself.
    pub corner_radius: Option<u32>,
    /// A shadow is drawn under the surface: the one of the windows
    /// (`desktop.appearance.effects.shadow`: how far it reaches, where it
    /// lies, its colour and its strength under the focused window),
    /// whether the windows have theirs or not.
    pub shadow: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// Whether a name is one a pattern stands for.
pub fn matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();
    let name = name.to_lowercase();
    let mut parts = pattern.split('*');
    // Without a star: the whole name.
    let Some(first) = parts.next() else {
        return false;
    };
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        return !pattern.is_empty() && name == pattern;
    }
    let Some(mut left) = name.strip_prefix(first) else {
        return false;
    };
    // Every part between stars in its order, the last at the very end.
    let (last, between) = rest.split_last().expect("not empty");
    for part in between {
        match left.find(part) {
            Some(at) => left = &left[at + part.len()..],
            None => return false,
        }
    }
    left.ends_with(last)
}

/// The rule for a surface of this name: the first that names it.
pub fn rule_for<'a>(rules: &'a [LayerRule], namespace: &str) -> Option<&'a LayerRule> {
    rules
        .iter()
        .find(|rule| matches(&rule.namespace, namespace))
}

/// Whether the wallpaper is shown blurred behind a surface of this name.
pub fn blur_of(rules: &[LayerRule], namespace: &str) -> bool {
    rule_for(rules, namespace).is_some_and(|rule| rule.blur)
}

/// By how many pixels the corners of a surface of this name are cut
/// round; 0: not at all.
pub fn corner_radius_of(rules: &[LayerRule], namespace: &str) -> u32 {
    rule_for(rules, namespace)
        .and_then(|rule| rule.corner_radius)
        .unwrap_or(0)
}

/// Whether a shadow is drawn under a surface of this name.
pub fn shadow_of(rules: &[LayerRule], namespace: &str) -> bool {
    rule_for(rules, namespace).is_some_and(|rule| rule.shadow)
}

/// How solid a surface of this name is drawn, from 0 to 1.
pub fn opacity_of(rules: &[LayerRule], namespace: &str) -> f32 {
    rule_for(rules, namespace)
        .and_then(|rule| rule.opacity)
        .map_or(1.0, |opacity| f32::from(opacity) / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_stands_for_any_letters() {
        assert!(matches("gevrek-panel", "gevrek-panel"));
        assert!(matches("Gevrek-Panel", "gevrek-panel"));
        assert!(!matches("gevrek-panel", "gevrek-panel-2"));
        assert!(matches("gevrek-*", "gevrek-osd"));
        assert!(matches("gevrek-*", "gevrek-"));
        assert!(!matches("gevrek-*", "waybar"));
        assert!(matches("*bar", "waybar"));
        assert!(!matches("*bar", "barley"));
        assert!(matches("*", "anything"));
        assert!(matches("*", ""));
        assert!(matches("w*b*r", "waybar"));
        assert!(!matches("w*b*r", "waybars"));
        assert!(matches("*notif*", "swaync-notification-window"));
        // A part is not used twice.
        assert!(!matches("ab*ab", "ab"));
        assert!(matches("ab*ab", "abab"));
        // No pattern names nothing.
        assert!(!matches("", "waybar"));
        assert!(!matches("  ", ""));
    }

    #[test]
    fn the_first_rule_that_names_a_surface_holds() {
        let rules: Vec<LayerRule> = serde_json::from_str(
            r#"[{"namespace": "gevrek-osd", "opacity": 128},
                {"namespace": "gevrek-*", "opacity": 230},
                {"namespace": "waybar"}]"#,
        )
        .expect("rules");
        assert!((opacity_of(&rules, "gevrek-osd") - 128.0 / 255.0).abs() < 1e-6);
        assert!((opacity_of(&rules, "gevrek-panel") - 230.0 / 255.0).abs() < 1e-6);
        // A rule that says nothing of it, and no rule: as it draws itself.
        assert_eq!(opacity_of(&rules, "waybar"), 1.0);
        assert_eq!(opacity_of(&rules, "rofi"), 1.0);
        assert_eq!(opacity_of(&[], "gevrek-panel"), 1.0);
        let blurred: Vec<LayerRule> = serde_json::from_str(
            r#"[{"namespace": "gevrek-panel", "opacity": 128, "blur": true}]"#,
        )
        .expect("rules");
        assert!(blur_of(&blurred, "Gevrek-Panel"));
        assert!(!blur_of(&blurred, "waybar") && !blur_of(&rules, "gevrek-osd"));
    }
}
