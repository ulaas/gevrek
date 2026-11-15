//! The idle times of the configuration, which Windows does not let gevrek
//! keep.
//!
//! On Linux gevrek is the session: it locks and switches the displays off
//! after `desktop.session.lock_after_minutes` and
//! `displays_off_after_minutes`. On Windows both are the system's to do,
//! after the times of its own settings (the power plan, the sign-in
//! options), whatever runs. A configuration that sets them here would be
//! believed and not kept, so it is said (`Notice`), and where the times
//! that do hold are shown.

use gevrek_engine::overlay_data::Notice;
use gevrek_model::session::SessionConfig;

/// The notice about idle times that are set and cannot be kept; none when
/// they are left alone (0, as they are when nothing is said of them).
pub fn notice(session: &SessionConfig) -> Option<Notice> {
    let set: Vec<String> = [
        ("lock_after_minutes", session.lock_after_minutes),
        (
            "displays_off_after_minutes",
            session.displays_off_after_minutes,
        ),
    ]
    .into_iter()
    .filter(|(_, minutes)| *minutes > 0)
    .map(|(name, minutes)| format!("desktop.session.{name} ({minutes})"))
    .collect();
    if set.is_empty() {
        return None;
    }
    let (verb, pronoun) = if set.len() == 1 {
        ("is", "it")
    } else {
        ("are", "them")
    };
    Some(Notice {
        id: "idle-times-are-the-systems".to_string(),
        title: "Windows keeps its own idle times".to_string(),
        text: format!(
            "{} {verb} set, and gevrek cannot keep {pronoun} on Windows: when the screen \
             locks, the displays switch off and the machine sleeps is decided by Windows \
             (Settings, System, Power; Accounts, Sign-in options). `gevrek health` shows \
             the times that hold. The keys and commands for locking and for switching the \
             displays off work as everywhere.",
            set.join(" and ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_said_of_idle_times_that_are_left_alone() {
        assert!(notice(&SessionConfig::default()).is_none());
        assert!(notice(&gevrek_model::Config::with_default_graph().desktop.session).is_none());
    }

    #[test]
    fn an_idle_time_that_is_set_is_named_with_its_value() {
        let mut session = SessionConfig {
            lock_after_minutes: 5,
            ..Default::default()
        };
        let one = notice(&session).expect("a notice");
        assert!(
            one.text.starts_with(
                "desktop.session.lock_after_minutes (5) is set, and gevrek cannot keep it"
            ),
            "{}",
            one.text
        );
        assert!(!one.text.contains("displays_off_after_minutes"));
        session.displays_off_after_minutes = 10;
        let both = notice(&session).expect("a notice");
        assert!(
            both.text.starts_with(
                "desktop.session.lock_after_minutes (5) and \
                 desktop.session.displays_off_after_minutes (10) are set, and gevrek cannot keep them"
            ),
            "{}",
            both.text
        );
        // One notice whichever of the two is set: it is told once.
        assert_eq!(one.id, both.id);
        session.lock_after_minutes = 0;
        assert!(notice(&session)
            .expect("a notice")
            .text
            .starts_with("desktop.session.displays_off_after_minutes (10) is set"));
    }
}
