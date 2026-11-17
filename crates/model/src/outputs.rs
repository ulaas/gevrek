//! The displays (`desktop.outputs`): where each one is, its mode, scale
//! and which one is the primary.
//!
//! An output is named by its connector (`DP-3`, `HDMI-A-1`, `eDP-1`;
//! `gevrek state` shows the names). One that has no entry is placed to the
//! right of the others at its preferred mode.
//!
//! Where gevrek owns the displays (Linux, as the compositor) everything
//! here is applied. Where the system owns them (Windows) the arrangement,
//! the mode and the scale are applied through the system (a scale only
//! where the display offers it: Windows has its own steps).
//!
//! On Windows today only `enabled` is read: a display that is switched off
//! stays part of the Windows desktop, and gevrek leaves it alone (no
//! workspaces on it, the windows on it are not managed). Its name there is
//! the one `gevrek state` shows (`Monitor ACM2702`), or its device path.

use serde::{Deserialize, Serialize};

/// A resolution and refresh rate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OutputMode {
    pub width: u32,
    pub height: u32,
    /// In hertz (`143.973`); absent or 0: the highest the display has at
    /// this size.
    #[serde(default)]
    pub refresh: f64,
}

/// How the picture is turned.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputTransform {
    #[default]
    Normal,
    /// Turned a quarter to the right (a display standing on its side).
    Right,
    UpsideDown,
    Left,
}

/// One display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct OutputProfile {
    /// The connector the display is on.
    pub name: String,
    /// Off: the display is not used.
    pub enabled: bool,
    /// The top left corner among all displays, in logical pixels. Absent:
    /// to the right of the others.
    pub position: Option<(i32, i32)>,
    /// Absent: the display's preferred mode.
    pub mode: Option<OutputMode>,
    /// Device pixels per logical pixel (1, 1.25, 1.5, 2). Absent: 1, or
    /// what `platform.wayland.output_scales` says.
    pub scale: Option<f64>,
    pub transform: OutputTransform,
    /// The refresh rate follows the program that fills the display, where
    /// the display can.
    pub variable_refresh: bool,
    /// The one new windows and the first workspace go to. Without one,
    /// the first display is.
    pub primary: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for OutputProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            position: None,
            mode: None,
            scale: None,
            transform: OutputTransform::Normal,
            variable_refresh: false,
            primary: false,
            extra: Default::default(),
        }
    }
}

/// The entry of an output by its connector name (names are compared
/// without regard to case).
pub fn profile_of<'a>(profiles: &'a [OutputProfile], name: &str) -> Option<&'a OutputProfile> {
    profiles
        .iter()
        .find(|profile| profile.name.trim().eq_ignore_ascii_case(name.trim()))
}

/// Where each output goes: those with a position there, the others side by
/// side to the right of them in the order given. `outputs` are
/// `(name, logical width, logical height)`; the result has the same order.
pub fn arrange(profiles: &[OutputProfile], outputs: &[(String, i32, i32)]) -> Vec<(i32, i32)> {
    let placed: Vec<Option<(i32, i32)>> = outputs
        .iter()
        .map(|(name, _, _)| profile_of(profiles, name).and_then(|profile| profile.position))
        .collect();
    // The others start where the placed ones end.
    let mut next_x = outputs
        .iter()
        .zip(&placed)
        .filter_map(|((_, width, _), place)| place.map(|(x, _)| x.saturating_add(*width)))
        .max()
        .unwrap_or(0);
    placed
        .into_iter()
        .zip(outputs)
        .map(|(place, (_, width, _))| {
            place.unwrap_or_else(|| {
                let at = (next_x, 0);
                next_x = next_x.saturating_add(*width);
                at
            })
        })
        .collect()
}

/// Whether going from one list to the other can leave a display dark or
/// unreadable: a display's mode, its being on or off, or how its picture
/// is turned changed. Such a change is applied on trial and taken back
/// unless it is kept; a new place or scale is harmless and simply applied.
pub fn needs_trial(old: &[OutputProfile], new: &[OutputProfile]) -> bool {
    let risky = |profiles: &[OutputProfile], name: &str| {
        profile_of(profiles, name)
            .map(|profile| (profile.enabled, profile.mode, profile.transform))
            .unwrap_or((true, None, OutputTransform::Normal))
    };
    old.iter()
        .chain(new)
        .any(|profile| risky(old, &profile.name) != risky(new, &profile.name))
}

/// What is wrong with the entries, as `(path, message)`.
pub fn problems(profiles: &[OutputProfile]) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut primaries = 0;
    for (index, profile) in profiles.iter().enumerate() {
        let path = format!("desktop.outputs[{index}]");
        let name = profile.name.trim().to_ascii_lowercase();
        if name.is_empty() {
            problems.push((
                format!("{path}.name"),
                "An output is named by its connector, such as DP-3".to_string(),
            ));
        } else if !seen.insert(name) {
            problems.push((
                format!("{path}.name"),
                "This output has an entry already".to_string(),
            ));
        }
        if profile
            .scale
            .is_some_and(|scale| !(0.5..=4.0).contains(&scale))
        {
            problems.push((
                format!("{path}.scale"),
                "The scale is between 0.5 and 4".to_string(),
            ));
        }
        if let Some(mode) = profile.mode {
            if mode.width == 0 || mode.height == 0 || !(0.0..=1000.0).contains(&mode.refresh) {
                problems.push((
                    format!("{path}.mode"),
                    "A mode is a width, a height and a refresh rate in hertz".to_string(),
                ));
            }
        }
        if profile.primary && profile.enabled {
            primaries += 1;
        }
    }
    if primaries > 1 {
        problems.push((
            "desktop.outputs".to_string(),
            "Only one output can be the primary".to_string(),
        ));
    }
    problems
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_new_mode_is_tried_and_a_new_place_is_not() {
        let entry =
            |text: &str| -> Vec<OutputProfile> { serde_json::from_str(text).expect("outputs") };
        let plain = entry(r#"[{"name": "DP-1"}]"#);
        assert!(!needs_trial(&plain, &plain));
        assert!(!needs_trial(&[], &plain));
        assert!(!needs_trial(
            &plain,
            &entry(r#"[{"name": "DP-1", "position": [10, 0], "scale": 1.5, "primary": true}]"#)
        ));
        assert!(needs_trial(
            &plain,
            &entry(r#"[{"name": "dp-1", "mode": {"width": 1920, "height": 1080}}]"#)
        ));
        assert!(needs_trial(
            &plain,
            &entry(r#"[{"name": "DP-1", "enabled": false}]"#)
        ));
        assert!(needs_trial(
            &plain,
            &entry(r#"[{"name": "DP-1", "transform": "right"}]"#)
        ));
        // Taking an entry away that switched a display off switches it on.
        assert!(needs_trial(
            &entry(r#"[{"name": "DP-2", "enabled": false}]"#),
            &[]
        ));
    }

    use super::*;

    fn named(name: &str) -> OutputProfile {
        OutputProfile {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn outputs_without_a_place_follow_those_that_have_one() {
        let outputs = vec![
            ("DP-1".to_string(), 1920, 1080),
            ("DP-2".to_string(), 2560, 1440),
            ("HDMI-A-1".to_string(), 1280, 720),
        ];
        // Nothing configured: side by side in order.
        assert_eq!(arrange(&[], &outputs), [(0, 0), (1920, 0), (4480, 0)]);
        // The second one is put first, and above.
        let profiles = vec![OutputProfile {
            position: Some((0, -1440)),
            ..named("dp-2")
        }];
        assert_eq!(
            arrange(&profiles, &outputs),
            [(2560, 0), (0, -1440), (4480, 0)]
        );
    }

    #[test]
    fn entries_that_cannot_be_right_are_reported() {
        assert!(problems(&[named("DP-1"), named("DP-2")]).is_empty());
        let bad = vec![
            named("DP-1"),
            OutputProfile {
                scale: Some(9.0),
                primary: true,
                ..named("dp-1")
            },
            OutputProfile {
                primary: true,
                ..named("")
            },
        ];
        let paths: Vec<String> = problems(&bad).into_iter().map(|(path, _)| path).collect();
        assert_eq!(paths.len(), 4, "{paths:?}");
    }
}
