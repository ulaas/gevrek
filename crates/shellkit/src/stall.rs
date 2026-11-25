//! What the shell's one thread is busy with, said in the log where it is
//! busy for long.
//!
//! A shell does all on one thread: it reads what the compositor (or
//! Windows) tells it, handles the messages of its other threads, and
//! draws. While one of these takes long nothing else is done; on Wayland
//! the compositor's words meanwhile pile up in the connection, and where
//! they no longer fit the compositor cuts the shell off (the shell ends
//! with a broken pipe, and is started again). So:
//!
//! - a part of a pass that took longer than [`SLOW`] is said when it is
//!   over (a warning);
//! - a part that is still going on after [`STUCK`] is said by a thread of
//!   its own while it goes on, and again every [`STUCK_AGAIN`]: so that a
//!   shell that never comes back says what it was doing.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A part of a pass longer than this is said when it is over.
pub const SLOW: Duration = Duration::from_millis(250);
/// A part still going on after this is said while it goes on.
pub const STUCK: Duration = Duration::from_secs(1);
/// And again after each of these.
pub const STUCK_AGAIN: Duration = Duration::from_secs(5);

/// While something is going on, the watching thread looks this often.
const LOOK: Duration = Duration::from_millis(500);
/// With nothing going on it sleeps until something begins, and this long
/// at the most.
const REST: Duration = Duration::from_secs(30);

/// A part of the shell's pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Handling what the compositor (or Windows) told.
    Events,
    /// Handling the messages of the shell's other threads.
    Messages,
    /// Opening, closing and changing surfaces as was asked.
    CarryOut,
    /// Answering screen readers.
    Access,
    /// Drawing one surface: its number and its size in logical pixels.
    Draw { surface: u32, size: (u32, u32) },
    /// Something the loop names itself.
    Other(&'static str),
}

impl std::fmt::Display for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Part::Events => f.write_str("what the system told"),
            Part::Messages => f.write_str("the messages of its other threads"),
            Part::CarryOut => f.write_str("opening and changing surfaces"),
            Part::Access => f.write_str("what screen readers asked"),
            Part::Draw {
                surface,
                size: (width, height),
            } => {
                write!(f, "drawing surface {surface} ({width}x{height})")
            }
            Part::Other(what) => f.write_str(what),
        }
    }
}

struct Busy {
    part: Part,
    since: Instant,
    /// When it was last said while going on.
    told: Option<Instant>,
}

/// The watch of one shell's thread. Cheap to tell: a lock and a time.
#[derive(Clone)]
pub struct Watch {
    busy: Arc<Mutex<Option<Busy>>>,
    /// The watching thread sleeps until something begins (a shell at rest
    /// is busy with nothing, and nothing is to be watched): whoever
    /// begins something wakes it.
    asleep: Arc<std::sync::atomic::AtomicBool>,
    watcher: Option<std::thread::Thread>,
}

impl Watch {
    /// A watch with its thread, which ends with the last clone.
    pub fn start() -> Self {
        let busy: Arc<Mutex<Option<Busy>>> = Arc::new(Mutex::new(None));
        let asleep = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watched = Arc::downgrade(&busy);
        let sleeping = Arc::clone(&asleep);
        let spawned = std::thread::Builder::new().name("gevrek-shell-watch".to_string()).spawn(move || {
            while let Some(busy) = watched.upgrade() {
                let at_rest = {
                    let mut busy = busy.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(busy) = busy.as_mut() {
                        if let Some(long) = still_busy(busy, Instant::now()) {
                            log::warn!(
                                "the shell has been busy with {} for {:.1} s: it reads nothing meanwhile",
                                busy.part,
                                long.as_secs_f64()
                            );
                        }
                    }
                    // (Said under the lock: whoever begins something
                    // after this sees it, and wakes the thread.)
                    if busy.is_none() {
                        sleeping.store(true, std::sync::atomic::Ordering::Release);
                    }
                    busy.is_none()
                };
                drop(busy);
                if at_rest {
                    // Until something begins; and looked at now and then
                    // all the same, so that the thread ends with its
                    // shell. (It woke five times a second for the life
                    // of the shell.)
                    std::thread::park_timeout(REST);
                } else {
                    std::thread::sleep(LOOK);
                }
            }
        });
        let watcher = match spawned {
            Ok(thread) => Some(thread.thread().clone()),
            Err(error) => {
                log::warn!("nothing watches whether the shell is stuck: {error}");
                None
            }
        };
        Self {
            busy,
            asleep,
            watcher,
        }
    }

    /// `part` begins.
    pub fn enter(&self, part: Part) {
        *self.lock() = Some(Busy {
            part,
            since: Instant::now(),
            told: None,
        });
        if self.asleep.swap(false, std::sync::atomic::Ordering::AcqRel) {
            if let Some(watcher) = &self.watcher {
                watcher.unpark();
            }
        }
    }

    /// What began last is over.
    pub fn leave(&self) {
        let Some(busy) = self.lock().take() else {
            return;
        };
        let took = busy.since.elapsed();
        if took >= SLOW {
            log::warn!(
                "the shell was busy with {} for {} ms",
                busy.part,
                took.as_millis()
            );
        }
    }

    /// `run`, as `part`.
    pub fn during<T>(&self, part: Part, run: impl FnOnce() -> T) -> T {
        self.enter(part);
        let done = run();
        self.leave();
        done
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Busy>> {
        self.busy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// How long `busy` has gone on, where it is to be said now.
fn still_busy(busy: &mut Busy, now: Instant) -> Option<Duration> {
    let long = now.saturating_duration_since(busy.since);
    let due = match busy.told {
        None => long >= STUCK,
        Some(told) => now.saturating_duration_since(told) >= STUCK_AGAIN,
    };
    due.then(|| {
        busy.told = Some(now);
        long
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_that_goes_on_is_said_after_a_second_and_then_every_five() {
        let since = Instant::now();
        let mut busy = Busy {
            part: Part::Events,
            since,
            told: None,
        };
        assert_eq!(
            still_busy(&mut busy, since + Duration::from_millis(900)),
            None
        );
        assert_eq!(still_busy(&mut busy, since + STUCK), Some(STUCK));
        assert_eq!(still_busy(&mut busy, since + Duration::from_secs(3)), None);
        assert_eq!(
            still_busy(&mut busy, since + STUCK + STUCK_AGAIN),
            Some(STUCK + STUCK_AGAIN)
        );
    }

    #[test]
    fn what_was_left_is_not_busy_any_more() {
        let watch = Watch::start();
        watch.during(
            Part::Draw {
                surface: 3,
                size: (2560, 1440),
            },
            || {},
        );
        assert!(watch.lock().is_none());
        assert_eq!(
            Part::Draw {
                surface: 3,
                size: (2560, 1440)
            }
            .to_string(),
            "drawing surface 3 (2560x1440)"
        );
    }
}
