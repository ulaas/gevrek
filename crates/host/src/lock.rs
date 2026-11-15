//! Whether the session is locked. Set by the window manager when a lock
//! screen takes the session; read by the IPC server, which refuses
//! everything that acts on the desktop while nobody can see what happens.

use std::sync::atomic::{AtomicBool, Ordering};

static LOCKED: AtomicBool = AtomicBool::new(false);

pub fn set(locked: bool) {
    LOCKED.store(locked, Ordering::Release);
}

pub fn is_locked() -> bool {
    LOCKED.load(Ordering::Acquire)
}
