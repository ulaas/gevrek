//! What is said between the greeter, the service and a session's worker:
//! one line of JSON each way.

use serde::{Deserialize, Serialize};

/// What the greeter asks of the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum Ask {
    /// Log this user in.
    Login { user: String, password: String },
    /// Switch the machine off, or start it again.
    Power { restart: bool },
    /// The greeter has drawn its first picture: a boot splash that was
    /// kept until now may end.
    Shown,
}

/// What the service answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    /// Why not, in words for the person at the machine.
    #[serde(default)]
    pub why: String,
}

impl Answer {
    pub fn yes() -> Self {
        Self {
            ok: true,
            why: String::new(),
        }
    }

    pub fn no(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            why: why.into(),
        }
    }
}

/// Whose session a worker opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The greeter's own: no password, a session of the class `greeter`.
    Greeter,
    /// A person's: the password is checked.
    User,
}

/// What the service tells a worker (on its standard input, so that the
/// password is in no list of processes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub kind: Kind,
    pub user: String,
    #[serde(default)]
    pub password: String,
    /// The virtual console the session is on.
    pub console: u32,
    /// What the session runs, given to the user's shell.
    pub command: String,
    /// More for the session's environment.
    #[serde(default)]
    pub environment: Vec<(String, String)>,
}

/// A user's name as an account can have it: letters, digits, `_`, `-`,
/// `.` and a `$` at the end, not beginning with `-`, not too long.
/// (Checked before anything is handed to PAM or a shell.)
pub fn is_a_user_name(name: &str) -> bool {
    let body = name.strip_suffix('$').unwrap_or(name);
    !body.is_empty()
        && name.len() <= 64
        && !body.starts_with('-')
        && body
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_greeter_asks_is_one_line_of_json() {
        let ask: Ask =
            serde_json::from_str(r#"{"ask": "login", "user": "ada", "password": "s3cret"}"#)
                .expect("a login");
        assert_eq!(
            ask,
            Ask::Login {
                user: "ada".to_string(),
                password: "s3cret".to_string()
            }
        );
        let power: Ask =
            serde_json::from_str(r#"{"ask": "power", "restart": true}"#).expect("power");
        assert_eq!(power, Ask::Power { restart: true });
        assert_eq!(
            serde_json::from_str::<Ask>(r#"{"ask": "shown"}"#).expect("shown"),
            Ask::Shown
        );
        assert!(serde_json::from_str::<Ask>(r#"{"ask": "session", "name": "plasma"}"#).is_err());
        let line = serde_json::to_string(&Answer::no("Wrong password.")).expect("a line");
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<Answer>(&line).expect("an answer"),
            Answer::no("Wrong password.")
        );
        assert_eq!(
            serde_json::from_str::<Answer>(r#"{"ok": true}"#).expect("an answer"),
            Answer::yes()
        );
    }

    #[test]
    fn a_user_name_is_one_an_account_can_have() {
        for good in ["ada", "ada.lovelace", "a_b-c", "machine$", "u1"] {
            assert!(is_a_user_name(good), "{good}");
        }
        for bad in [
            "",
            "-rf",
            "a b",
            "a/b",
            "a;b",
            "$",
            "a$b",
            "ü",
            &"x".repeat(65),
        ] {
            assert!(!is_a_user_name(bad), "{bad}");
        }
    }
}
