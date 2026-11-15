//! Windows whose next move/resize event was caused by the WM itself.
//!
//! An entry is consumed by the first matching event. Entries used to be
//! cleared before *every* window event, so an unrelated event arriving between
//! our `SetWindowPos` and its location-change event dropped the suppression and
//! the WM treated its own move as a user move. Entries now live until consumed
//! or until they expire, which also covers moves that produce no event at all
//! (for example, a `SetWindowPos` to the current rectangle).

use crate::WindowHandle;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Longest time a self-triggered event is expected to take to arrive.
const SUPPRESSION_TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct SuppressedWindows {
    deadlines: HashMap<WindowHandle, Instant>,
}

impl SuppressedWindows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Suppress the next move/resize event of `handle`.
    pub fn insert(&mut self, handle: WindowHandle) {
        self.insert_at(handle, Instant::now());
    }

    /// Consume the suppression of `handle`. True if it was still active.
    pub fn remove(&mut self, handle: &WindowHandle) -> bool {
        self.remove_at(handle, Instant::now())
    }

    pub fn clear(&mut self) {
        self.deadlines.clear();
    }

    fn insert_at(&mut self, handle: WindowHandle, now: Instant) {
        self.deadlines.retain(|_, deadline| *deadline > now);
        self.deadlines.insert(handle, now + SUPPRESSION_TTL);
    }

    fn remove_at(&mut self, handle: &WindowHandle, now: Instant) -> bool {
        self.deadlines
            .remove(handle)
            .is_some_and(|deadline| deadline > now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(raw: isize) -> WindowHandle {
        WindowHandle(raw)
    }

    #[test]
    fn suppression_is_consumed_once() {
        let mut suppressed = SuppressedWindows::new();
        suppressed.insert(handle(1));
        assert!(suppressed.remove(&handle(1)));
        assert!(!suppressed.remove(&handle(1)));
    }

    #[test]
    fn unrelated_windows_do_not_disturb_a_suppression() {
        let mut suppressed = SuppressedWindows::new();
        suppressed.insert(handle(1));
        // Another window's event in between used to clear everything.
        assert!(!suppressed.remove(&handle(2)));
        suppressed.insert(handle(3));
        assert!(suppressed.remove(&handle(1)));
    }

    #[test]
    fn a_stale_suppression_never_swallows_a_later_user_move() {
        let mut suppressed = SuppressedWindows::new();
        let start = Instant::now();
        suppressed.insert_at(handle(1), start);
        assert!(!suppressed.remove_at(&handle(1), start + SUPPRESSION_TTL));
    }
}
