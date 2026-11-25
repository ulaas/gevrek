//! Whether gevrek's input hooks still hear, said in the log.
//!
//! gevrek's keys come from two low-level hooks (`hotkey.rs`), called on one
//! thread of their own while it waits for messages. When they stop being
//! called, every shortcut is dead and nothing says so: the hooks are renewed
//! every half minute (`HOOKS_RENEWED_EVERY`), and once that did not help,
//! only starting gevrek again did. So that the next time says why:
//!
//! - the input thread says which part of its pass it is in; a part still
//!   going on after [`STUCK`] (other than waiting for messages) is said by a
//!   thread of its own while it goes on, and again every [`STUCK_AGAIN`]: a
//!   thread that never comes back cannot renew its hooks, and every key of
//!   every program waits for it until Windows takes them away;
//! - a call of a hook still going on after [`HOOK_SLOW`] is said while it
//!   goes on (Windows takes a hook away, without a word, when it answers
//!   slower than `LowLevelHooksTimeout`);
//! - every [`SUMMARY_EVERY`], how often each hook was called and its
//!   slowest answer (a debug line), and a warning where the system had
//!   input meanwhile (`GetLastInputInfo`) but neither hook was called:
//!   Windows took them away, or another program's hook keeps the input from
//!   them. Said once when it begins and once when it ends.
//!
//! The hooks only count and time (atomics, no lock, no log): they are
//! called for every key and every movement of the mouse of every program.
//! How many keys, never which.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A part still going on after this is said while it goes on.
pub const STUCK: Duration = Duration::from_secs(1);
/// And again after each of these.
pub const STUCK_AGAIN: Duration = Duration::from_secs(5);
/// A call of a hook still going on after this is said.
pub const HOOK_SLOW: Duration = Duration::from_millis(500);
/// How often the hooks' counts are said.
pub const SUMMARY_EVERY: Duration = Duration::from_secs(30);

/// A part of the input thread's pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Part {
    /// Waiting for messages (the hooks are called meanwhile): never stuck.
    Waiting = 0,
    /// Taking the hooks out and putting them in anew.
    Renewing = 1,
    /// Bindings registered and taken away, gestures configured.
    Control = 2,
    /// The keys and buttons the hooks heard, matched to the bindings.
    Keys = 3,
    /// The touchpad's frames and gestures.
    Touchpad = 4,
    /// Taking and handling the thread's messages (the hooks are called
    /// from here as well).
    Messages = 5,
}

impl Part {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Part::Renewing,
            2 => Part::Control,
            3 => Part::Keys,
            4 => Part::Touchpad,
            5 => Part::Messages,
            _ => Part::Waiting,
        }
    }
}

impl std::fmt::Display for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Part::Waiting => "waiting for messages",
            Part::Renewing => "renewing the hooks",
            Part::Control => "registering bindings",
            Part::Keys => "matching what the hooks heard to the bindings",
            Part::Touchpad => "the touchpad's gestures",
            Part::Messages => "taking its messages",
        })
    }
}

/// The time since the process's first look at it, in microseconds; never 0
/// (0 stands for "none").
fn now_us() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    (epoch.elapsed().as_micros() as u64).max(1)
}

/// One hook's counts.
pub struct Hook {
    calls: AtomicU64,
    /// When the call that goes on now began (0: none goes on).
    inside_since: AtomicU64,
    /// The slowest call since the last summary, in microseconds.
    slowest: AtomicU64,
}

impl Hook {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            inside_since: AtomicU64::new(0),
            slowest: AtomicU64::new(0),
        }
    }

    /// A call begins; it is over when what is returned is dropped.
    pub fn enter(&'static self) -> Inside {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let since = now_us();
        self.inside_since.store(since, Ordering::Relaxed);
        Inside { hook: self, since }
    }
}

/// A call of a hook that goes on.
pub struct Inside {
    hook: &'static Hook,
    since: u64,
}

impl Drop for Inside {
    fn drop(&mut self) {
        let took = now_us().saturating_sub(self.since);
        self.hook.slowest.fetch_max(took, Ordering::Relaxed);
        self.hook.inside_since.store(0, Ordering::Relaxed);
    }
}

/// The keyboard's hook.
pub static KEYBOARD: Hook = Hook::new();
/// The mouse's hook.
pub static MOUSE: Hook = Hook::new();

static PART: AtomicU8 = AtomicU8::new(Part::Waiting as u8);
static PART_SINCE: AtomicU64 = AtomicU64::new(0);
static RENEWED_AT: AtomicU64 = AtomicU64::new(0);

/// The input thread is in `part` from now on.
pub fn part(part: Part) {
    PART_SINCE.store(now_us(), Ordering::Relaxed);
    PART.store(part as u8, Ordering::Relaxed);
}

/// The hooks were put in (anew).
pub fn renewed() {
    RENEWED_AT.store(now_us(), Ordering::Relaxed);
}

/// Start the watch, once a process: `who` names the hooks' owner in the
/// log ("the window manager's", "the helper's").
pub fn start(who: &'static str) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    now_us();
    let spawned = std::thread::Builder::new()
        .name("gevrek-input-watch".to_string())
        .spawn(move || watch(who));
    if let Err(error) = spawned {
        log::warn!("nothing watches whether {who} input hooks still hear: {error}");
    }
}

fn watch(who: &'static str) {
    let mut stuck = Stuck::default();
    let mut slow_told: [u64; 2] = [0, 0];
    let mut summary_at = Instant::now();
    let mut last_input = last_input_tick();
    let mut deaf = false;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let now = now_us();

        let part = Part::from_u8(PART.load(Ordering::Relaxed));
        let since = PART_SINCE.load(Ordering::Relaxed);
        if let Some(long) = stuck.due(part, since, now) {
            log::warn!(
                "{who} input thread has been busy with {part} for {:.1} s: it cannot renew its hooks, and every key and movement of the mouse waits for it until Windows takes them away",
                long.as_secs_f64()
            );
        }

        for (index, (hook, name)) in [(&KEYBOARD, "keyboard"), (&MOUSE, "mouse")]
            .into_iter()
            .enumerate()
        {
            let inside = hook.inside_since.load(Ordering::Relaxed);
            if inside != 0
                && inside != slow_told[index]
                && Duration::from_micros(now.saturating_sub(inside)) >= HOOK_SLOW
            {
                slow_told[index] = inside;
                log::warn!(
                    "a call of {who} {name} hook has gone on for {} ms: Windows takes away a hook that answers this slowly",
                    now.saturating_sub(inside) / 1000
                );
            }
        }

        if summary_at.elapsed() < SUMMARY_EVERY {
            continue;
        }
        summary_at = Instant::now();
        let keys = KEYBOARD.calls.swap(0, Ordering::Relaxed);
        let moves = MOUSE.calls.swap(0, Ordering::Relaxed);
        let slowest_keys = KEYBOARD.slowest.swap(0, Ordering::Relaxed);
        let slowest_moves = MOUSE.slowest.swap(0, Ordering::Relaxed);
        let renewed = RENEWED_AT.load(Ordering::Relaxed);
        let renewed = if renewed == 0 {
            "never".to_string()
        } else {
            format!("{} s ago", now.saturating_sub(renewed) / 1_000_000)
        };
        log::debug!(
            "the last half minute: {who} keyboard hook was called {keys} times (the slowest answer {:.1} ms), its mouse hook {moves} times ({:.1} ms); the hooks were put in {renewed}",
            slowest_keys as f64 / 1000.0,
            slowest_moves as f64 / 1000.0,
        );

        let input = last_input_tick();
        let had_input = input.is_some() && input != last_input;
        last_input = input;
        let deaf_now = had_input && keys == 0 && moves == 0;
        if deaf_now != deaf {
            deaf = deaf_now;
            if deaf {
                log::warn!(
                    "the system had input in the last half minute, but {who} hooks were not called once: Windows took them away, or another program's hook keeps the input from them (or the screen was locked, or it all went to windows of programs that run as an administrator); gevrek's keys do nothing meanwhile"
                );
            } else {
                log::info!("{who} hooks hear the input again");
            }
        }
    }
}

/// When the system last had input (`GetLastInputInfo`, its tick count).
fn last_input_tick() -> Option<u32> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: a plain question, into a struct of the size it is told.
    unsafe { GetLastInputInfo(&mut info) }
        .as_bool()
        .then_some(info.dwTime)
}

/// Whether the part that goes on is to be said now.
#[derive(Default)]
struct Stuck {
    /// The part told of last (by when it began), and when it was told.
    told: Option<(u64, u64)>,
}

impl Stuck {
    fn due(&mut self, part: Part, since: u64, now: u64) -> Option<Duration> {
        if part == Part::Waiting || since == 0 {
            return None;
        }
        let long = Duration::from_micros(now.saturating_sub(since));
        let due = match self.told {
            Some((told_since, told_at)) if told_since == since => {
                Duration::from_micros(now.saturating_sub(told_at)) >= STUCK_AGAIN
            }
            _ => long >= STUCK,
        };
        due.then(|| {
            self.told = Some((since, now));
            long
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000;

    #[test]
    fn a_part_that_goes_on_is_said_after_a_second_and_then_every_five() {
        let mut stuck = Stuck::default();
        let since = 10 * SECOND;
        assert_eq!(stuck.due(Part::Messages, since, since + SECOND / 2), None);
        assert!(stuck.due(Part::Messages, since, since + SECOND).is_some());
        assert_eq!(stuck.due(Part::Messages, since, since + 2 * SECOND), None);
        assert!(stuck
            .due(Part::Messages, since, since + 6 * SECOND)
            .is_some());
    }

    #[test]
    fn waiting_is_never_stuck() {
        let mut stuck = Stuck::default();
        assert_eq!(stuck.due(Part::Waiting, SECOND, 100 * SECOND), None);
    }

    #[test]
    fn a_new_part_is_said_on_its_own_time() {
        let mut stuck = Stuck::default();
        assert!(stuck.due(Part::Keys, SECOND, 3 * SECOND).is_some());
        // Another part began at 4 s: a second of it first.
        assert_eq!(
            stuck.due(Part::Messages, 4 * SECOND, 4 * SECOND + SECOND / 2),
            None
        );
        assert!(stuck.due(Part::Messages, 4 * SECOND, 5 * SECOND).is_some());
    }

    #[test]
    fn a_call_of_a_hook_is_counted_and_timed() {
        static HOOK: Hook = Hook::new();
        {
            let _inside = HOOK.enter();
            assert_ne!(HOOK.inside_since.load(Ordering::Relaxed), 0);
        }
        assert_eq!(HOOK.calls.load(Ordering::Relaxed), 1);
        assert_eq!(HOOK.inside_since.load(Ordering::Relaxed), 0);
    }
}
