//! The little that moves by itself on the zones: a zone rolling open and
//! shut, everything fading when it is hidden at once, the zones coming
//! above the windows, and the short shake of a zone that refused what was
//! asked of it. Nothing else is animated.
//!
//! Everything here is a number on its way from one value to another,
//! asked for at a time that is given (the frame's), so that a test says
//! where it is at any moment. With `still` set (the desktop's
//! `animations.reduce_motion`) a number is at its end at once and nothing
//! shakes. A frame is asked for only while a number is on its way
//! ([`Motion::busy`]); at rest there is nothing to draw again.

use gevrek_zones::ZoneId;
use std::collections::BTreeMap;

/// How long a zone takes to roll open or shut, in seconds.
pub const ROLL: f32 = 0.16;
/// How long everything takes to fade away or back.
pub const FADE: f32 = 0.18;
/// How long the zones take to come above the windows.
pub const PEEK: f32 = 0.14;
/// How long a refusal is shown.
pub const REFUSAL: f32 = 0.42;

/// What moves.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// How far a zone is open: 0 rolled up to its title, 1 whole.
    Open(ZoneId),
    /// How much is seen of what the one key puts away: 0 hidden, 1 there.
    Loose,
    /// How far the zones are above the windows: 0 below, 1 above.
    Peek,
}

/// A number on its way.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Glide {
    from: f32,
    to: f32,
    began: f64,
    lasts: f32,
}

impl Glide {
    fn value(&self, now: f64) -> f32 {
        if self.lasts <= 0.0 {
            return self.to;
        }
        let along = (((now - self.began) as f32) / self.lasts).clamp(0.0, 1.0);
        // Quick at first and gentle at the end.
        let eased = 1.0 - (1.0 - along).powi(3);
        self.from + (self.to - self.from) * eased
    }

    fn moving(&self, now: f64) -> bool {
        self.lasts > 0.0 && ((now - self.began) as f32) < self.lasts && self.from != self.to
    }
}

/// Everything that moves, and whether anything is allowed to.
#[derive(Debug, Clone, Default)]
pub struct Motion {
    /// Nothing moves by itself: every number is at its end at once.
    pub still: bool,
    glides: BTreeMap<Part, Glide>,
    /// When each zone last refused something.
    refusals: BTreeMap<ZoneId, f64>,
}

impl Motion {
    /// Where a part is at `now`, heading for `to`. A part that is first
    /// heard of is there already (nothing moves when the zones are first
    /// shown); one whose end changed sets out from where it is.
    pub fn toward(&mut self, part: Part, to: f32, lasts: f32, now: f64) -> f32 {
        let still = self.still;
        let glide = self.glides.entry(part).or_insert(Glide { from: to, to, began: now, lasts: 0.0 });
        if glide.to != to {
            let from = glide.value(now);
            *glide = Glide { from, to, began: now, lasts: if still { 0.0 } else { lasts } };
        }
        if still {
            return to;
        }
        glide.value(now)
    }

    /// A zone refused what was asked of it: it says so for a moment.
    pub fn refused(&mut self, zone: &ZoneId, now: f64) {
        self.refusals.insert(zone.clone(), now);
    }

    /// How a zone that refused shows it at `now`: how far it is shaken
    /// aside (in points), and how strongly its lock is marked (0 to 1).
    /// With movement reduced it is marked and not shaken.
    pub fn refusal(&self, zone: &ZoneId, now: f64) -> Option<(f32, f32)> {
        let since = (now - self.refusals.get(zone)?) as f32;
        if !(0.0..REFUSAL).contains(&since) {
            return None;
        }
        let along = since / REFUSAL;
        let fading = 1.0 - along;
        let aside = if self.still { 0.0 } else { (along * std::f32::consts::TAU * 2.5).sin() * 5.0 * fading };
        Some((aside, if self.still { 1.0 } else { fading.sqrt() }))
    }

    /// Whether something is on its way at `now`: the next frame is wanted.
    pub fn busy(&self, now: f64) -> bool {
        (!self.still && self.glides.values().any(|glide| glide.moving(now)))
            || (!self.still && self.refusals.values().any(|at| ((now - at) as f32) < REFUSAL))
    }

    /// In how many seconds something that is shown for a while and does
    /// not move ends (a refusal's mark with movement reduced): one frame
    /// is wanted then, and none before.
    pub fn ends_in(&self, now: f64) -> Option<f32> {
        if !self.still {
            return None;
        }
        self.refusals
            .values()
            .map(|at| REFUSAL - (now - at) as f32)
            .filter(|left| *left > 0.0)
            .reduce(f32::max)
    }

    /// Forget what is over, and the zones that are no longer there.
    pub fn settle(&mut self, now: f64, there: &dyn Fn(&ZoneId) -> bool) {
        self.refusals.retain(|zone, at| ((now - *at) as f32) < REFUSAL && there(zone));
        self.glides.retain(|part, _| match part {
            Part::Open(zone) => there(zone),
            Part::Loose | Part::Peek => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str) -> ZoneId {
        ZoneId::new(name)
    }

    #[test]
    fn a_part_is_where_it_is_first_seen_and_goes_to_its_new_end_in_time() {
        let mut motion = Motion::default();
        let part = Part::Open(zone("a"));
        // First heard of: there already, and nothing asks for a frame.
        assert_eq!(motion.toward(part.clone(), 0.0, ROLL, 10.0), 0.0);
        assert!(!motion.busy(10.0));
        // Its end changes: it sets out from where it is.
        assert_eq!(motion.toward(part.clone(), 1.0, ROLL, 10.0), 0.0);
        assert!(motion.busy(10.0));
        let half = motion.toward(part.clone(), 1.0, ROLL, 10.0 + f64::from(ROLL) / 2.0);
        assert!(half > 0.5 && half < 1.0, "{half}: quick at first");
        // Turned round on the way: from where it is, not from an end.
        let back = motion.toward(part.clone(), 0.0, ROLL, 10.0 + f64::from(ROLL) / 2.0);
        assert_eq!(back, half);
        let later = motion.toward(part.clone(), 0.0, ROLL, 10.0 + f64::from(ROLL));
        assert!(later < half && later > 0.0);
        // At its end it rests, and no frame is asked for.
        assert_eq!(motion.toward(part.clone(), 0.0, ROLL, 11.0), 0.0);
        assert!(!motion.busy(11.0));
    }

    #[test]
    fn with_movement_reduced_everything_is_at_its_end_at_once() {
        let mut motion = Motion { still: true, ..Motion::default() };
        let part = Part::Loose;
        assert_eq!(motion.toward(part.clone(), 1.0, FADE, 5.0), 1.0);
        assert_eq!(motion.toward(part.clone(), 0.0, FADE, 5.0), 0.0);
        assert!(!motion.busy(5.0));
        // A refusal is marked for a while and shakes nothing; one frame
        // is wanted when the mark ends.
        motion.refused(&zone("a"), 5.0);
        assert_eq!(motion.refusal(&zone("a"), 5.1), Some((0.0, 1.0)));
        assert!(!motion.busy(5.1));
        let left = motion.ends_in(5.1).unwrap();
        assert!((left - (REFUSAL - 0.1)).abs() < 0.001, "{left}");
        assert_eq!(motion.refusal(&zone("a"), 5.0 + f64::from(REFUSAL) + 0.01), None);
        assert_eq!(motion.ends_in(6.0), None);
    }

    #[test]
    fn a_refusal_shakes_for_a_moment_and_is_forgotten() {
        let mut motion = Motion::default();
        assert_eq!(motion.refusal(&zone("a"), 1.0), None);
        motion.refused(&zone("a"), 1.0);
        let (aside, mark) = motion.refusal(&zone("a"), 1.05).unwrap();
        assert!(aside.abs() > 1.0 && aside.abs() <= 5.0 && mark > 0.8, "{aside} {mark}");
        assert!(motion.busy(1.05));
        assert_eq!(motion.ends_in(1.05), None, "frames follow each other while it shakes");
        assert!(!motion.busy(2.0));
        motion.settle(2.0, &|_| true);
        assert!(motion.refusals.is_empty());
        // What is no longer there has nothing kept.
        motion.toward(Part::Open(zone("gone")), 1.0, ROLL, 2.0);
        motion.toward(Part::Peek, 1.0, PEEK, 2.0);
        motion.settle(2.0, &|zone| zone.as_str() != "gone");
        assert_eq!(motion.glides.len(), 1);
    }
}
