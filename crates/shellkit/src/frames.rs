//! How often the shell's loop went round and how many frames it drew,
//! summed up every half minute: so that a shell that should be still is
//! known to be still, and one that is not says how busy it is.
//!
//! Nothing is said unless the log is at its debug level for these lines:
//! `RUST_LOG=info,gevrek::frames=debug`.

use std::time::{Duration, Instant};

/// The name these lines are logged under.
pub const LOG: &str = "gevrek::frames";

/// How often the passes and frames since the last time are summed up.
const EVERY: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Frames {
    passes: u64,
    frames: u64,
    sum_ms: f64,
    worst_ms: f64,
    summed_up: Instant,
}

impl Frames {
    pub fn new(now: Instant) -> Self {
        Self {
            passes: 0,
            frames: 0,
            sum_ms: 0.0,
            worst_ms: 0.0,
            summed_up: now,
        }
    }

    /// The loop woke and went round once.
    pub fn pass(&mut self) {
        self.passes += 1;
    }

    /// A frame was drawn in so many milliseconds.
    pub fn frame(&mut self, took_ms: f64) {
        self.frames += 1;
        self.sum_ms += took_ms;
        self.worst_ms = self.worst_ms.max(took_ms);
    }

    /// What is to be said of the time since the last summing up, where it
    /// is time for that; the counts begin anew then.
    pub fn summary(&mut self, now: Instant) -> Option<String> {
        let since = now.saturating_duration_since(self.summed_up);
        if since < EVERY {
            return None;
        }
        let minutes = since.as_secs_f64() / 60.0;
        let said = if self.frames == 0 {
            format!(
                "{} passes ({:.0} a minute), no frame drawn",
                self.passes,
                self.passes as f64 / minutes
            )
        } else {
            format!(
                "{} passes ({:.0} a minute), {} frames ({:.0} a minute), {:.2} ms each on average, the longest {:.2} ms",
                self.passes,
                self.passes as f64 / minutes,
                self.frames,
                self.frames as f64 / minutes,
                self.sum_ms / self.frames as f64,
                self.worst_ms
            )
        };
        *self = Self::new(now);
        Some(said)
    }

    /// Sum up into the log where it is time and somebody listens.
    pub fn tell(&mut self, now: Instant) {
        if !log::log_enabled!(target: LOG, log::Level::Debug) {
            return;
        }
        if let Some(said) = self.summary(now) {
            log::debug!(target: LOG, "the last half minute: {said}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_said_before_its_time_and_the_counts_begin_anew_after() {
        let start = Instant::now();
        let mut frames = Frames::new(start);
        frames.pass();
        frames.pass();
        frames.frame(2.0);
        frames.frame(4.0);
        assert_eq!(frames.summary(start + Duration::from_secs(10)), None);
        let said = frames
            .summary(start + Duration::from_secs(30))
            .expect("it is time");
        assert_eq!(
            said,
            "2 passes (4 a minute), 2 frames (4 a minute), 3.00 ms each on average, the longest 4.00 ms"
        );
        // (Summed up: from nothing again.)
        assert_eq!(
            frames.summary(start + Duration::from_secs(60)).as_deref(),
            Some("0 passes (0 a minute), no frame drawn")
        );
    }
}
