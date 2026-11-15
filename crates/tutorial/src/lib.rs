//! gevrek's tutorial.
//!
//! A person learns gevrek by doing: a lesson sets a scene (practice
//! windows on a workspace of the tutorial's own), and each of its steps
//! says one thing to do and waits until the window manager says it was
//! done. This crate is the part of that which needs no window and no
//! system:
//!
//! - [`facts`]: what a step can know of the desktop, read from the window
//!   manager's state, and what a step waits for ([`facts::Goal`]). A goal
//!   is about what came of it, not about which key was pressed: done by
//!   the pointer, or by another key, it is done.
//! - [`lessons`]: the lessons, as data. A step names the command it
//!   teaches; the keys shown for it are the ones this person's
//!   configuration has for that command ([`keys_for`]), never keys
//!   written here.
//! - [`run`]: one lesson while it is taken: where it is, when a step is
//!   done, when help is offered, what is asked of the window manager.
//! - [`progress`]: what is remembered between sessions: that the person
//!   was welcomed, which lessons they finished, where they stopped, and
//!   the version they last saw (for what is new after an update).
//!
//! The shell draws the card that says a step and the ring around what it
//! is about; the studio draws the welcome. Both ask this crate what to
//! show.

pub mod facts;
pub mod lessons;
pub mod progress;
pub mod run;

pub use facts::{Facts, Goal, Window};
pub use lessons::{lessons, Chapter, Demo, Lesson, Need, Scene, Spot, Step};
pub use progress::Progress;
pub use run::{Effect, Help, Input, Run, StepState};

/// The name of the workspace the tutorial practises on. It is the
/// tutorial's own: made when a lesson begins, gone when it ends.
pub const WORKSPACE: &str = "Tutorial";

/// What a practice window's application is called (its class, its app
/// id): by this the window manager knows them, keeps them out of what
/// it saves, and the tutorial closes them.
pub const PRACTICE_APP: &str = "gevrek-practice";

/// The keys this configuration has for a command (given as the JSON a
/// binding holds for it), as people read them (`Win+Left`): the bindings
/// of the keyboard that are switched on, in the usual mode, in the
/// configuration's order. None: nothing is bound to it.
pub fn keys_for(config: &gevrek_model::Config, command: &str) -> Vec<String> {
    let Ok(wanted) = serde_json::from_str::<serde_json::Value>(command) else {
        return Vec::new();
    };
    gevrek_model::shortcuts::keys_of_command(config, &wanted)
}

/// Set for a practice window by whoever opens it: its input is a pipe
/// held by the opener, and the window goes when the pipe is let go
/// (the lesson ended, or the shell is gone).
pub const HELD: &str = "GEVREK_PRACTICE_HELD";

/// How the welcome's program ends where the tour was asked for (its
/// exit code): whoever opened it begins the tour then.
pub const TOUR_ASKED: i32 = 10;

/// Set (to anything) where nobody is to be welcomed: checks, machines
/// that are set up by a script.
pub const NO_WELCOME: &str = "GEVREK_NO_WELCOME";

/// Whether this is the first time gevrek runs for this person: they
/// have no configuration of their own and were never welcomed.
pub fn first_launch(has_configuration: bool) -> bool {
    std::env::var_os(NO_WELCOME).is_none()
        && Progress::load(&progress::file()).first_launch(has_configuration)
}

/// The first launch floats: nothing of the person's is moved before
/// they were asked. Every workspace of this configuration gets the
/// floating layout (for this session; it is written nowhere).
pub fn float_first(config: &mut gevrek_model::Config) {
    for workspace in &mut config.desktop.workspaces {
        workspace.layout = gevrek_model::layout::LayoutType::Floating;
    }
}

/// Whether a configuration is one that floats as the first launch does.
pub fn floats_first(config: &gevrek_model::Config) -> bool {
    !config.desktop.workspaces.is_empty()
        && config
            .desktop
            .workspaces
            .iter()
            .all(|workspace| workspace.layout == gevrek_model::layout::LayoutType::Floating)
}

/// Whether the person is to be welcomed now, by what runs: a first
/// launch, floating as one does.
pub fn welcome_due(running: &gevrek_model::Config) -> bool {
    floats_first(running)
        && first_launch(gevrek_model::paths::resolve_config(None).0.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_launch_floats_on_every_workspace() {
        let mut config = gevrek_model::examples::starting_configuration();
        assert!(!floats_first(&config));
        float_first(&mut config);
        assert!(floats_first(&config));
        assert!(config.validate().iter().all(|issue| issue.severity
            != gevrek_model::settings::ConfigIssueSeverity::Error));
    }

    #[test]
    fn a_steps_keys_are_the_configurations_own() {
        let mut config = gevrek_model::examples::starting_configuration();
        let focus_right = r#"{"command":"focus_direction","direction":"right"}"#;
        let keys = keys_for(&config, focus_right);
        // (Both that the starting configuration has, the arrow first.)
        assert_eq!(keys.len(), 2, "{keys:?}");
        assert!(keys[0].ends_with("+RIGHT") || keys[0].ends_with("+Right"), "{keys:?}");
        assert!(keys[1].ends_with("+L"), "{keys:?}");
        // The wheel's binding of the same command is no key.
        assert!(keys.iter().all(|keys| !keys.contains("WHEEL")));
        // Bound to another key by the person: that one is shown.
        for binding in &mut config.desktop.bindings {
            if serde_json::to_value(&binding.command).ok()
                == serde_json::from_str::<serde_json::Value>(focus_right).ok()
            {
                binding.enabled = false;
            }
        }
        assert!(keys_for(&config, focus_right).is_empty());
        // What is no command has no keys.
        assert!(keys_for(&config, "not json").is_empty());
    }
}
