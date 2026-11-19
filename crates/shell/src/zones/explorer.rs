//! The system's own desktop icons, put away while the zones draw the
//! desktop and put back afterwards. On Windows these are Explorer's; a
//! backend with nothing of the kind answers every step with "done".
//!
//! One rule: a person's desktop is never lost. So the order is fixed,
//! and it is here in one place:
//!
//! - what a run before left away is put back first;
//! - how the desktop is, is written down before anything is changed;
//! - only when that was written are the icons put away;
//! - if putting away fails, what was begun is taken back, and the zones
//!   are not shown at all;
//! - putting back can be asked for any number of times.
//!
//! Nothing here touches a desktop: it tells a [`Desk`] what to do, and
//! the tests give it one in memory.

use super::Desk;

/// How the system's own icons stand, as far as the zones had to do with them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SystemIcons {
    /// As the person has them; nothing of ours was done to them.
    #[default]
    Theirs,
    /// Written down and put away, for as long as the zones are shown.
    Away,
    /// They could not be put away safely, and why. The zones are not
    /// shown while this stands.
    Refused(String),
}

impl SystemIcons {
    /// Whether the zones may be shown as far as the system's icons go.
    pub fn lets_zones_show(&self) -> bool {
        !matches!(self, SystemIcons::Refused(_))
    }

    /// Why the zones are not shown, if this is why.
    pub fn refusal(&self) -> Option<&str> {
        match self {
            SystemIcons::Refused(why) => Some(why),
            _ => None,
        }
    }
}

/// Put back what an earlier run left away (it ended without doing so).
/// Called before anything else is done with the desktop, whether the
/// icons are to be put away or not.
pub fn after_a_crash(desk: &mut dyn Desk) -> Result<(), String> {
    if !desk.system_icons_to_restore() {
        return Ok(());
    }
    log::warn!("the desktop's own icons were left put away by an earlier run; putting them back");
    desk.restore_system_icons()
        .map_err(|error| format!("the desktop's own icons could not be put back: {error}"))
}

/// Write the desktop down and put its own icons away, in that order.
/// Whatever fails, nothing is left away.
pub fn put_away(desk: &mut dyn Desk) -> SystemIcons {
    if let Err(why) = after_a_crash(desk) {
        return SystemIcons::Refused(why);
    }
    if let Err(error) = desk.journal_system_icons() {
        // Nothing was changed: there is nothing to take back.
        return SystemIcons::Refused(format!(
            "how the desktop is could not be written down, so its icons are left alone: {error}"
        ));
    }
    match desk.hide_system_icons() {
        Ok(()) => SystemIcons::Away,
        Err(error) => {
            // Whatever of it was begun is taken back, by what was written.
            let back = desk.restore_system_icons();
            SystemIcons::Refused(match back {
                Ok(()) => format!("the desktop's own icons could not be put away: {error}"),
                Err(again) => format!(
                    "the desktop's own icons could not be put away ({error}), and putting back what was \
                     begun failed too ({again}); `gevrek zones restore-explorer` tries again"
                ),
            })
        }
    }
}

/// Put the system's icons back as they were written down. Asked for when
/// the zones are switched off, when the shell ends, and from outside; as
/// often as one likes. What stands after it: theirs again, or (when the
/// system refused) still away, to be tried again.
pub fn put_back(desk: &mut dyn Desk, now: &SystemIcons) -> Result<SystemIcons, String> {
    // (Also where nothing of ours is away: a journal an earlier run left
    // open is put back all the same, and where there is none this does
    // nothing.)
    if *now != SystemIcons::Away && !desk.system_icons_to_restore() {
        return Ok(SystemIcons::Theirs);
    }
    desk.restore_system_icons()
        .map(|()| SystemIcons::Theirs)
        .map_err(|error| format!("the desktop's own icons could not be put back: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zones::draw::tests::FakeDesk;
    use gevrek_zones::backend::BackendError;
    use gevrek_zones::testing::FakeBackend;

    fn desk() -> FakeDesk {
        FakeDesk::new(FakeBackend::new())
    }

    fn said(desk: &FakeDesk) -> Vec<String> {
        desk.said.lock().unwrap().clone()
    }

    fn refused() -> BackendError {
        BackendError::Refused { what: "no".to_string() }
    }

    #[test]
    fn the_desktop_is_written_down_and_only_then_are_its_icons_put_away() {
        let mut desk = desk();
        assert_eq!(put_away(&mut desk), SystemIcons::Away);
        assert_eq!(said(&desk), ["journal", "hide"]);
        assert!(desk.icons_hidden && desk.backend.journal_open);
        // Put back: shown again, the journal closed.
        assert_eq!(put_back(&mut desk, &SystemIcons::Away), Ok(SystemIcons::Theirs));
        assert!(!desk.icons_hidden && !desk.backend.journal_open);
        assert_eq!(said(&desk), ["journal", "hide", "restore"]);
    }

    #[test]
    fn where_the_desktop_cannot_be_written_down_nothing_is_put_away() {
        let mut desk = desk();
        desk.fail_journal = Some(refused());
        let now = put_away(&mut desk);
        assert!(!now.lets_zones_show(), "{now:?}");
        assert!(now.refusal().unwrap().contains("written down"));
        assert_eq!(said(&desk), ["journal"], "hiding was never asked for");
        assert!(!desk.icons_hidden);
        // And there is nothing to put back.
        assert_eq!(put_back(&mut desk, &now), Ok(SystemIcons::Theirs));
        assert_eq!(said(&desk), ["journal"]);
    }

    #[test]
    fn where_putting_away_fails_what_was_begun_is_taken_back_and_the_zones_stay_away() {
        let mut desk = desk();
        desk.fail_hide = Some(refused());
        let now = put_away(&mut desk);
        assert!(matches!(&now, SystemIcons::Refused(why) if why.contains("could not be put away")));
        assert!(!now.lets_zones_show());
        assert_eq!(said(&desk), ["journal", "hide", "restore"]);
        assert!(!desk.icons_hidden && !desk.backend.journal_open);
        // Even the taking back fails: said, and to be tried again.
        let mut desk = self::desk();
        desk.fail_hide = Some(refused());
        desk.fail_restore = Some(refused());
        let now = put_away(&mut desk);
        assert!(now.refusal().unwrap().contains("restore-explorer"), "{now:?}");
        assert!(desk.backend.journal_open, "the journal stays open for the next try");
        assert_eq!(put_back(&mut desk, &now), Ok(SystemIcons::Theirs));
        assert!(!desk.backend.journal_open);
    }

    #[test]
    fn what_an_earlier_run_left_away_is_put_back_before_anything_else() {
        let mut desk = desk();
        // It ended with the icons away and its journal open.
        desk.backend.journal_open = true;
        desk.icons_hidden = true;
        assert_eq!(put_away(&mut desk), SystemIcons::Away);
        assert_eq!(said(&desk), ["restore", "journal", "hide"]);
        // Also where the icons are not to be put away at all this time.
        let mut desk = self::desk();
        desk.backend.journal_open = true;
        desk.icons_hidden = true;
        assert_eq!(after_a_crash(&mut desk), Ok(()));
        assert_eq!(said(&desk), ["restore"]);
        assert!(!desk.icons_hidden);
        // If that fails, nothing more is done to the desktop.
        let mut desk = self::desk();
        desk.backend.journal_open = true;
        desk.fail_restore = Some(refused());
        let now = put_away(&mut desk);
        assert!(matches!(now, SystemIcons::Refused(_)));
        assert_eq!(said(&desk), ["restore"]);
    }

    #[test]
    fn putting_back_can_be_asked_for_again_and_again() {
        let mut desk = desk();
        // With nothing away: nothing is asked of the system.
        for _ in 0..3 {
            assert_eq!(put_back(&mut desk, &SystemIcons::Theirs), Ok(SystemIcons::Theirs));
        }
        assert_eq!(said(&desk), Vec::<String>::new());
        let away = put_away(&mut desk);
        let back = put_back(&mut desk, &away).unwrap();
        assert_eq!(put_back(&mut desk, &back), Ok(SystemIcons::Theirs));
        assert_eq!(put_back(&mut desk, &back), Ok(SystemIcons::Theirs));
        assert_eq!(said(&desk), ["journal", "hide", "restore"], "put back once, and no more");
        // A system that refuses leaves them away, to be asked again.
        let away = put_away(&mut desk);
        desk.fail_restore = Some(refused());
        assert!(put_back(&mut desk, &away).is_err());
        assert!(desk.icons_hidden);
        assert_eq!(put_back(&mut desk, &away), Ok(SystemIcons::Theirs));
        assert!(!desk.icons_hidden);
    }
}
