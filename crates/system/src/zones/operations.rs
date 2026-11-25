//! File operations that end later: carried out one after the other on a
//! thread of their own, so that a large copy holds nobody who draws.
//!
//! The caller is answered at once (`ZonesBackend::begin_operation`); the
//! end of each operation is told as `BackendEvent::OperationDone` with
//! the caller's number, saying exactly what `perform` would have
//! answered. On the way, how far it has come is told now and then
//! (`OperationProgress`). An operation can be called off
//! ([`Operations::cancel`]): one that has not begun is not begun, one
//! that is under way is stopped where its system allows, and either way
//! its end is told (`BackendError::Cancelled`, or what came of it if it
//! was too late).
//!
//! They run one after the other, in the order they were asked for: what
//! takes an operation back assumes that the ones before it are done.
//! The thread sleeps in its queue between them.

use super::sources::Tell;
use gevrek_zones::backend::{BackendError, BackendEvent, FileOperation};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// One operation under way, as whoever carries it out sees it: where to
/// say how far it is, and whether it was called off.
pub struct Watched {
    request: u64,
    tell: Tell,
    called_off: Arc<Mutex<HashSet<u64>>>,
    /// How far it was when that was last told.
    told_at: AtomicU64,
}

impl Watched {
    /// Whether somebody called the operation off.
    pub fn called_off(&self) -> bool {
        locked(&self.called_off).contains(&self.request)
    }

    /// It has come this far, of this much (0: not known). Told on when
    /// it is a hundredth further than last told (or, with the whole not
    /// known, four million further), and at its end.
    pub fn progress(&self, done: u64, total: u64) {
        let step = if total == 0 {
            4_000_000
        } else {
            (total / 100).max(1)
        };
        let last = self.told_at.load(Ordering::Relaxed);
        if done < last.saturating_add(step) && !(done == total && done != last) {
            return;
        }
        self.told_at.store(done, Ordering::Relaxed);
        (self.tell)(vec![BackendEvent::OperationProgress {
            request: self.request,
            done,
            total,
        }]);
    }
}

#[cfg(all(test, windows))]
impl Watched {
    /// An operation as its thread hands it over, called off already or
    /// not: for trying what carries operations out without a thread.
    pub(crate) fn alone(request: u64, tell: Tell, called_off: bool) -> Arc<Self> {
        let off: HashSet<u64> = called_off.then_some(request).into_iter().collect();
        Arc::new(Self {
            request,
            tell,
            called_off: Arc::new(Mutex::new(off)),
            told_at: AtomicU64::new(0),
        })
    }
}

/// Carries one operation out; the answer is what takes it back.
pub type Carry = Box<dyn FnOnce(&Arc<Watched>) -> Result<FileOperation, BackendError> + Send>;

fn locked<T>(shared: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The operations that end later.
pub struct Operations {
    tell: Tell,
    /// The queue of the thread; made with the first operation.
    jobs: Option<mpsc::Sender<(u64, Carry)>>,
    /// Asked for and not ended.
    pending: Arc<Mutex<HashSet<u64>>>,
    called_off: Arc<Mutex<HashSet<u64>>>,
}

impl Operations {
    /// `tell` hands events to the backend (and wakes the host).
    pub fn new(tell: Tell) -> Self {
        Self {
            tell,
            jobs: None,
            pending: Arc::default(),
            called_off: Arc::default(),
        }
    }

    /// Have an operation carried out after those asked for before it.
    /// The answer is whether it was taken (no, where no thread could be
    /// had: the caller then carries it out itself).
    pub fn begin(&mut self, request: u64, carry: Carry) -> bool {
        if self.jobs.is_none() {
            let (jobs, asked) = mpsc::channel::<(u64, Carry)>();
            let (tell, pending, called_off) = (
                Arc::clone(&self.tell),
                Arc::clone(&self.pending),
                Arc::clone(&self.called_off),
            );
            let started = std::thread::Builder::new()
                .name("gevrek-zones-files".to_string())
                .spawn(move || {
                    // (Ends when nobody can ask any more: the backend went.)
                    for (request, carry) in asked {
                        let watched = Arc::new(Watched {
                            request,
                            tell: Arc::clone(&tell),
                            called_off: Arc::clone(&called_off),
                            told_at: AtomicU64::new(0),
                        });
                        let outcome = if watched.called_off() {
                            Err(BackendError::Cancelled)
                        } else {
                            // What failed after it was called off failed
                            // for that; what was done all the same is done.
                            carry(&watched).map_err(|error| {
                                if watched.called_off() {
                                    BackendError::Cancelled
                                } else {
                                    error
                                }
                            })
                        };
                        locked(&pending).remove(&request);
                        locked(&called_off).remove(&request);
                        tell(vec![match outcome {
                            Ok(undo) => BackendEvent::OperationDone {
                                request,
                                undo: Some(undo),
                                error: None,
                            },
                            Err(error) => BackendEvent::OperationDone {
                                request,
                                undo: None,
                                error: Some(error),
                            },
                        }]);
                    }
                });
            if started.is_err() {
                return false;
            }
            self.jobs = Some(jobs);
        }
        locked(&self.pending).insert(request);
        let sent = self
            .jobs
            .as_ref()
            .is_some_and(|jobs| jobs.send((request, carry)).is_ok());
        if !sent {
            locked(&self.pending).remove(&request);
        }
        sent
    }

    /// Call an operation off. Nothing for one that has ended (its end
    /// was told) or was never begun.
    pub fn cancel(&self, request: u64) {
        if locked(&self.pending).contains(&request) {
            locked(&self.called_off).insert(request);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn operations_end_later_in_order_and_can_be_called_off() {
        let (told, tellings) = mpsc::channel();
        let told = Mutex::new(told);
        let tell: Tell = Arc::new(move |events| {
            for event in events {
                let _ = told.lock().expect("the channel").send(event);
            }
        });
        let mut operations = Operations::new(tell);
        let next = || {
            tellings
                .recv_timeout(Duration::from_secs(20))
                .expect("told")
        };
        let back = |name: &str| FileOperation::Trash {
            path: name.to_string(),
        };

        // The first waits until the test lets it go on; it says how far
        // it is, and stops when it is called off.
        let (go_on, gate) = mpsc::channel::<()>();
        let asked = std::time::Instant::now();
        assert!(operations.begin(
            1,
            Box::new(move |watched| {
                watched.progress(0, 1000);
                watched.progress(5, 1000);
                watched.progress(500, 1000);
                let _ = gate.recv();
                if watched.called_off() {
                    return Err(BackendError::Refused {
                        what: "stopped half way".to_string(),
                    });
                }
                Ok(back("one"))
            }),
        ));
        assert!(operations.begin(2, Box::new(move |_| Ok(back("two")))));
        assert!(operations.begin(3, Box::new(move |_| Ok(back("three")))));
        assert!(asked.elapsed() < Duration::from_millis(100));
        // (Told when a hundredth further: not for the step from 0 to 5.)
        assert_eq!(
            next(),
            BackendEvent::OperationProgress {
                request: 1,
                done: 500,
                total: 1000
            }
        );
        // Called off: the one under way and one that waits. One that
        // was never asked for is nobody's.
        operations.cancel(1);
        operations.cancel(3);
        operations.cancel(99);
        drop(go_on);
        assert_eq!(
            next(),
            BackendEvent::OperationDone {
                request: 1,
                undo: None,
                error: Some(BackendError::Cancelled)
            }
        );
        assert_eq!(
            next(),
            BackendEvent::OperationDone {
                request: 2,
                undo: Some(back("two")),
                error: None
            }
        );
        assert_eq!(
            next(),
            BackendEvent::OperationDone {
                request: 3,
                undo: None,
                error: Some(BackendError::Cancelled)
            }
        );
        // Called off when it has ended: nothing more is told, and the
        // next one is carried out as any.
        operations.cancel(2);
        assert!(operations.begin(
            4,
            Box::new(move |watched| {
                watched.progress(7, 7);
                Err(BackendError::NotFound {
                    what: "four".to_string(),
                })
            })
        ));
        assert_eq!(
            next(),
            BackendEvent::OperationProgress {
                request: 4,
                done: 7,
                total: 7
            }
        );
        assert_eq!(
            next(),
            BackendEvent::OperationDone {
                request: 4,
                undo: None,
                error: Some(BackendError::NotFound {
                    what: "four".to_string()
                })
            }
        );
        assert!(tellings.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
