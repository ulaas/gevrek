//! One lesson while it is taken.
//!
//! The one who draws the tutorial tells this what happens (how the
//! desktop stands, which commands were carried out, what the person
//! pressed on the card, that time passed) and is told what follows: a
//! command to carry out, another step, the lesson's end. Nothing here
//! touches a window or a clock: the time is handed in.

use crate::facts::{met, Facts, Goal};
use crate::lessons::{Act, Lesson, Step};
use std::time::{Duration, Instant};

/// How long a step that was done stays, with its tick, before the next.
pub const DONE_FOR: Duration = Duration::from_millis(900);
/// After so long without the step being done, what it is about is ringed.
pub const RING_AFTER: Duration = Duration::from_secs(8);
/// And after so long, the card offers to show how it is done.
pub const OFFER_AFTER: Duration = Duration::from_secs(20);

/// What reaches a lesson.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The desktop as it stands now.
    Facts(Facts),
    /// The window manager carried out a command of this name.
    CarriedOut(String),
    /// Time passed.
    Tick,
    /// The card's buttons.
    Next,
    Back,
    Skip,
    /// Do it for me: the step's own command, or its button's.
    ShowMe,
    Pause(bool),
    Leave,
}

/// What follows from it, for the one who draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Hand this command (the JSON of a binding) to the window manager.
    Dispatch(String),
    /// Open one more practice window.
    OpenPractice,
    /// Another step is shown.
    Step,
    /// The lesson's last step is done.
    Finished,
    /// The lesson was left before its end.
    Left,
}

/// How a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Waiting,
    /// It was done: shown with its tick for a moment.
    Done,
}

/// How much help a waiting step shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    /// The words and the keys.
    Quiet,
    /// What it is about is ringed, the rest dimmed.
    Ring,
    /// The card offers to do it once, to be watched.
    Offer,
}

/// A lesson that is being taken.
#[derive(Debug, Clone)]
pub struct Run {
    lesson: &'static Lesson,
    at: usize,
    /// The desktop when the step began (none until it was first told).
    before: Option<Facts>,
    now: Facts,
    carried_out: Vec<String>,
    began: Instant,
    done: Option<Instant>,
    /// It was shown how: the offer is not made again for this step.
    shown: bool,
    paused: Option<Instant>,
}

impl Run {
    /// Begin a lesson, at this step (0 the first; a lesson left earlier
    /// is taken up where it was).
    pub fn begin(lesson: &'static Lesson, at: usize, now: Instant) -> Self {
        Self {
            lesson,
            at: at.min(lesson.steps.len().saturating_sub(1)),
            before: None,
            now: Facts::default(),
            carried_out: Vec::new(),
            began: now,
            done: None,
            shown: false,
            paused: None,
        }
    }

    pub fn lesson(&self) -> &'static Lesson {
        self.lesson
    }

    /// The step that is shown, counted from 0.
    pub fn at(&self) -> usize {
        self.at
    }

    pub fn step(&self) -> &'static Step {
        &self.lesson.steps[self.at]
    }

    pub fn state(&self) -> StepState {
        if self.done.is_some() {
            StepState::Done
        } else {
            StepState::Waiting
        }
    }

    pub fn paused(&self) -> bool {
        self.paused.is_some()
    }

    /// How much help the step shows now. A step that is only read, one
    /// that is done and a paused lesson show none.
    pub fn help(&self, now: Instant) -> Help {
        if self.done.is_some() || self.paused.is_some() || self.step().goal == Goal::Read {
            return Help::Quiet;
        }
        let waited = now.saturating_duration_since(self.began);
        if waited >= OFFER_AFTER && !self.shown && self.step().act != Act::None {
            Help::Offer
        } else if waited >= RING_AFTER {
            Help::Ring
        } else {
            Help::Quiet
        }
    }

    fn check(&mut self, now: Instant) {
        if self.done.is_some() || self.paused.is_some() {
            return;
        }
        let Some(before) = &self.before else {
            return;
        };
        if met(self.step().goal, before, &self.now, &self.carried_out) {
            self.done = Some(now);
        }
    }

    /// Go to another step (or past the last: the lesson is finished).
    fn go(&mut self, to: usize, now: Instant) -> Effect {
        if to >= self.lesson.steps.len() {
            return Effect::Finished;
        }
        self.at = to;
        // The next step is measured against the desktop as it is now.
        self.before = self.before.as_ref().map(|_| self.now.clone());
        self.carried_out.clear();
        self.began = now;
        self.done = None;
        self.shown = false;
        Effect::Step
    }

    /// Take in what happened. What follows from it.
    pub fn take(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        match input {
            Input::Facts(facts) => {
                if self.before.is_none() {
                    self.before = Some(facts.clone());
                }
                self.now = facts;
                self.check(now);
                Vec::new()
            }
            Input::CarriedOut(name) => {
                self.carried_out.push(name);
                self.check(now);
                Vec::new()
            }
            Input::Tick => match self.done {
                Some(done) if now.saturating_duration_since(done) >= DONE_FOR => {
                    vec![self.go(self.at + 1, now)]
                }
                _ => Vec::new(),
            },
            // On from a step that is read, or that is done; a step that
            // still waits is gone past with Skip, which says so.
            Input::Next => {
                if self.step().goal == Goal::Read || self.done.is_some() {
                    vec![self.go(self.at + 1, now)]
                } else {
                    Vec::new()
                }
            }
            Input::Skip => vec![self.go(self.at + 1, now)],
            Input::Back => match self.at.checked_sub(1) {
                Some(to) => vec![self.go(to, now)],
                None => Vec::new(),
            },
            Input::ShowMe => {
                self.shown = true;
                match self.step().act {
                    Act::Command(command) if !command.is_empty() => {
                        vec![Effect::Dispatch(command.to_string())]
                    }
                    Act::Practice => vec![Effect::OpenPractice],
                    _ => Vec::new(),
                }
            }
            Input::Pause(on) => {
                match (on, self.paused) {
                    (true, None) => self.paused = Some(now),
                    // (The time it was paused is not time waited.)
                    (false, Some(since)) => {
                        self.began += now.saturating_duration_since(since);
                        self.paused = None;
                        self.check(now);
                    }
                    _ => {}
                }
                Vec::new()
            }
            Input::Leave => vec![Effect::Left],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::Window;
    use crate::lessons::lesson;

    fn desk(focused: u64) -> Facts {
        let window = |id: u64, x: i32| Window {
            id,
            at: (x, 0, 800, 900),
            focused: id == focused,
            floating: false,
            maximized: false,
            app: crate::PRACTICE_APP.to_string(),
        };
        Facts {
            display: String::new(),
            workspace: crate::WORKSPACE.to_string(),
            layout: "Dwindle".to_string(),
            windows: vec![window(1, 0), window(2, 800), window(3, 1600)],
            containers: 3,
            displays: 1,
            surfaces: Vec::new(),
        }
    }

    #[test]
    fn a_step_is_done_when_the_desktop_says_so_and_the_next_comes_by_itself() {
        let start = Instant::now();
        let focus = lesson("focus").expect("the lesson");
        let mut run = Run::begin(focus, 0, start);
        assert_eq!((run.at(), run.state()), (0, StepState::Waiting));
        // The desktop as it is: nothing has happened.
        assert!(run.take(Input::Facts(desk(1)), start).is_empty());
        assert_eq!(run.state(), StepState::Waiting);
        // The keyboard goes to another window: done, however it was done.
        let then = start + Duration::from_secs(2);
        run.take(Input::Facts(desk(2)), then);
        assert_eq!(run.state(), StepState::Done);
        // It stays a moment, then the next step comes.
        assert!(run.take(Input::Tick, then + Duration::from_millis(300)).is_empty());
        assert_eq!(run.take(Input::Tick, then + DONE_FOR), [Effect::Step]);
        assert_eq!((run.at(), run.state()), (1, StepState::Waiting));
        // The next step is measured from where the desktop is now: the
        // same facts again are nothing new.
        run.take(Input::Facts(desk(2)), then + Duration::from_secs(2));
        assert_eq!(run.state(), StepState::Waiting);
    }

    #[test]
    fn help_comes_with_waiting_and_show_me_does_the_steps_own_command() {
        let start = Instant::now();
        let mut run = Run::begin(lesson("focus").expect("the lesson"), 0, start);
        run.take(Input::Facts(desk(1)), start);
        assert_eq!(run.help(start + Duration::from_secs(1)), Help::Quiet);
        assert_eq!(run.help(start + RING_AFTER), Help::Ring);
        assert_eq!(run.help(start + OFFER_AFTER), Help::Offer);
        // Shown: the command the step teaches is handed over, and the
        // offer is not made a second time.
        let effects = run.take(Input::ShowMe, start + OFFER_AFTER);
        assert!(matches!(&effects[..], [Effect::Dispatch(command)] if command.contains("focus_direction")));
        assert_eq!(run.help(start + OFFER_AFTER + Duration::from_secs(60)), Help::Ring);
        // Paused: no help, nothing is done, and the pause is not waiting.
        let mut run = Run::begin(lesson("focus").expect("the lesson"), 0, start);
        run.take(Input::Facts(desk(1)), start);
        run.take(Input::Pause(true), start + Duration::from_secs(1));
        run.take(Input::Facts(desk(2)), start + Duration::from_secs(2));
        assert_eq!(run.state(), StepState::Waiting);
        assert_eq!(run.help(start + Duration::from_secs(100)), Help::Quiet);
        run.take(Input::Pause(false), start + Duration::from_secs(100));
        assert_eq!(run.state(), StepState::Done, "what was done meanwhile counts when it goes on");
        // A practice window is asked of the one who draws.
        let mut run = Run::begin(lesson("tiling").expect("the lesson"), 3, start);
        assert_eq!(run.take(Input::ShowMe, start), [Effect::OpenPractice]);
    }

    #[test]
    fn the_cards_buttons_go_on_back_past_and_out() {
        let start = Instant::now();
        let tiling = lesson("tiling").expect("the lesson");
        let mut run = Run::begin(tiling, 0, start);
        run.take(Input::Facts(desk(1)), start);
        // A step that is read is gone on from by hand, and not by itself.
        assert!(run.take(Input::Tick, start + Duration::from_secs(60)).is_empty());
        assert_eq!(run.take(Input::Next, start), [Effect::Step]);
        // One that waits is not gone on from with Next, but past with Skip.
        assert!(run.take(Input::Next, start).is_empty());
        assert_eq!(run.take(Input::Skip, start), [Effect::Step]);
        assert_eq!(run.at(), 2);
        assert_eq!(run.take(Input::Back, start), [Effect::Step]);
        assert_eq!(run.at(), 1);
        run.take(Input::Back, start);
        assert!(run.take(Input::Back, start).is_empty(), "before the first there is none");
        // Past the last: finished. And out at any time.
        let mut run = Run::begin(tiling, tiling.steps.len() - 1, start);
        assert_eq!(run.take(Input::Next, start), [Effect::Finished]);
        assert_eq!(run.take(Input::Leave, start), [Effect::Left]);
        // A command that leaves nothing to look at is done when the
        // window manager says it carried it out.
        let mut run = Run::begin(lesson("workspaces").expect("the lesson"), 4, start);
        run.take(Input::Facts(desk(1)), start);
        run.take(Input::CarriedOut("focus_direction".to_string()), start);
        assert_eq!(run.state(), StepState::Waiting);
        run.take(Input::CarriedOut("toggle_expose".to_string()), start);
        assert_eq!(run.state(), StepState::Done);
    }
}
