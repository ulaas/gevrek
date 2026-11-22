//! Where the windows gevrek parked stood before.
//!
//! A window of a workspace that is not shown waits in a corner of its
//! display (`park`). If gevrek ends without putting it back (it crashed, it
//! was killed), the window would stay there, a sliver nobody can reach. So
//! every parked window is written down here, with the place it came from,
//! and the next gevrek puts back what is written before it adopts anything
//! (`Desktop::restore_journaled_windows`). The same window server id names
//! the same window until it closes, so a window found again is that one.

use gevrek_model::geometry::Rect;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One parked window: its program, and where it stood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parked {
    pub pid: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Parked {
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }
}

/// The journal, kept in memory and written through to its file.
#[derive(Debug, Default)]
pub struct Journal {
    path: Option<PathBuf>,
    parked: BTreeMap<u32, Parked>,
}

impl Journal {
    /// The journal of this instance of gevrek, read from its file.
    pub fn open(path: PathBuf) -> Self {
        let parked = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            parked,
        }
    }

    /// Where this instance keeps it: in the runtime folder, by instance.
    pub fn default_path() -> PathBuf {
        let instance = std::env::var("GEVREK_INSTANCE").unwrap_or_else(|_| "main".to_string());
        gevrek_model::paths::runtime_dir().join(format!("{instance}.parked.json"))
    }

    /// A journal that is not written anywhere (tests).
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn record(&mut self, window: u32, parked: Parked) {
        self.parked.insert(window, parked);
        self.write();
    }

    /// The window is back (or gone): forget it.
    pub fn forget(&mut self, window: u32) -> Option<Parked> {
        let parked = self.parked.remove(&window);
        if parked.is_some() {
            self.write();
        }
        parked
    }

    pub fn get(&self, window: u32) -> Option<Parked> {
        self.parked.get(&window).copied()
    }

    /// Everything written down, to put back.
    pub fn take_all(&mut self) -> Vec<(u32, Parked)> {
        let all = std::mem::take(&mut self.parked).into_iter().collect();
        self.write();
        all
    }

    fn write(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let written = serde_json::to_vec(&self.parked)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                // Written whole and then put in place: a crash in between
                // leaves the old journal, never half of one.
                let partial = path.with_extension("json.partial");
                std::fs::write(&partial, bytes)?;
                std::fs::rename(&partial, path)
            });
        if let Err(error) = written {
            log::warn!("Could not write the parked windows to {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parked(x: i32) -> Parked {
        Parked {
            pid: 7,
            x,
            y: 20,
            width: 300,
            height: 200,
        }
    }

    #[test]
    fn what_is_written_down_is_found_by_the_next_gevrek() {
        let folder = std::env::temp_dir().join(format!("gevrek-journal-{}", std::process::id()));
        let path = folder.join("test.parked.json");
        let mut journal = Journal::open(path.clone());
        journal.record(41, parked(10));
        journal.record(42, parked(500));
        journal.forget(41);

        let mut next = Journal::open(path.clone());
        assert_eq!(next.get(42), Some(parked(500)));
        assert_eq!(next.get(41), None);
        assert_eq!(next.take_all(), vec![(42, parked(500))]);
        // Put back, it is gone from the file too.
        assert!(Journal::open(path).take_all().is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_journal_that_is_not_there_or_spoilt_is_empty() {
        let folder = std::env::temp_dir().join(format!("gevrek-journal-bad-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("spoilt.parked.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(Journal::open(path).take_all().is_empty());
        assert!(Journal::open(folder.join("absent.json")).take_all().is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }
}
