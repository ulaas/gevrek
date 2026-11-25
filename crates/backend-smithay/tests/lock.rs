//! The session's lock: who may lock, and who may unlock. Real clients in
//! the test's own process ask for the lock as a lock screen does.
#![cfg(target_os = "linux")]

mod common;

use common::Harness;
use gevrek_model::config::Config;

#[test]
fn a_locked_session_is_locked_by_one_program_and_only_that_one_unlocks_it() {
    let mut harness = Harness::new(Config::new());
    let locker = harness.connect();
    let other = harness.connect();
    assert_eq!(harness.lock(locker), Some(true));
    assert!(harness.state.locked);
    // Another program asks for the lock of the locked session: it is told
    // no (finished), has nothing to unlock with, and the session stays
    // locked. (It was told yes, and its unlocking unlocked the session
    // without a password.)
    assert_eq!(harness.lock(other), Some(false));
    harness.clients[other]
        .app
        .lock
        .take()
        .expect("its lock")
        .destroy();
    harness.pump();
    assert!(harness.state.locked);
    // The one that locked unlocks.
    harness.unlock(locker);
    assert!(!harness.state.locked);
}

#[test]
fn a_lock_screen_that_is_gone_can_be_replaced_by_another() {
    let mut harness = Harness::new(Config::new());
    let locker = harness.connect();
    let other = harness.connect();
    assert_eq!(harness.lock(locker), Some(true));
    // The lock screen ends without unlocking (a fault): the session stays
    // locked, and another program may lock in its place, so that it can be
    // unlocked at all.
    let gone = harness.clients.remove(locker);
    drop(gone);
    harness.pump();
    assert!(harness.state.locked);
    let other = if other > locker { other - 1 } else { other };
    assert_eq!(harness.lock(other), Some(true));
    harness.unlock(other);
    assert!(!harness.state.locked);
}
