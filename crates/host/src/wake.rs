//! Waking the WM loop: whoever hands the loop something to do (the IPC
//! server a command or a configuration, a hook an event) says so here, and
//! a loop that waits for work instead of passing at a fixed rate stops
//! waiting at once.
//!
//! The loop installs how it is woken, once per process. Where none is
//! installed (a loop driven by its windowing system's own event queue, a
//! test), [`now`] does nothing: a loop must never depend on being woken,
//! only be quicker for it.

use std::sync::OnceLock;

static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// The loop says how it is woken. The first one installed stays; `false`
/// when there was one already.
pub fn install(waker: impl Fn() + Send + Sync + 'static) -> bool {
    WAKER.set(Box::new(waker)).is_ok()
}

/// There is work for the loop. Cheap, and safe from any thread.
pub fn now() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn the_first_waker_stays_and_is_called_every_time() {
        // Before one is installed this must not fail (other tests of this
        // process may or may not have run first; this is the only one that
        // installs).
        now();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        assert!(install(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(!install(|| panic!(
            "a second waker must not replace the first"
        )));
        now();
        now();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
