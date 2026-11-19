//! A folder a portal shows: gone to, read and watched off the caller's
//! thread, and told as away when it cannot be reached.
//!
//! A folder may be on another machine (an NFS or SMB mount), on a stick
//! that is pulled out, or simply deleted. Going to such a folder can take
//! as long as the kernel is willing to wait, and nothing can end that
//! wait from outside. So nobody who draws ever goes: a thread of the
//! folder's own does, and what it found comes back as an [`Answer`] that
//! the backend takes when it is woken.
//!
//! - **There.** The folder is given to the kernel to tell of (a watcher
//!   of its own, begun before the reading so that nothing between the
//!   two is missed), read, and told whole.
//! - **Slow.** It did not answer within the patience it is given
//!   ([`crate::zones::reach::PATIENCE`]): told as away. Its thread goes
//!   on waiting; when the folder answers after all, it is told whole.
//! - **Away.** It is not there. The nearest folder above it that is
//!   there is watched for the next name on the way down, so that its
//!   coming (back) is noticed without looking by the clock.
//!
//! A watched folder's own going needs no folder above it: the kernel
//! tells a folder that it was deleted or moved (`IN_DELETE_SELF`,
//! `IN_MOVE_SELF`), the backend reads it anew, and goes to it again.
//!
//! What no folder above can tell (a mount that comes back: mounting
//! changes no folder) is tried again when somebody asks
//! (`SystemBackend::retry_unreachable`, the core's `Relist`).

use super::watch::{Change, Watcher};
use super::System;
use crate::zones::reach::{Reach, PATIENCE};
use gevrek_zones::backend::{BackendError, Location, WatchId};
use gevrek_zones::item::Item;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// How a folder is gone to. Other than the system's own in tests, which
/// play a mount that does not answer.
pub struct How {
    /// How long a folder may take before it is told as away.
    pub patience: Duration,
    /// Whether the folder at this path is there. This is the first going
    /// to it, and the one that takes long when a machine is gone.
    pub reach: Box<dyn Fn(&Path) -> bool + Send + Sync>,
}

impl Default for How {
    fn default() -> Self {
        Self { patience: PATIENCE, reach: Box::new(|path| path.is_dir()) }
    }
}

/// A folder that is watched for a portal, as the backend keeps it.
pub struct Portal {
    /// As it was asked for: events name it so.
    pub location: Location,
    pub folder: PathBuf,
    pub reach: Reach,
    /// A thread is going to the folder now.
    pub opening: bool,
    /// Something happened meanwhile: go once more when it is back.
    pub again: bool,
    /// The kernel's telling of the folder, while it is there.
    pub watcher: Option<Watcher>,
    /// The telling of a folder above it while it is away, and the path
    /// whose coming is waited for there.
    pub sentinel: Option<(Watcher, PathBuf)>,
}

impl Portal {
    pub fn new(location: Location, folder: PathBuf) -> Self {
        Self {
            location,
            folder,
            reach: Reach::Opening,
            opening: false,
            again: false,
            watcher: None,
            sentinel: None,
        }
    }

    /// Whether what the folder above told means that this folder may be
    /// there now.
    pub fn may_be_back(&mut self) -> bool {
        let Some((watcher, next)) = &mut self.sentinel else {
            return false;
        };
        watcher.take().iter().any(|change| match change {
            Change::Created(path)
            | Change::Removed(path)
            | Change::Changed(path)
            | Change::MovedFrom(path, _)
            | Change::MovedTo(path, _) => path == next,
            Change::Lost(_) => true,
        })
    }
}

/// What a going to a folder found.
pub enum Outcome {
    /// It does not answer yet; its thread still waits.
    Slow,
    /// It is there: the kernel's telling of it, and what is in it.
    There { watcher: Watcher, items: Vec<Item> },
    /// It is not there (or cannot be read), with the watch on a folder
    /// above it where one could be had.
    Away { why: String, sentinel: Option<(Watcher, PathBuf)> },
    /// A folder nobody watches, read once.
    Once { location: Location, listed: Result<Vec<Item>, BackendError> },
}

/// What a thread found, for the backend.
pub struct Answer {
    /// The watch it went for; none for a folder read once.
    pub watch: Option<WatchId>,
    pub outcome: Outcome,
}

/// Wakes the host.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

fn watcher(wake: &Wake) -> notify::Result<Watcher> {
    let wake = Arc::clone(wake);
    Watcher::new(move || wake())
}

/// Go to the folder on this thread: watch it, read it; or, where it is
/// not there, watch for it above.
fn read(folder: &Path, system: &System, how: &How, wake: &Wake) -> Outcome {
    loop {
        let mut why = format!("{} is not there", folder.display());
        if (how.reach)(folder) {
            // The telling begins before the reading, so that nothing
            // that happens between the two is missed.
            let begun = watcher(wake).and_then(|mut watcher| watcher.add(folder).map(|()| watcher));
            match begun.map_err(|error| error.to_string()).and_then(|watcher| {
                system.list(folder).map(|items| (watcher, items)).map_err(|error| error.to_string())
            }) {
                Ok((watcher, items)) => return Outcome::There { watcher, items },
                Err(error) => why = format!("{}: {error}", folder.display()),
            }
        }
        // The nearest folder above that is there, and the next name on
        // the way down from it.
        let mut next = folder;
        let sentinel = loop {
            let Some(above) = next.parent() else {
                break None;
            };
            if (how.reach)(above) {
                let watching = watcher(wake).and_then(|mut watcher| watcher.add(above).map(|()| watcher));
                break watching.ok().map(|watcher| (watcher, next.to_path_buf()));
            }
            next = above;
        };
        // What happened before the folder above was listened to went
        // unheard: looked at once more, the folder is still away, or it
        // is gone to again.
        if sentinel.is_some() && (how.reach)(folder) {
            continue;
        }
        return Outcome::Away { why, sentinel };
    }
}

/// Go to a folder for a watch, on threads of its own. What it finds is
/// sent to `answers` (first that it is slow, where it is), and the host
/// is woken each time.
pub fn go(
    watch: WatchId,
    folder: PathBuf,
    system: Arc<System>,
    how: Arc<How>,
    answers: mpsc::Sender<Answer>,
    wake: Wake,
) -> std::io::Result<()> {
    std::thread::Builder::new().name("gevrek-zones-portal".to_string()).spawn(move || {
        let (found, finding) = mpsc::channel();
        let (reading, reader_how, reader_wake) = (folder.clone(), Arc::clone(&how), Arc::clone(&wake));
        // The going itself is on a second thread, so that this one can
        // say "it does not answer" while that one still waits.
        let reader = std::thread::Builder::new().name("gevrek-zones-portal-read".to_string()).spawn(move || {
            let _ = found.send(read(&reading, &system, &reader_how, &reader_wake));
        });
        let lost = || Outcome::Away { why: format!("{}: no answer", folder.display()), sentinel: None };
        let outcome = match reader {
            Err(_) => lost(),
            Ok(_) => match finding.recv_timeout(how.patience) {
                Ok(outcome) => outcome,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = answers.send(Answer { watch: Some(watch), outcome: Outcome::Slow });
                    wake();
                    finding.recv().unwrap_or_else(|_| lost())
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => lost(),
            },
        };
        let _ = answers.send(Answer { watch: Some(watch), outcome });
        wake();
    })?;
    Ok(())
}

/// Read a folder nobody watches once, on a thread of its own.
pub fn once(location: Location, folder: PathBuf, system: Arc<System>, answers: mpsc::Sender<Answer>, wake: Wake) -> bool {
    std::thread::Builder::new()
        .name("gevrek-zones-list".to_string())
        .spawn(move || {
            let listed = system.list(&folder).map_err(|error| super::refusal(error, &folder.to_string_lossy()));
            let _ = answers.send(Answer { watch: None, outcome: Outcome::Once { location, listed } });
            wake();
        })
        .is_ok()
}
