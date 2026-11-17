//! What the types do not say about a number: the range worth offering, the
//! step, the unit. By the name of the setting, which is how they are
//! consistent in the model (`*_ms` is always milliseconds).
//!
//! A range here is what a slider offers, not a limit: the model's own
//! validation decides what is accepted, and a value outside the range is
//! shown as it is. A range lies within the model's limits, though: a
//! slider does not offer what is then refused (the test below holds the
//! ones that have a limit against it).

/// How to offer a number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hint {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: &'static str,
}

const fn of(min: f64, max: f64, step: f64, unit: &'static str) -> Hint {
    Hint {
        min,
        max,
        step,
        unit,
    }
}

/// By the whole pointer, for the names that mean different things in
/// different places.
const PLACES: &[(&str, Hint)] = &[
    (
        "/desktop/animations/session/duration_ms",
        of(100.0, 3000.0, 20.0, "ms"),
    ),
    ("/desktop/shell/panel/height", of(20.0, 64.0, 1.0, "px")),
    ("/desktop/shell/panel/margin", of(0.0, 32.0, 1.0, "px")),
    ("/desktop/shell/panel/width", of(0.0, 3840.0, 10.0, "px")),
    ("/desktop/shell/panel/padding", of(0.0, 40.0, 1.0, "px")),
    ("/desktop/shell/glass/tint", of(0.0, 255.0, 5.0, "")),
    (
        "/desktop/shell/panel/title_width",
        of(0.0, 800.0, 10.0, "px"),
    ),
    (
        "/desktop/shell/panel/task_width",
        of(0.0, 400.0, 10.0, "px"),
    ),
    ("/desktop/shell/panel/spacing", of(0.0, 32.0, 1.0, "px")),
    (
        "/desktop/shell/panel/corner_radius",
        of(0.0, 32.0, 1.0, "px"),
    ),
    (
        "/desktop/platform/wayland/cursor_size",
        of(0.0, 96.0, 1.0, "px"),
    ),
    (
        "/desktop/platform/wayland/output_scale",
        of(0.5, 4.0, 0.05, "×"),
    ),
    ("/desktop/animations/fps", of(30.0, 240.0, 1.0, "fps")),
    // (A window nobody sees cannot be found again: not below 20.)
    (
        "/desktop/appearance/window_opacity",
        of(20.0, 255.0, 1.0, ""),
    ),
    (
        "/desktop/appearance/inactive_window_opacity",
        of(20.0, 255.0, 1.0, ""),
    ),
    (
        "/desktop/appearance/stackbar/height",
        of(10.0, 96.0, 1.0, "px"),
    ),
    (
        "/desktop/appearance/invisible_borders/size",
        of(1.0, 96.0, 1.0, "px"),
    ),
    // (Parts of one, not of 255, and no pixels.)
    (
        "/desktop/zones/behavior/snap_threshold",
        of(0.0, 1.0, 0.01, ""),
    ),
    (
        "/desktop/zones/behavior/preview_opacity",
        of(0.0, 1.0, 0.01, ""),
    ),
    ("/desktop/hot_corners/size", of(1.0, 64.0, 1.0, "px")),
    ("/desktop/hot_corners/delay_ms", of(0.0, 2000.0, 10.0, "ms")),
    ("/desktop/focus/history_size", of(1.0, 256.0, 1.0, "")),
    ("/desktop/schema_version", of(0.0, 1000.0, 1.0, "")),
    ("/version", of(0.0, 1000.0, 1.0, "")),
];

/// By the last part of the name.
const NAMES: &[(&str, Hint)] = &[
    ("duration_ms", of(16.0, 2000.0, 10.0, "ms")),
    ("long_press_ms", of(100.0, 5000.0, 10.0, "ms")),
    ("focus_delay_ms", of(0.0, 1000.0, 10.0, "ms")),
    ("idle_timeout_ms", of(50.0, 2000.0, 10.0, "ms")),
    ("repeat_delay_ms", of(100.0, 2000.0, 10.0, "ms")),
    ("repeat_rate", of(1.0, 100.0, 1.0, "/s")),
    ("spin_seconds", of(0.0, 60.0, 0.5, "s")),
    ("lock_after_minutes", of(0.0, 120.0, 1.0, "min")),
    ("displays_off_after_minutes", of(0.0, 120.0, 1.0, "min")),
    ("every_minutes", of(1.0, 1440.0, 1.0, "min")),
    ("angle", of(0.0, 360.0, 1.0, "°")),
    ("border_width", of(1.0, 32.0, 1.0, "px")),
    ("border_offset", of(-16.0, 16.0, 1.0, "px")),
    ("corner_radius", of(0.0, 48.0, 1.0, "px")),
    ("radius", of(0.0, 96.0, 1.0, "px")),
    ("glow_radius", of(0.0, 96.0, 1.0, "px")),
    ("shown", of(1.0, 10.0, 1.0, "")),
    ("temperature", of(1000.0, 6500.0, 100.0, "K")),
    ("stay_seconds", of(1.0, 60.0, 1.0, "s")),
    ("dim_before_lock_seconds", of(0.0, 120.0, 1.0, "s")),
    ("size", of(0.0, 96.0, 1.0, "px")),
    ("offset_x", of(-64.0, 64.0, 1.0, "px")),
    ("offset_y", of(-64.0, 64.0, 1.0, "px")),
    ("height", of(0.0, 96.0, 1.0, "px")),
    ("margin", of(0.0, 64.0, 1.0, "px")),
    ("left", of(0.0, 400.0, 1.0, "px")),
    ("top", of(0.0, 400.0, 1.0, "px")),
    ("right", of(0.0, 400.0, 1.0, "px")),
    ("bottom", of(0.0, 400.0, 1.0, "px")),
    ("gap", of(0.0, 64.0, 1.0, "px")),
    ("padding", of(0.0, 64.0, 1.0, "px")),
    ("snap_threshold", of(0.0, 1.0, 0.01, "")),
    ("axis_lock_ratio", of(1.0, 3.0, 0.05, "")),
    ("distance_percent", of(2.0, 100.0, 1.0, "%")),
    ("speed", of(-1.0, 1.0, 0.05, "")),
    ("scale", of(0.5, 4.0, 0.05, "×")),
    ("ratio", of(0.05, 0.95, 0.01, "")),
    ("fingers", of(2.0, 5.0, 1.0, "")),
    ("scroll_fingers", of(0.0, 5.0, 1.0, "")),
];

/// Names that end like this are a part of 255 (how see-through, how strong).
const BYTE_ENDINGS: &[&str] = &["opacity", "alpha"];

/// The settings whose change cannot take effect while the session runs
/// (the plan's principle 7: every other one does, at once): when it
/// holds, in a word or two for beside the setting's name, and why.
const WAITS: &[(&str, &str, &str)] = &[
    (
        "/desktop/platform/wayland/xwayland",
        "next session",
        "Holds from the next start of the session: X11 programs run on Xwayland, and \
         ending it would end every one of them.",
    ),
    (
        "/desktop/session/environment",
        "new programs",
        "For programs started from now on: one that runs already keeps the variables it \
         was started with.",
    ),
    (
        "/desktop/manage_existing_on_startup",
        "at the start",
        "Looked at once, when gevrek starts: the windows that are there then.",
    ),
    (
        "/desktop/restore_session_on_startup",
        "at the start",
        "Looked at once, when gevrek starts.",
    ),
];

/// When a change of the setting at `pointer` holds, if not at once: a
/// word or two, and the reason. `None`: it takes effect while the
/// session runs, as every setting does that can.
pub fn waits(pointer: &str) -> Option<(&'static str, &'static str)> {
    WAITS
        .iter()
        .find(|(place, _, _)| *place == pointer)
        .map(|(_, when, why)| (*when, *why))
}

/// How to offer the number at `pointer`, when its name says.
pub fn hint(pointer: &str) -> Option<Hint> {
    if let Some((_, hint)) = PLACES.iter().find(|(place, _)| *place == pointer) {
        return Some(*hint);
    }
    let name = pointer.rsplit('/').next().unwrap_or(pointer);
    if let Some((_, hint)) = NAMES.iter().find(|(known, _)| *known == name) {
        return Some(*hint);
    }
    if BYTE_ENDINGS.iter().any(|ending| name.ends_with(ending)) {
        return Some(of(0.0, 255.0, 1.0, ""));
    }
    if name.ends_with("_ms") {
        return Some(of(0.0, 5000.0, 10.0, "ms"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_that_wait_are_settings_and_say_why() {
        for (place, when, why) in WAITS {
            assert!(
                crate::schema().at(place).is_some(),
                "{place} is no setting of the schema"
            );
            assert!(!when.is_empty() && why.len() > 20, "{place}");
            assert_eq!(waits(place), Some((*when, *why)));
        }
        assert_eq!(waits("/desktop/animations/enabled"), None);
    }

    #[test]
    fn a_slider_offers_nothing_the_model_refuses() {
        // The settings the model limits (`DesktopConfig::validate` and
        // the parts' own), each with what it accepts.
        let limits: &[(&str, f64, f64)] = &[
            ("/desktop/appearance/border_width", 1.0, 32.0),
            ("/desktop/workspaces/0/border_width", 1.0, 32.0),
            ("/desktop/appearance/window_opacity", 20.0, 255.0),
            ("/desktop/appearance/inactive_window_opacity", 20.0, 255.0),
            ("/desktop/appearance/stackbar/height", 10.0, 200.0),
            ("/desktop/appearance/invisible_borders/size", 1.0, 200.0),
            ("/desktop/appearance/shadow/radius", 0.0, 200.0),
            ("/desktop/appearance/shadow/offset_x", -200.0, 200.0),
            ("/desktop/appearance/glow/radius", 0.0, 200.0),
            ("/desktop/animations/fps", 30.0, 240.0),
            ("/desktop/animations/workspace/duration_ms", 16.0, 5000.0),
            ("/desktop/animations/session/duration_ms", 0.0, 5000.0),
            ("/desktop/zones/behavior/snap_threshold", 0.0, 1.0),
            ("/desktop/zones/behavior/preview_opacity", 0.0, 1.0),
            ("/desktop/zones/default_layout/zones/0/gap", 0.0, 500.0),
            ("/desktop/bindings/0/long_press_ms", 100.0, 10000.0),
            ("/desktop/gestures/idle_timeout_ms", 50.0, 5000.0),
            ("/desktop/gestures/axis_lock_ratio", 1.0, 3.0),
            ("/desktop/gestures/profiles/0/fingers", 2.0, 5.0),
            ("/desktop/gestures/profiles/0/distance_percent", 2.0, 100.0),
            ("/desktop/hot_corners/size", 1.0, 64.0),
            ("/desktop/hot_corners/delay_ms", 0.0, 5000.0),
            ("/desktop/input/keyboard/repeat_delay_ms", 100.0, 2000.0),
            ("/desktop/input/touchpad/speed", -1.0, 1.0),
            ("/desktop/outputs/0/scale", 0.5, 4.0),
            ("/desktop/focus/history_size", 1.0, 256.0),
            ("/desktop/shell/notifications/shown", 1.0, 10.0),
            ("/desktop/shell/notifications/stay_seconds", 1.0, 60.0),
        ];
        for (place, least, most) in limits {
            let offered = hint(place).unwrap_or_else(|| panic!("{place} has no range"));
            assert!(
                offered.min >= *least && offered.max <= *most,
                "{place} is offered from {} to {}; the model takes {least} to {most}",
                offered.min,
                offered.max
            );
        }
    }
}
