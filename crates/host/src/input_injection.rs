//! Input made up for a test: keys, pointer moves, clicks, drags and wheel
//! steps that the window manager takes exactly as if a device had sent
//! them, so bindings, focus rules and drags can be checked without a
//! person at the keyboard.
//!
//! A window manager only accepts these when it was started to (`gevrek-wl
//! --test-input`); a session a person works in never does.

use serde::{Deserialize, Serialize};

/// One step of made-up input. Positions are logical pixels in the space of
/// all outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum InjectedInput {
    /// Press and release a chord: `Super+Shift+T`, `Ctrl+Alt+F11`, `F1`.
    /// The modifiers go down, the key is pressed and released, the
    /// modifiers go up.
    Chord { chord: String },
    /// One key down or up, by the name bindings use (`T`, `ENTER`, `LEFT`)
    /// or a modifier (`Super`, `Ctrl`, `Alt`, `Shift`).
    Key { key: String, pressed: bool },
    /// Press and release a key by its place on the keyboard (the XKB
    /// keycode: the evdev code plus 8; the key that types `i` on a US
    /// keyboard is 31), with the named modifiers held. For checks of
    /// layouts, where a key's name depends on the layout.
    Keycode {
        code: u32,
        #[serde(default)]
        hold: Vec<String>,
    },
    /// Move the pointer.
    Move { x: i32, y: i32 },
    /// A mouse button down or up: `left`, `right`, `middle`.
    Button { button: String, pressed: bool },
    /// Move there, press and release.
    Click {
        x: i32,
        y: i32,
        #[serde(default = "left")]
        button: String,
    },
    /// Press at `from`, move to `to` in steps, release; with the named
    /// modifiers held meanwhile.
    Drag {
        from: (i32, i32),
        to: (i32, i32),
        #[serde(default = "left")]
        button: String,
        #[serde(default)]
        hold: Vec<String>,
    },
    /// One step of the wheel, with the named modifiers held.
    Wheel {
        up: bool,
        #[serde(default)]
        hold: Vec<String>,
    },
    /// A finger on a touch screen: `phase` is `down` (at `x`, `y`),
    /// `move` (to there), `up` or `cancel`. `finger` tells one finger
    /// from another while several are down.
    Touch {
        #[serde(default)]
        finger: u32,
        phase: String,
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
    },
    /// Two fingers on a touchpad that move apart or together, told to
    /// the program under the pointer: `phase` is `begin`, `update` (with
    /// `scale`: how far apart the fingers are now, 1 being where they
    /// began) or `end`.
    Pinch {
        phase: String,
        #[serde(default = "one")]
        scale: f64,
    },
    /// Wait, so that what was started (an animation, a program) can go on.
    Wait { ms: u64 },
    /// Plug in another output of this size (a window manager without a
    /// display only: its outputs exist in memory).
    PlugOutput { width: i32, height: i32 },
    /// Unplug an output by its name (`HEADLESS-2`).
    UnplugOutput { name: String },
}

fn left() -> String {
    "left".to_string()
}

/// The modifiers and the key of a chord written as `Super+Shift+T`.
/// Modifier names are not case sensitive; `Win`, `Meta` and `Mod` are
/// Super, `Control` is Ctrl.
pub fn split_chord(chord: &str) -> (Vec<&'static str>, Option<String>) {
    let mut modifiers = Vec::new();
    let mut key = None;
    for part in chord
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match modifier_name(part) {
            Some(modifier) => modifiers.push(modifier),
            None => key = Some(part.to_string()),
        }
    }
    (modifiers, key)
}

/// The one spelling of a modifier, or `None` for another key.
pub fn modifier_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "super" | "win" | "meta" | "mod" | "logo" => Some("Super"),
        "ctrl" | "control" => Some("Ctrl"),
        "alt" => Some("Alt"),
        "shift" => Some("Shift"),
        _ => None,
    }
}

fn one() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_is_its_modifiers_and_one_key() {
        assert_eq!(
            split_chord("Super+Shift+T"),
            (vec!["Super", "Shift"], Some("T".to_string()))
        );
        assert_eq!(
            split_chord("ctrl + alt + F11"),
            (vec!["Ctrl", "Alt"], Some("F11".to_string()))
        );
        assert_eq!(split_chord("F1"), (vec![], Some("F1".to_string())));
        assert_eq!(split_chord("Win"), (vec!["Super"], None));
    }

    #[test]
    fn steps_are_written_as_tagged_objects() {
        let steps: Vec<InjectedInput> = serde_json::from_str(
            r#"[{"input":"chord","chord":"Super+T"},
                {"input":"click","x":10,"y":20},
                {"input":"drag","from":[1,2],"to":[3,4],"hold":["alt"]},
                {"input":"wheel","up":true}]"#,
        )
        .unwrap();
        assert_eq!(steps.len(), 4);
        assert_eq!(
            steps[1],
            InjectedInput::Click {
                x: 10,
                y: 20,
                button: "left".to_string()
            }
        );
    }
}
