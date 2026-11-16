//! A configuration on trial: applied for a short while, and unless the
//! person says "keep" the one from before comes back by itself.
//!
//! This is for the change that might take away the means to undo it (the
//! keys rebound, the windows somewhere unreachable, the panel gone): if
//! nothing can be clicked any more, waiting is enough. On Linux the
//! sandbox answers the same worry by trying the draft in a nested session;
//! Windows cannot nest a desktop, so there the trial is the way.

use gevrek_model::Config;
use std::time::{Duration, Instant};

/// How long a configuration is on trial.
pub const TRIAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct TryOut {
    /// What the window manager ran before the trial.
    before: Config,
    until: Instant,
}

impl TryOut {
    pub fn begin(before: Config, now: Instant) -> Self {
        Self {
            before,
            until: now + TRIAL,
        }
    }

    /// The configuration that comes back when the trial is not kept.
    pub fn before(&self) -> &Config {
        &self.before
    }

    /// End the trial at once, as if its time were up (asked for while the
    /// window manager has not yet answered: going back waits for that).
    pub fn cut_short(&mut self, now: Instant) {
        self.until = now;
    }

    pub fn over(&self, now: Instant) -> bool {
        now >= self.until
    }

    /// The seconds left, as a person counts them down: 15 at the start, 1
    /// during the last second, 0 when it is over.
    pub fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.until.saturating_duration_since(now);
        if left.is_zero() {
            0
        } else {
            left.as_secs() + u64::from(left.subsec_nanos() > 0)
        }
    }
}

/// What sends the configuration from before a trial, should this program
/// fall over while the trial runs.
static REVERT: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> = std::sync::Mutex::new(None);

/// A trial runs: should the program panic, `revert` is called (it sends
/// the configuration from before on the line's own thread, which lives a
/// moment longer than the window).
pub fn revert_on_panic(revert: impl Fn() + Send + 'static) {
    static HOOKED: std::sync::Once = std::sync::Once::new();
    HOOKED.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let revert = REVERT.lock().ok().and_then(|mut held| held.take());
            if let Some(revert) = revert {
                revert();
                // (Time for the line to carry it.)
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            before(panic);
        }));
    });
    if let Ok(mut held) = REVERT.lock() {
        *held = Some(Box::new(revert));
    }
}

/// The trial is over: nothing is sent should the program fall over.
pub fn forget_revert() {
    if let Ok(mut held) = REVERT.lock() {
        *held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trial_counts_down_and_is_over_after_its_time() {
        let start = Instant::now();
        let trial = TryOut::begin(Config::default(), start);
        assert_eq!(trial.seconds_left(start), 15);
        assert!(!trial.over(start));
        assert_eq!(trial.seconds_left(start + Duration::from_millis(100)), 15);
        assert_eq!(trial.seconds_left(start + Duration::from_millis(14_200)), 1);
        assert!(!trial.over(start + Duration::from_millis(14_999)));
        assert!(trial.over(start + TRIAL));
        assert_eq!(trial.seconds_left(start + TRIAL), 0);
        assert_eq!(trial.seconds_left(start + TRIAL * 2), 0);

        let mut cut = TryOut::begin(Config::default(), start);
        cut.cut_short(start + Duration::from_secs(2));
        assert!(cut.over(start + Duration::from_secs(2)));
    }
}
