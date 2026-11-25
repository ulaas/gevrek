//! How often the costly things are done: counted where they are done,
//! read (and set back to nothing) by whoever sums up a stretch of time.
//!
//! A count says what a timing cannot: that the state was put together
//! thirty times in a half minute in which nothing happened, or that the
//! rules were held against every window for each window that opened.
//! Counting costs one addition; nothing is said unless somebody asks.

use std::sync::atomic::{AtomicU64, Ordering};

/// What is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    /// The state the clients read was put together.
    StatePublished,
    /// The rules were held against one window.
    RulesResolved,
    /// A window was asked whether it is managed at all.
    IgnoreAsked,
    /// The windows were all asked for (a copy of each).
    WindowsListed,
    /// A frame of a motion was made.
    MotionFrame,
}

const ALL: [(Counted, &str); 5] = [
    (Counted::StatePublished, "state put together"),
    (Counted::RulesResolved, "rules held against a window"),
    (Counted::IgnoreAsked, "asked whether a window is managed"),
    (Counted::WindowsListed, "windows listed"),
    (Counted::MotionFrame, "frames of motion"),
];

static COUNTS: [AtomicU64; ALL.len()] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

fn place(what: Counted) -> usize {
    ALL.iter()
        .position(|(counted, _)| *counted == what)
        .unwrap_or(0)
}

/// It was done once more.
pub fn count(what: Counted) {
    COUNTS[place(what)].fetch_add(1, Ordering::Relaxed);
}

/// How often it was done since the counts were last taken.
pub fn so_far(what: Counted) -> u64 {
    COUNTS[place(what)].load(Ordering::Relaxed)
}

/// Every count with its name, and all of them set back to nothing.
pub fn take() -> Vec<(&'static str, u64)> {
    ALL.iter()
        .enumerate()
        .map(|(index, (_, name))| (*name, COUNTS[index].swap(0, Ordering::Relaxed)))
        .collect()
}

/// `state put together 30, rules held against a window 412`: the counts
/// that are more than nothing, taken (see [`take`]).
pub fn taken_in_words() -> String {
    let said: Vec<String> = take()
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    if said.is_empty() {
        "nothing counted".to_string()
    } else {
        said.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (The counts are the whole process's, and tests run side by side:
    // only what cannot be disturbed by another test is held here.)
    #[test]
    fn what_is_counted_is_there_to_be_read() {
        let before = so_far(Counted::MotionFrame);
        count(Counted::MotionFrame);
        count(Counted::MotionFrame);
        assert!(so_far(Counted::MotionFrame) >= before + 2);
    }

    #[test]
    fn every_count_has_a_place_and_a_name_of_its_own() {
        for (index, (what, name)) in ALL.iter().enumerate() {
            assert_eq!(place(*what), index);
            assert!(!name.is_empty());
            assert_eq!(ALL.iter().filter(|(_, other)| other == name).count(), 1);
        }
    }
}
