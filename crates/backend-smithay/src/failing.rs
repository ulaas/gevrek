//! A failure in a path that runs each frame (or each move of the
//! pointer) is said once, when it begins, and once when it ends: not each
//! time, which would fill the journal (`docs/LOGGING.md`).
//!
//! The same path runs for each window of a frame, and it may fail for one
//! and work for the next: it counts as working again only when it has not
//! failed for a while.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a path must go without failing to count as working again.
const QUIET: Duration = Duration::from_secs(2);

/// One path that may fail again and again: `static`, one for each.
pub struct Failing {
    what: &'static str,
    level: log::Level,
    failing: AtomicBool,
    last: Mutex<Option<Instant>>,
}

impl Failing {
    /// For the path named `what` ("drawing a stackbar"), whose failures are
    /// said at debug.
    pub const fn new(what: &'static str) -> Self {
        Self::at(what, log::Level::Debug)
    }

    /// As `new`, for failures that are said at `level`.
    pub const fn at(what: &'static str, level: log::Level) -> Self {
        Self {
            what,
            level,
            failing: AtomicBool::new(false),
            last: Mutex::new(None),
        }
    }

    /// It failed: said only if it was working until now.
    pub fn failed(&self, error: impl std::fmt::Display) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some(Instant::now());
        }
        if !self.failing.swap(true, Ordering::Relaxed) {
            log::log!(
                self.level,
                "{} failed (said once while it goes on failing): {error}",
                self.what
            );
        }
    }

    /// It worked: said if it was failing, and has not failed for a while.
    pub fn worked(&self) {
        if !self.failing.load(Ordering::Relaxed) {
            return;
        }
        let quiet = self
            .last
            .lock()
            .map_or(true, |last| last.is_none_or(|at| at.elapsed() >= QUIET));
        if quiet && self.failing.swap(false, Ordering::Relaxed) {
            log::log!(self.level, "{} works again", self.what);
        }
    }

    /// The result of one try, said as above.
    pub fn note<T, E: std::fmt::Display>(&self, result: Result<T, E>) -> Option<T> {
        match result {
            Ok(value) => {
                self.worked();
                Some(value)
            }
            Err(error) => {
                self.failed(error);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_that_goes_on_is_counted_as_one() {
        static PATH: Failing = Failing::new("a path of the test");
        assert!(!PATH.failing.load(Ordering::Relaxed));
        PATH.failed("broken");
        PATH.failed("broken");
        assert!(PATH.failing.load(Ordering::Relaxed));
        // (One window worked between two that failed: still failing.)
        PATH.worked();
        assert!(PATH.failing.load(Ordering::Relaxed));
        *PATH.last.lock().unwrap() = Some(Instant::now() - QUIET);
        PATH.worked();
        assert!(!PATH.failing.load(Ordering::Relaxed));
    }
}
