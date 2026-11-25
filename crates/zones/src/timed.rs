//! A call that may wait on the system (the file system, another process),
//! said in the log with what was asked, how it came out and how long it
//! took.
//!
//! Whoever runs the zones calls the backend on its one thread: while a
//! call waits nothing else is done, and on Wayland the compositor's words
//! pile up meanwhile until it cuts the shell off. So each such call is
//! said at debug, and one that took longer than [`SLOW`] as a warning,
//! whatever the level: that the shell was stuck, and in what.

use std::fmt::Display;
use std::time::{Duration, Instant};

/// A call longer than this is said as a warning.
pub const SLOW: Duration = Duration::from_millis(100);

/// Make the call, and say it: `asked` is what was asked ("listing
/// /home/a/Desktop"), `outcome` what came of it ("12 items"). Neither is
/// made where nothing is written.
pub fn timed<T>(
    asked: impl FnOnce() -> String,
    call: impl FnOnce() -> T,
    outcome: impl FnOnce(&T) -> String,
) -> T {
    said(log::Level::Debug, asked, call, outcome)
}

/// As [`timed`], for a call made many times over (a picture asked for
/// each icon): said at trace, and as a warning only where it is slow.
pub fn timed_quietly<T>(
    asked: impl FnOnce() -> String,
    call: impl FnOnce() -> T,
    outcome: impl FnOnce(&T) -> String,
) -> T {
    said(log::Level::Trace, asked, call, outcome)
}

fn said<T>(
    quick: log::Level,
    asked: impl FnOnce() -> String,
    call: impl FnOnce() -> T,
    outcome: impl FnOnce(&T) -> String,
) -> T {
    let started = Instant::now();
    let result = call();
    let took = started.elapsed();
    if took >= SLOW {
        log::warn!(
            "{} held the zones' thread for {} ms (more than {} ms): {}",
            asked(),
            took.as_millis(),
            SLOW.as_millis(),
            outcome(&result)
        );
    } else if log::log_enabled!(quick) {
        log::log!(
            quick,
            "{} took {} ms: {}",
            asked(),
            took.as_millis(),
            outcome(&result)
        );
    }
    result
}

/// How a call that may fail came out: `ok` says what it gave, the error
/// says itself.
pub fn result<T, E: Display>(result: &Result<T, E>, ok: impl FnOnce(&T) -> String) -> String {
    match result {
        Ok(value) => ok(value),
        Err(error) => format!("failed: {error}"),
    }
}

/// How a call that gives nothing back but may fail came out.
pub fn done<E: Display>(outcome: &Result<(), E>) -> String {
    result(outcome, |_| "done".to_string())
}

/// So many things, in words ("1 item", "3 items").
pub fn count(number: usize, one: &str, many: &str) -> String {
    if number == 1 {
        format!("1 {one}")
    } else {
        format!("{number} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_gives_back_what_it_gave_and_says_how_it_came_out() {
        let listed: Result<Vec<u32>, String> = timed(
            || "listing".to_string(),
            || Ok(vec![1, 2]),
            |listed| result(listed, |items| count(items.len(), "item", "items")),
        );
        assert_eq!(listed, Ok(vec![1, 2]));
        assert_eq!(
            result(&listed, |items| count(items.len(), "item", "items")),
            "2 items"
        );
        assert_eq!(done::<String>(&Err("no".to_string())), "failed: no");
        assert_eq!(count(1, "item", "items"), "1 item");
    }
}
