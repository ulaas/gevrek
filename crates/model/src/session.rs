//! The session as a whole (`desktop.session`): what every program started
//! in it is told through its environment. More of the session (default
//! programs, idle steps, locking) joins this section as it is built.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `desktop.session`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SessionConfig {
    /// Environment variables for every program of the session
    /// (`TERMINAL`, `QT_QPA_PLATFORMTHEME`, ...). An empty value removes
    /// the variable. Where gevrek is the session (Linux) they are set
    /// before anything is started; elsewhere programs gevrek starts get
    /// them.
    pub environment: BTreeMap<String, String>,
    /// Lock the screen after so many minutes without a key or the mouse;
    /// 0: never. Not while a program keeps the screen awake (a video, a
    /// call). On Windows the system's own setting decides.
    pub lock_after_minutes: u32,
    /// For so many seconds before the screen locks by itself the picture
    /// grows dark, as a warning: a key or the mouse brings it back and
    /// the screen does not lock. 0: no warning.
    pub dim_before_lock_seconds: u32,
    /// Lock the screen before the machine sleeps, so that it wakes up
    /// locked (where the system tells of sleep: logind on Linux).
    pub lock_before_sleep: bool,
    /// A window that fills the whole display (a game, a film) keeps the
    /// session awake: no lock and no switching off for want of a key or
    /// the mouse while one is shown. A game played with a controller
    /// moves neither. Off: only a program that asks for it does.
    pub awake_while_fullscreen: bool,
    /// The lock screen shows the wallpaper, darkened, behind the clock;
    /// off: the plain background.
    pub lock_shows_wallpaper: bool,
    /// Switch the displays off after so many minutes without a key or
    /// the mouse; 0: never. The next key or movement switches them on.
    /// On Windows the system's own setting decides.
    pub displays_off_after_minutes: u32,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            environment: BTreeMap::new(),
            lock_after_minutes: 0,
            dim_before_lock_seconds: 10,
            lock_before_sleep: true,
            awake_while_fullscreen: true,
            lock_shows_wallpaper: true,
            displays_off_after_minutes: 0,
        }
    }
}

impl SessionConfig {
    /// How dark the picture is, from 0 (as it is) to 1 (as dark as the
    /// warning gets), when nobody has used the session for `idle` and it
    /// locks after `lock_after`.
    pub fn dimmed(&self, idle: std::time::Duration, lock_after: std::time::Duration) -> f32 {
        let warning =
            std::time::Duration::from_secs(u64::from(self.dim_before_lock_seconds)).min(lock_after);
        if warning.is_zero() || idle >= lock_after {
            return 0.0;
        }
        let left = lock_after - idle;
        if left >= warning {
            0.0
        } else {
            1.0 - left.as_secs_f32() / warning.as_secs_f32()
        }
    }

    /// What is wrong with the settings, as `(path, message)`.
    pub fn problems(&self) -> Vec<(String, String)> {
        self.environment
            .keys()
            .filter(|name| {
                name.is_empty()
                    || name.starts_with(|c: char| c.is_ascii_digit())
                    || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .map(|name| {
                (
                    format!("desktop.session.environment.{name}"),
                    "A variable's name is letters, digits and _, and does not start with a digit"
                        .to_string(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_no_variable_is_reported() {
        let mut session = SessionConfig::default();
        session
            .environment
            .insert("TERMINAL".to_string(), "kitty".to_string());
        assert!(session.problems().is_empty());
        session
            .environment
            .insert("1BAD".to_string(), String::new());
        session
            .environment
            .insert("WITH SPACE".to_string(), String::new());
        assert_eq!(session.problems().len(), 2);
    }

    #[test]
    fn the_picture_grows_dark_in_the_seconds_before_the_lock() {
        use std::time::Duration;
        let session = SessionConfig {
            lock_after_minutes: 5,
            dim_before_lock_seconds: 10,
            ..Default::default()
        };
        let lock = Duration::from_secs(300);
        let at = |seconds: f32| session.dimmed(Duration::from_secs_f32(seconds), lock);
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(289.0), 0.0);
        assert!((at(295.0) - 0.5).abs() < 0.01, "{}", at(295.0));
        assert!(at(299.5) > 0.9);
        // Locked: the lock is what is shown.
        assert_eq!(at(300.0), 0.0);
        // No warning asked for.
        let none = SessionConfig {
            dim_before_lock_seconds: 0,
            ..session.clone()
        };
        assert_eq!(none.dimmed(Duration::from_secs(299), lock), 0.0);
        // A lock sooner than the warning is long: dark from the start on.
        assert!(session.dimmed(Duration::from_secs(2), Duration::from_secs(4)) > 0.4);
    }
}
