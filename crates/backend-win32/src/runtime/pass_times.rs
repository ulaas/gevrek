//! How long a pass of the loop takes, and which part of it: so that what
//! is slow is known by name before anything is made faster.
//!
//! A pass is one turn of the loop in `run`: what was asked is carried
//! out, the system's events are taken in, the windows are laid out and
//! moved, the state is told. Its parts are marked as they end. Nothing
//! is said of it unless the log is at its debug level: then every half
//! minute how many passes there were, how long one took and which was
//! the slowest; at its trace level, a pass that took longer than a frame
//! is told at once as well, part by part. To have only this and no other
//! debug line, set `RUST_LOG=info,gevrek::passes=debug` (`=trace` for
//! each slow pass).

use std::time::{Duration, Instant};

/// The name these lines are logged under.
pub const LOG: &str = "gevrek::passes";

/// A pass longer than this is told of by itself: one frame at 60 Hz.
const SLOW: Duration = Duration::from_millis(16);
/// How often the passes since the last time are summed up.
const EVERY: Duration = Duration::from_secs(30);

/// The parts of a pass, in the order they come in.
pub const PARTS: [&str; 6] = [
    "commands",
    "looking around",
    "events",
    "layout and motion",
    "checks",
    "state and effects",
];

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pass {
    pub whole: Duration,
    pub parts: [Duration; PARTS.len()],
}

impl Pass {
    /// `events 3.1 ms, layout and motion 14.0 ms`: the parts that took
    /// a tenth of a millisecond or more.
    pub fn in_words(&self) -> String {
        let parts: Vec<String> = PARTS
            .iter()
            .zip(self.parts)
            .filter(|(_, took)| *took >= Duration::from_micros(100))
            .map(|(name, took)| format!("{name} {}", millis(took)))
            .collect();
        if parts.is_empty() {
            "nothing that took time".to_string()
        } else {
            parts.join(", ")
        }
    }
}

fn millis(time: Duration) -> String {
    format!("{:.1} ms", time.as_secs_f64() * 1000.0)
}

#[derive(Debug)]
pub struct PassTimes {
    began: Instant,
    last_mark: Instant,
    now: Pass,
    next_part: usize,
    // Since the last summing up.
    passes: u32,
    slow: u32,
    sum: Duration,
    slowest: Pass,
    summed_up: Instant,
}

impl PassTimes {
    pub fn new(now: Instant) -> Self {
        Self {
            began: now,
            last_mark: now,
            now: Pass::default(),
            next_part: 0,
            passes: 0,
            slow: 0,
            sum: Duration::ZERO,
            slowest: Pass::default(),
            summed_up: now,
        }
    }

    /// A pass begins. (One that was left before its end, as while gevrek
    /// is paused, is not counted.)
    pub fn begin(&mut self, now: Instant) {
        self.began = now;
        self.last_mark = now;
        self.now = Pass::default();
        self.next_part = 0;
    }

    /// The next part of the pass has ended.
    pub fn mark(&mut self, now: Instant) {
        if let Some(part) = self.now.parts.get_mut(self.next_part) {
            *part = now.saturating_duration_since(self.last_mark);
        }
        self.next_part += 1;
        self.last_mark = now;
    }

    /// The pass has ended (before the loop waits for work). Returns the
    /// pass where it was longer than a frame, and what is to be said of
    /// the passes since the last time, where it is time for that.
    pub fn end(&mut self, now: Instant) -> (Option<Pass>, Option<String>) {
        self.now.whole = now.saturating_duration_since(self.began);
        let pass = self.now;
        self.passes += 1;
        self.sum += pass.whole;
        if pass.whole > self.slowest.whole {
            self.slowest = pass;
        }
        let slow = (pass.whole > SLOW).then_some(pass);
        if slow.is_some() {
            self.slow += 1;
        }
        let summary = (now.saturating_duration_since(self.summed_up) >= EVERY).then(|| {
            let said = format!(
                "{} passes, {} each on average, {} longer than a frame; the slowest {}: {}",
                self.passes,
                millis(self.sum / self.passes.max(1)),
                self.slow,
                millis(self.slowest.whole),
                self.slowest.in_words(),
            );
            self.passes = 0;
            self.slow = 0;
            self.sum = Duration::ZERO;
            self.slowest = Pass::default();
            self.summed_up = now;
            said
        });
        (slow, summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    #[test]
    fn a_pass_is_timed_part_by_part() {
        let start = Instant::now();
        let mut times = PassTimes::new(start);
        times.begin(start);
        // Commands 1 ms, looking around nothing, events 3 ms, layout 14 ms.
        times.mark(start + ms(1));
        times.mark(start + ms(1));
        times.mark(start + ms(4));
        times.mark(start + ms(18));
        times.mark(start + ms(18));
        times.mark(start + ms(19));
        let (slow, summary) = times.end(start + ms(19));
        let pass = slow.expect("longer than a frame");
        assert_eq!(pass.whole, ms(19));
        assert_eq!(pass.parts, [ms(1), ms(0), ms(3), ms(14), ms(0), ms(1)]);
        assert_eq!(
            pass.in_words(),
            "commands 1.0 ms, events 3.0 ms, layout and motion 14.0 ms, state and effects 1.0 ms"
        );
        assert_eq!(summary, None, "not yet time to sum up");
    }

    #[test]
    fn a_quick_pass_is_not_told_of_and_the_passes_are_summed_up_in_time() {
        let start = Instant::now();
        let mut times = PassTimes::new(start);
        let mut at = start;
        let mut said = Vec::new();
        // A pass of 2 ms every second for a minute, and one of 40 ms.
        for second in 0..61u64 {
            at = start + Duration::from_secs(second);
            times.begin(at);
            let took = if second == 7 { ms(40) } else { ms(2) };
            for _ in 0..3 {
                times.mark(at);
            }
            times.mark(at + took);
            let (slow, summary) = times.end(at + took);
            assert_eq!(slow.is_some(), second == 7);
            said.extend(summary);
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].starts_with("31 passes, 3.2 ms each on average, 1 longer than a frame; the slowest 40.0 ms: layout and motion 40.0 ms"),
            "{}",
            said[0]
        );
        // The second half minute knows nothing of the first.
        assert!(
            said[1].starts_with("30 passes, 2.0 ms each on average, 0 longer"),
            "{}",
            said[1]
        );
        let _ = at;
    }

    #[test]
    fn a_pass_left_before_its_end_is_not_counted_and_more_marks_do_no_harm() {
        let start = Instant::now();
        let mut times = PassTimes::new(start);
        times.begin(start);
        times.mark(start + ms(500));
        // (Left: the loop began anew.)
        times.begin(start + ms(600));
        for _ in 0..10 {
            times.mark(start + ms(601));
        }
        let (slow, _) = times.end(start + ms(601));
        assert_eq!(slow, None);
        assert_eq!(Pass::default().in_words(), "nothing that took time");
    }
}
