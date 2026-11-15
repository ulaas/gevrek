//! The shortcuts applications ask the desktop for (the GlobalShortcuts
//! portal), as bindings of gevrek's own.
//!
//! An application names what it would have keys for ("push to talk")
//! and may say which keys it would like. Allowed by the user, each
//! becomes two bindings in the configuration (the keys going down, and
//! being let go) whose command is `app_shortcut`: they are listed, bound
//! to other keys or taken out where every binding is, and they are
//! still there the next time the application runs. A shortcut whose
//! keys another binding has, or that names none, is not bound.

use gevrek_model::command::DispatchCommand;
use gevrek_model::settings::{BindingDevice, BindingProfile, BindingTrigger};

/// A shortcut an application asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The application's own name for it.
    pub id: String,
    /// What it does, in words for the user.
    pub description: String,
    /// The keys the application would like (`CTRL+ALT+t`), if it says.
    pub trigger: Option<String>,
}

/// Keys with what is held with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Chord {
    /// As it is read: `Ctrl+Alt+T`.
    pub fn words(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (held, name) in [
            (self.win, "Super"),
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ] {
            if held {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    fn of(binding: &BindingProfile) -> Self {
        Self {
            key: binding.normalized_key(),
            ctrl: binding.ctrl,
            alt: binding.alt,
            shift: binding.shift,
            win: binding.win,
        }
    }
}

/// The name gevrek's bindings have for a key the applications name as
/// XKB does (`Return`, `space`, `Page_Up`, `a`, `F5`).
fn key_of(name: &str) -> Option<String> {
    let known = [
        ("return", "RETURN"),
        ("space", "SPACE"),
        ("tab", "TAB"),
        ("escape", "ESCAPE"),
        ("backspace", "BACKSPACE"),
        ("delete", "DELETE"),
        ("insert", "INSERT"),
        ("home", "HOME"),
        ("end", "END"),
        ("page_up", "PAGEUP"),
        ("prior", "PAGEUP"),
        ("page_down", "PAGEDOWN"),
        ("next", "PAGEDOWN"),
        ("up", "UP"),
        ("down", "DOWN"),
        ("left", "LEFT"),
        ("right", "RIGHT"),
        ("minus", "MINUS"),
        ("equal", "EQUALS"),
        ("comma", "COMMA"),
        ("period", "PERIOD"),
        ("semicolon", "SEMICOLON"),
        ("slash", "SLASH"),
        ("backslash", "BACKSLASH"),
        ("apostrophe", "QUOTE"),
        ("grave", "BACKTICK"),
        ("bracketleft", "LEFTBRACKET"),
        ("bracketright", "RIGHTBRACKET"),
    ];
    let lower = name.to_ascii_lowercase();
    if let Some((_, key)) = known.iter().find(|(xkb, _)| *xkb == lower) {
        return Some((*key).to_string());
    }
    let mut letters = lower.chars();
    match (letters.next(), letters.as_str()) {
        // A letter or a digit.
        (Some(one), "") if one.is_ascii_alphanumeric() => {
            Some(one.to_ascii_uppercase().to_string())
        }
        // F1 to F24.
        (Some('f'), number) if number.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            Some(lower.to_ascii_uppercase())
        }
        _ => None,
    }
}

/// The keys an application names, in the way the portal has them
/// written: what is held (`CTRL`, `ALT`, `SHIFT`, `LOGO`) and a key,
/// joined by `+`. `None` where that is not said so that gevrek can bind
/// it (a key it has no name for, no key at all, two keys).
pub fn chord_of(trigger: &str) -> Option<Chord> {
    let mut chord = Chord {
        key: String::new(),
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
    };
    for part in trigger.split('+').map(str::trim) {
        match part.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => chord.ctrl = true,
            "ALT" => chord.alt = true,
            "SHIFT" => chord.shift = true,
            "LOGO" | "SUPER" | "META" => chord.win = true,
            // (Num Lock being on is no part of a binding here.)
            "NUM" => {}
            _ if chord.key.is_empty() => chord.key = key_of(part)?,
            _ => return None,
        }
    }
    (!chord.key.is_empty()).then_some(chord)
}

/// The shortcuts of an application that are bound: its name for each,
/// what it does, and the keys as they are read.
pub fn bound(bindings: &[BindingProfile], app: &str) -> Vec<(String, String, String)> {
    bindings
        .iter()
        .filter_map(|binding| match &binding.command {
            DispatchCommand::AppShortcut {
                app: of,
                id,
                released: false,
            } if of == app && binding.enabled => {
                Some((id.clone(), binding.name.clone(), Chord::of(binding).words()))
            }
            _ => None,
        })
        .collect()
}

/// What is to become of a shortcut that is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// It is bound already (to these keys).
    Bound(String),
    /// It is bound to these keys, if the user allows.
    Bind(Chord),
    /// It is not bound: why.
    Not(&'static str),
}

/// The shortcuts that are asked for, each with what is to become of it.
pub type Planned = Vec<(Wanted, Plan)>;

/// What is to become of each shortcut an application asks for, with
/// the bindings there are.
pub fn plan(bindings: &[BindingProfile], app: &str, wanted: &[Wanted]) -> Planned {
    let have = bound(bindings, app);
    let mut taken: Vec<Chord> = bindings
        .iter()
        .filter(|binding| binding.enabled && binding.device == BindingDevice::Keyboard)
        .map(Chord::of)
        .collect();
    wanted
        .iter()
        .map(|wanted| {
            if let Some((_, _, keys)) = have.iter().find(|(id, ..)| *id == wanted.id) {
                return (wanted.clone(), Plan::Bound(keys.clone()));
            }
            let plan = match wanted.trigger.as_deref().map(chord_of) {
                None => Plan::Not("it names none"),
                Some(None) => Plan::Not("gevrek cannot bind the ones it names"),
                Some(Some(chord)) if taken.contains(&chord) => {
                    Plan::Not("its keys are another binding's")
                }
                Some(Some(chord)) => {
                    taken.push(chord.clone());
                    Plan::Bind(chord)
                }
            };
            (wanted.clone(), plan)
        })
        .collect()
}

/// The two bindings of a shortcut: its keys going down, and let go.
pub fn bindings_of(app: &str, wanted: &Wanted, chord: &Chord) -> [BindingProfile; 2] {
    let binding = |released: bool| BindingProfile {
        name: if wanted.description.trim().is_empty() {
            wanted.id.clone()
        } else {
            wanted.description.clone()
        },
        key: chord.key.clone(),
        ctrl: chord.ctrl,
        alt: chord.alt,
        shift: chord.shift,
        win: chord.win,
        trigger: if released {
            BindingTrigger::Release
        } else {
            BindingTrigger::Press
        },
        command: DispatchCommand::AppShortcut {
            app: app.to_string(),
            id: wanted.id.clone(),
            released,
        },
        ..BindingProfile::default()
    };
    [binding(false), binding(true)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(id: &str, trigger: Option<&str>) -> Wanted {
        Wanted {
            id: id.to_string(),
            description: format!("does {id}"),
            trigger: trigger.map(str::to_string),
        }
    }

    #[test]
    fn the_keys_an_application_names_are_read() {
        let chord = chord_of("CTRL+ALT+t").expect("keys");
        assert_eq!(chord.words(), "Ctrl+Alt+T");
        assert_eq!(
            chord_of("LOGO+SHIFT+F5").expect("keys").words(),
            "Super+Shift+F5"
        );
        assert_eq!(chord_of("Page_Up").expect("a key alone").words(), "PAGEUP");
        assert_eq!(chord_of("ctrl+space").expect("keys").words(), "Ctrl+SPACE");
        assert_eq!(chord_of("NUM+CTRL+1").expect("keys").words(), "Ctrl+1");
        // No key, a key gevrek has no name for, two keys.
        assert_eq!(chord_of("CTRL+ALT"), None);
        assert_eq!(chord_of("CTRL+XF86AudioPlay"), None);
        assert_eq!(chord_of("a+b"), None);
        assert_eq!(chord_of(""), None);
        assert_eq!(chord_of("F25"), None);
    }

    #[test]
    fn a_shortcut_is_bound_once_and_not_to_another_bindings_keys() {
        let mut bindings = vec![BindingProfile {
            name: "Terminal".to_string(),
            key: "T".to_string(),
            win: true,
            ..BindingProfile::default()
        }];
        let asked = [
            wanted("talk", Some("CTRL+ALT+t")),
            wanted("terminal", Some("LOGO+t")),
            wanted("again", Some("CTRL+ALT+T")),
            wanted("free", None),
            wanted("odd", Some("CTRL+XF86Launch1")),
        ];
        let planned = plan(&bindings, "org.voice", &asked);
        assert!(matches!(&planned[0].1, Plan::Bind(chord) if chord.words() == "Ctrl+Alt+T"));
        assert_eq!(planned[1].1, Plan::Not("its keys are another binding's"));
        // (The keys the first of them just got are taken too.)
        assert_eq!(planned[2].1, Plan::Not("its keys are another binding's"));
        assert!(matches!(planned[3].1, Plan::Not(_)) && matches!(planned[4].1, Plan::Not(_)));
        // Bound: two bindings, down and let go; the next time it is known.
        let Plan::Bind(chord) = &planned[0].1 else {
            unreachable!()
        };
        let made = bindings_of("org.voice", &asked[0], chord);
        assert_eq!(made[0].trigger, BindingTrigger::Press);
        assert_eq!(made[1].trigger, BindingTrigger::Release);
        assert_eq!(made[0].name, "does talk");
        assert!((made[0].ctrl, made[0].alt, made[0].win) == (true, true, false));
        bindings.extend(made);
        assert_eq!(
            bound(&bindings, "org.voice"),
            [(
                "talk".to_string(),
                "does talk".to_string(),
                "Ctrl+Alt+T".to_string()
            )]
        );
        assert!(bound(&bindings, "org.other").is_empty());
        let again = plan(&bindings, "org.voice", &asked[..1]);
        assert_eq!(again[0].1, Plan::Bound("Ctrl+Alt+T".to_string()));
        // Another application that asks for the same keys does not get them.
        let other = plan(&bindings, "org.other", &asked[..1]);
        assert_eq!(other[0].1, Plan::Not("its keys are another binding's"));
    }
}
