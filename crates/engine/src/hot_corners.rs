//! Hot corners: the pointer, put into a corner of a display and left
//! there for a moment, runs a command (`desktop.hot_corners`).
//!
//! The desktop says where the pointer is, as often as it looks
//! (`Watch::step`); this says when a corner's command is due. A corner
//! runs its command once: the pointer has to leave it and come back for
//! the next time. Nothing runs while a mouse button is held (something is
//! being dragged there, not pointed at).

use gevrek_model::command::DispatchCommand;
use gevrek_model::geometry::Rect;
use gevrek_model::settings::{HotCornersProfile, ScreenCorner};
use std::time::{Duration, Instant};

/// The corner the pointer is in, since when, and whether its command has
/// been run.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    /// The entry of the profile (by its place) and when the pointer came.
    within: Option<(usize, Instant)>,
    ran: bool,
}

/// The entry of `profile` whose corner of one of `displays` the point is
/// in, by its place among the entries.
pub fn corner_at(profile: &HotCornersProfile, displays: &[Rect], point: (i32, i32)) -> Option<usize> {
    let size = profile.reach();
    displays.iter().find_map(|display| {
        if display.width <= 0 || display.height <= 0 {
            return None;
        }
        let (x, y) = (point.0 - display.x, point.1 - display.y);
        if x < 0 || y < 0 || x >= display.width || y >= display.height {
            return None;
        }
        let (left, right) = (x < size, x >= display.width - size);
        let (top, bottom) = (y < size, y >= display.height - size);
        let corner = match (left, right, top, bottom) {
            (true, _, true, _) => ScreenCorner::TopLeft,
            (_, true, true, _) => ScreenCorner::TopRight,
            (true, _, _, true) => ScreenCorner::BottomLeft,
            (_, true, _, true) => ScreenCorner::BottomRight,
            _ => return None,
        };
        profile
            .corners
            .iter()
            .position(|entry| entry.enabled && entry.corner == corner)
    })
}

impl Watch {
    /// The pointer is at `point` now (`held`: with a mouse button down).
    /// The command to run, if one is due.
    pub fn step(
        &mut self,
        profile: &HotCornersProfile,
        displays: &[Rect],
        point: (i32, i32),
        held: bool,
        now: Instant,
    ) -> Option<DispatchCommand> {
        let at = profile
            .watched()
            .then(|| corner_at(profile, displays, point))
            .flatten();
        let Some(at) = at else {
            self.within = None;
            self.ran = false;
            return None;
        };
        // (Let go in the corner, the moment starts then.)
        if held {
            self.within = None;
            return None;
        }
        let since = match self.within {
            Some((known, since)) if known == at => since,
            _ => {
                self.within = Some((at, now));
                self.ran = false;
                now
            }
        };
        if self.ran || now.duration_since(since) < Duration::from_millis(profile.delay()) {
            return None;
        }
        self.ran = true;
        profile.corners.get(at).map(|entry| entry.command.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::settings::HotCorner;

    const DISPLAY: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    const SECOND: Rect = Rect {
        x: 1920,
        y: 0,
        width: 1280,
        height: 1024,
    };

    fn after(start: Instant, milliseconds: u64) -> Instant {
        start + Duration::from_millis(milliseconds)
    }

    #[test]
    fn the_first_corner_is_the_top_left_and_shows_the_overview() {
        let profile = HotCornersProfile::default();
        assert!(profile.watched());
        assert_eq!(corner_at(&profile, &[DISPLAY], (0, 0)), Some(0));
        assert_eq!(corner_at(&profile, &[DISPLAY], (1919, 0)), None);
        assert_eq!(corner_at(&profile, &[DISPLAY], (400, 300)), None);
        assert!(matches!(
            profile.corners[0].command,
            DispatchCommand::ToggleExpose
        ));
    }

    #[test]
    fn a_corner_runs_once_after_its_moment_and_again_after_the_pointer_left() {
        let profile = HotCornersProfile::default();
        let mut watch = Watch::default();
        let start = Instant::now();
        let displays = [DISPLAY];
        // Come: not yet. Stayed for the moment: once. Stayed on: no more.
        assert!(watch.step(&profile, &displays, (1, 1), false, start).is_none());
        assert!(watch
            .step(&profile, &displays, (1, 1), false, after(start, 50))
            .is_none());
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 400))
            .is_some());
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 900))
            .is_none());
        // Left and come back: again, after the moment.
        assert!(watch
            .step(&profile, &displays, (300, 300), false, after(start, 1000))
            .is_none());
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 1100))
            .is_none());
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 1500))
            .is_some());
    }

    #[test]
    fn nothing_runs_while_a_button_is_held_or_where_it_is_turned_off() {
        let mut profile = HotCornersProfile::default();
        let mut watch = Watch::default();
        let start = Instant::now();
        let displays = [DISPLAY];
        // Dragged into the corner and held there.
        assert!(watch.step(&profile, &displays, (0, 0), true, start).is_none());
        assert!(watch
            .step(&profile, &displays, (0, 0), true, after(start, 800))
            .is_none());
        // Let go: the moment starts then.
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 900))
            .is_none());
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 1400))
            .is_some());

        profile.enabled = false;
        let mut watch = Watch::default();
        assert!(watch.step(&profile, &displays, (0, 0), false, start).is_none());
        assert!(watch
            .step(&profile, &displays, (0, 0), false, after(start, 800))
            .is_none());
        profile.enabled = true;
        profile.corners[0].enabled = false;
        assert!(!profile.watched());
    }

    #[test]
    fn every_display_has_its_corners() {
        let profile = HotCornersProfile {
            corners: vec![
                HotCorner::default(),
                HotCorner {
                    corner: ScreenCorner::BottomRight,
                    command: DispatchCommand::WorkspaceOverview,
                    ..HotCorner::default()
                },
            ],
            ..HotCornersProfile::default()
        };
        let displays = [DISPLAY, SECOND];
        // The second display's top left, and each one's bottom right.
        assert_eq!(corner_at(&profile, &displays, (1920, 0)), Some(0));
        assert_eq!(corner_at(&profile, &displays, (1919, 1079)), Some(1));
        assert_eq!(corner_at(&profile, &displays, (3199, 1023)), Some(1));
        // Below the second display there is no display.
        assert_eq!(corner_at(&profile, &displays, (3199, 1079)), None);
    }
}
