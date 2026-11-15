//! Keyboards, mice and touchpads (`desktop.input`).
//!
//! Where gevrek owns the input devices (it is the compositor: Linux) these
//! settings are applied. Where the system owns them (Windows, macOS) they
//! are kept in the file and the system's own settings apply; the backend
//! says so through its capabilities.

use serde::{Deserialize, Serialize};

/// `desktop.input`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct InputConfig {
    pub keyboard: KeyboardConfig,
    /// Mice and other pointing devices that are not touchpads.
    pub mouse: PointerConfig,
    pub touchpad: TouchpadConfig,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// One keyboard layout: `us`, or `tr` with the variant `f`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct KeyboardLayout {
    /// The XKB layout name (`us`, `tr`, `de`).
    pub layout: String,
    /// The variant of it, or empty for the plain one.
    pub variant: String,
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// The keys that go to the next keyboard layout.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LayoutSwitchKeys {
    /// Alt and Shift together.
    #[default]
    AltShift,
    /// Super and Space.
    SuperSpace,
    /// Ctrl and Shift together.
    CtrlShift,
    /// Alt and Space.
    AltSpace,
    /// Caps Lock (which then no longer locks capitals).
    CapsLock,
    /// No keys: the layout is changed from the panel or by a binding.
    None,
}

impl LayoutSwitchKeys {
    /// The XKB option that makes these keys switch the layout.
    pub fn xkb_option(self) -> Option<&'static str> {
        match self {
            Self::AltShift => Some("grp:alt_shift_toggle"),
            Self::SuperSpace => Some("grp:win_space_toggle"),
            Self::CtrlShift => Some("grp:ctrl_shift_toggle"),
            Self::AltSpace => Some("grp:alt_space_toggle"),
            Self::CapsLock => Some("grp:caps_toggle"),
            Self::None => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct KeyboardConfig {
    /// The layouts to switch between, in order. Empty: what the system
    /// says (the `XKB_DEFAULT_*` environment, else `us`).
    pub layouts: Vec<KeyboardLayout>,
    /// The keys that go to the next layout, where there is more than
    /// one: Alt+Shift unless another is chosen.
    pub switch_with: LayoutSwitchKeys,
    /// XKB options, such as `caps:escape`. (An option that names keys
    /// for switching the layout itself, `grp:...`, is used in place of
    /// `switch_with`.)
    pub options: Vec<String>,
    /// The XKB keyboard model; empty for the default.
    pub model: String,
    /// How long a key is held before it repeats.
    pub repeat_delay_ms: u32,
    /// Repeats per second while it is held. 0: keys do not repeat.
    pub repeat_rate: u32,
    /// Num Lock is on when the session starts.
    pub numlock: bool,
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self {
            layouts: Vec::new(),
            switch_with: LayoutSwitchKeys::AltShift,
            options: Vec::new(),
            model: String::new(),
            repeat_delay_ms: 400,
            repeat_rate: 30,
            numlock: false,
            extra: Default::default(),
        }
    }
}

impl KeyboardConfig {
    /// The options as XKB gets them: the ones that are written, and,
    /// where there are several layouts and no option of them names keys
    /// for switching, the option of `switch_with`.
    pub fn xkb_options(&self) -> Vec<String> {
        let mut options: Vec<String> = self
            .options
            .iter()
            .map(|option| option.trim().to_string())
            .filter(|option| !option.is_empty())
            .collect();
        let named = options.iter().any(|option| option.starts_with("grp:"));
        if self.layouts.len() > 1 && !named {
            if let Some(option) = self.switch_with.xkb_option() {
                options.push(option.to_string());
            }
        }
        options
    }

    /// The layouts and variants as XKB wants them: two comma-separated
    /// lists of the same length (`us,tr` and `,f`).
    pub fn xkb_layouts(&self) -> (String, String) {
        let layouts: Vec<&str> = self
            .layouts
            .iter()
            .map(|entry| entry.layout.trim())
            .collect();
        let variants: Vec<&str> = self
            .layouts
            .iter()
            .map(|entry| entry.variant.trim())
            .collect();
        (layouts.join(","), variants.join(","))
    }
}

/// How pointer movement is turned into speed.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AccelProfile {
    /// The device's own default.
    #[default]
    Default,
    /// Faster movement goes disproportionately further.
    Adaptive,
    /// The same factor at every speed.
    Flat,
}

/// A mouse or other pointing device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PointerConfig {
    /// From -1 (slowest) to 1 (fastest); 0 is the device's normal speed.
    pub speed: f64,
    pub accel_profile: AccelProfile,
    /// Content follows the fingers (the wheel turns the other way).
    pub natural_scroll: bool,
    /// The buttons are swapped.
    pub left_handed: bool,
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// How a touchpad scrolls.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ScrollMethod {
    /// The device's own default.
    #[default]
    Default,
    TwoFinger,
    Edge,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TouchpadConfig {
    /// A tap is a click.
    pub tap: bool,
    /// Tap and hold drags.
    pub tap_drag: bool,
    pub natural_scroll: bool,
    /// The touchpad is off while keys are typed.
    pub disable_while_typing: bool,
    pub speed: f64,
    pub accel_profile: AccelProfile,
    pub scroll_method: ScrollMethod,
    pub left_handed: bool,
    /// Pressing left and right together is the middle button.
    pub middle_emulation: bool,
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        Self {
            tap: true,
            tap_drag: true,
            natural_scroll: true,
            disable_while_typing: true,
            speed: 0.0,
            accel_profile: AccelProfile::Default,
            scroll_method: ScrollMethod::Default,
            left_handed: false,
            middle_emulation: false,
            extra: Default::default(),
        }
    }
}

impl InputConfig {
    /// What is wrong with the settings, as `(path, message)`.
    pub fn problems(&self) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        for (index, entry) in self.keyboard.layouts.iter().enumerate() {
            let ok = |text: &str| {
                text.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '(' | ')'))
            };
            if entry.layout.trim().is_empty() || !ok(entry.layout.trim()) {
                problems.push((
                    format!("desktop.input.keyboard.layouts[{index}].layout"),
                    "A layout is an XKB name such as us, tr or de".to_string(),
                ));
            }
            if !ok(entry.variant.trim()) {
                problems.push((
                    format!("desktop.input.keyboard.layouts[{index}].variant"),
                    "A variant is an XKB name such as f or nodeadkeys, or empty".to_string(),
                ));
            }
        }
        for (index, option) in self.keyboard.options.iter().enumerate() {
            if option.trim().is_empty()
                || option.contains(',')
                || option.contains(char::is_whitespace)
            {
                problems.push((
                    format!("desktop.input.keyboard.options[{index}]"),
                    "One XKB option each, such as grp:alt_shift_toggle".to_string(),
                ));
            }
        }
        if !(100..=2000).contains(&self.keyboard.repeat_delay_ms) {
            problems.push((
                "desktop.input.keyboard.repeat_delay_ms".to_string(),
                "The repeat delay is between 100 and 2000 ms".to_string(),
            ));
        }
        if self.keyboard.repeat_rate > 200 {
            problems.push((
                "desktop.input.keyboard.repeat_rate".to_string(),
                "The repeat rate is at most 200 per second".to_string(),
            ));
        }
        for (path, speed) in [
            ("desktop.input.mouse.speed", self.mouse.speed),
            ("desktop.input.touchpad.speed", self.touchpad.speed),
        ] {
            if !(-1.0..=1.0).contains(&speed) {
                problems.push((
                    path.to_string(),
                    "The speed is between -1 and 1".to_string(),
                ));
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_are_handed_to_xkb_as_two_lists() {
        let keyboard = KeyboardConfig {
            layouts: vec![
                KeyboardLayout {
                    layout: "us".to_string(),
                    ..Default::default()
                },
                KeyboardLayout {
                    layout: "tr".to_string(),
                    variant: "f".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            keyboard.xkb_layouts(),
            ("us,tr".to_string(), ",f".to_string())
        );
        assert_eq!(
            KeyboardConfig::default().xkb_layouts(),
            (String::new(), String::new())
        );
    }

    #[test]
    fn what_cannot_be_right_is_reported() {
        assert!(InputConfig::default().problems().is_empty());
        let mut input = InputConfig::default();
        input.keyboard.layouts.push(KeyboardLayout {
            layout: "us; rm".to_string(),
            ..Default::default()
        });
        input.keyboard.options.push("a,b".to_string());
        input.keyboard.repeat_delay_ms = 5;
        input.mouse.speed = 3.0;
        let paths: Vec<String> = input.problems().into_iter().map(|(path, _)| path).collect();
        assert_eq!(paths.len(), 4, "{paths:?}");
    }

    #[test]
    fn a_file_names_only_what_it_changes() {
        let input: InputConfig = serde_json::from_str(
            r#"{"keyboard":{"layouts":[{"layout":"us"},{"layout":"tr"}],"options":["grp:alt_shift_toggle"],"numlock":true}}"#,
        )
        .unwrap();
        assert_eq!(input.keyboard.layouts.len(), 2);
        assert!(input.keyboard.numlock);
        assert_eq!(input.keyboard.repeat_rate, 30);
        assert!(input.touchpad.tap);
    }
}

#[cfg(test)]
mod switch_tests {
    use super::*;

    fn with(layouts: &[&str], options: &[&str], keys: LayoutSwitchKeys) -> Vec<String> {
        KeyboardConfig {
            layouts: layouts
                .iter()
                .map(|layout| KeyboardLayout {
                    layout: layout.to_string(),
                    variant: String::new(),
                    extra: Default::default(),
                })
                .collect(),
            options: options.iter().map(|option| option.to_string()).collect(),
            switch_with: keys,
            ..Default::default()
        }
        .xkb_options()
    }

    #[test]
    fn alt_and_shift_switch_the_layout_unless_told_otherwise() {
        assert_eq!(
            KeyboardConfig::default().switch_with,
            LayoutSwitchKeys::AltShift
        );
        assert_eq!(
            with(&["us", "tr"], &[], LayoutSwitchKeys::AltShift),
            ["grp:alt_shift_toggle"]
        );
        assert_eq!(
            with(
                &["us", "tr"],
                &["caps:escape"],
                LayoutSwitchKeys::SuperSpace
            ),
            ["caps:escape", "grp:win_space_toggle"]
        );
        // One layout: nothing to switch, no keys taken.
        assert!(with(&["us"], &[], LayoutSwitchKeys::AltShift).is_empty());
        assert!(with(&[], &[], LayoutSwitchKeys::AltShift).is_empty());
        // No keys asked for; or keys named by an option of the person's own.
        assert!(with(&["us", "tr"], &[], LayoutSwitchKeys::None).is_empty());
        assert_eq!(
            with(
                &["us", "tr"],
                &["grp:caps_toggle"],
                LayoutSwitchKeys::AltShift
            ),
            ["grp:caps_toggle"]
        );
        let read: KeyboardConfig =
            serde_json::from_str(r#"{"switch_with": "ctrl_shift"}"#).expect("read");
        assert_eq!(read.switch_with, LayoutSwitchKeys::CtrlShift);
    }
}
