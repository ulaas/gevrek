//! The heartbeat of the WM loop: the loop marks every pass, and the IPC
//! server reads how long ago the last one was, to tell a stuck loop from a
//! healthy one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// When the loop last started a pass, in milliseconds since
/// [`process_start`], plus one (zero means it never ran).
static LAST_BEAT_MS: AtomicU64 = AtomicU64::new(0);

fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// The WM loop calls this at the start of every pass.
pub fn beat() {
    let now = process_start().elapsed().as_millis() as u64;
    LAST_BEAT_MS.store(now + 1, Ordering::Relaxed);
}

/// How long ago the WM loop last started a pass; `None` before the first
/// one. A healthy loop passes many times a second, so a large value means it
/// is stuck inside one pass.
pub fn time_since_last_beat() -> Option<Duration> {
    let last = LAST_BEAT_MS.load(Ordering::Relaxed);
    if last == 0 {
        return None;
    }
    let now = process_start().elapsed().as_millis() as u64;
    Some(Duration::from_millis(now.saturating_sub(last - 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_since_the_last_beat_grows_until_the_next_beat() {
        beat();
        std::thread::sleep(Duration::from_millis(60));
        let stale = time_since_last_beat().expect("a beat was recorded");
        assert!(stale >= Duration::from_millis(50), "{stale:?}");
        beat();
        let fresh = time_since_last_beat().expect("a beat was recorded");
        assert!(fresh < stale, "{fresh:?} vs {stale:?}");
    }
}
