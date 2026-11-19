//! A list a smart zone shows that is no folder: the files opened lately,
//! the programs that are pinned or started most, the programs that run.
//!
//! Such a list is worked out from something else (the system's record of
//! opened files, gevrek's own count of started programs, the window
//! manager's state), so there is nothing to say of one item changing:
//! the list is read whole and told whole (`BackendEvent::SourceListed`), in its
//! own order, whenever it is other than it was last told.
//!
//! It is read on a thread of the list's own, never on the caller's: the
//! thread sleeps until it is told that the list may be other now, reads,
//! and tells if it is. What tells it is the file system (a watch on the
//! files the list is made of, through `notify`) or whoever holds the
//! list ([`Source::refresh`]). Many tellings while it reads are one more
//! reading. Nothing looks by the clock.

use gevrek_zones::backend::BackendEvent;
use gevrek_zones::item::Item;
use notify::Watcher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// How what is told reaches the backend's events (and wakes the host).
pub type Tell = Arc<dyn Fn(Vec<BackendEvent>) + Send + Sync>;

/// One such list, read and told for as long as this lives.
pub struct Source {
    again: mpsc::Sender<()>,
    known: Arc<Mutex<Option<Vec<Item>>>>,
    /// The next reading is told also where the list is as it was.
    anyway: Arc<AtomicBool>,
    /// The file system's telling of what the list is made of. Ends with
    /// this.
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Source {
    /// A list read by `read` (on a thread of the list's own) now and
    /// whenever something changes in one of the `watched` folders that
    /// `concerns` it (a folder is watched itself, not what is below it; a
    /// folder that is not there is left out). `event` makes of the list
    /// what the core is told (nothing, while nobody asked for the list).
    pub fn new(
        event: impl Fn(Vec<Item>) -> Vec<BackendEvent> + Send + 'static,
        mut read: impl FnMut() -> Vec<Item> + Send + 'static,
        watched: &[PathBuf],
        concerns: impl Fn(&Path) -> bool + Send + 'static,
        tell: Tell,
    ) -> Self {
        let (again, asked) = mpsc::channel::<()>();
        let known: Arc<Mutex<Option<Vec<Item>>>> = Arc::new(Mutex::new(None));
        let kept = Arc::clone(&known);
        let anyway = Arc::new(AtomicBool::new(false));
        let asked_anyway = Arc::clone(&anyway);
        let started = std::thread::Builder::new().name("gevrek-zones-source".to_string()).spawn(move || {
            // (Ends when nobody can ask any more: the source went.)
            while asked.recv().is_ok() {
                // Everything asked meanwhile is answered by this one
                // reading.
                while asked.try_recv().is_ok() {}
                let anyway = asked_anyway.swap(false, Ordering::SeqCst);
                let items = read();
                let mut known = kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if anyway || known.as_ref() != Some(&items) {
                    *known = Some(items.clone());
                    // (Told while the list is held, so that two readings
                    // are told in the order they were made in.)
                    tell(event(items));
                }
            }
        });
        if let Err(error) = started {
            log::warn!("zones: a list is not read: no thread: {error}");
        }
        let _ = again.send(());
        let wake = again.clone();
        let handler = move |event: notify::Result<notify::Event>| {
            let ours = match event {
                Err(_) => true,
                // (Being read is no change.)
                Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => false,
                Ok(event) => event.need_rescan() || event.paths.iter().any(|path| concerns(path)),
            };
            if ours {
                let _ = wake.send(());
            }
        };
        let mut watcher = notify::recommended_watcher(handler).ok();
        if let Some(watcher) = &mut watcher {
            for folder in watched.iter().filter(|folder| folder.is_dir()) {
                if let Err(error) = watcher.watch(folder, notify::RecursiveMode::NonRecursive) {
                    log::debug!("zones: {} is not watched: {error}", folder.display());
                }
            }
        }
        Self { again, known, anyway, _watcher: watcher }
    }

    /// The list may be other than it was: read it again (on its thread)
    /// and tell if it is.
    pub fn refresh(&self) {
        let _ = self.again.send(());
    }

    /// Read the list again and tell it also where it is as it was: for
    /// whoever begins to ask for it now.
    pub fn tell_again(&self) {
        self.anyway.store(true, Ordering::SeqCst);
        self.refresh();
    }

    /// The list as it was last read; nothing before the first reading.
    pub fn items(&self) -> Vec<Item> {
        self.known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::config::SmartSource;
    use std::time::Duration;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("gevrek-zones-source-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::canonicalize(&folder).expect("a real folder")
    }

    #[test]
    fn a_list_is_told_whole_when_it_is_other_than_it_was_and_only_then() {
        let root = folder("list");
        let at = SmartSource::Recent;
        let (told, tellings) = mpsc::channel();
        let told = Mutex::new(told);
        let tell: Tell = Arc::new(move |events| {
            let _ = told.lock().expect("the channel").send(events);
        });
        // The list is the names of the text files in the folder.
        let read_from = root.clone();
        let read = move || {
            let mut names: Vec<String> = std::fs::read_dir(&read_from)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".txt"))
                .collect();
            names.sort();
            names.iter().map(|name| Item::file(name)).collect()
        };
        let told_as = at.clone();
        let source = Source::new(
            move |items| vec![BackendEvent::SourceListed { source: told_as.clone(), items }],
            read,
            std::slice::from_ref(&root),
            |path| path.extension().is_some_and(|ending| ending == "txt"),
            tell,
        );
        let next = || -> Vec<String> {
            let events = tellings.recv_timeout(Duration::from_secs(20)).expect("told");
            let [BackendEvent::SourceListed { source, items }] = &events[..] else {
                panic!("one listing at a time: {events:?}");
            };
            assert_eq!(source, &at);
            items.iter().map(|item| item.name.clone()).collect()
        };
        // At the start: as it is, also when that is nothing.
        assert_eq!(next(), Vec::<String>::new());
        std::fs::write(root.join("a.txt"), b"a").expect("written");
        assert_eq!(next(), ["a.txt"]);
        assert_eq!(source.items().len(), 1);
        // What does not concern the list wakes nobody, and a reading
        // that finds the same list tells nothing.
        std::fs::write(root.join("other.bin"), b"b").expect("written");
        source.refresh();
        assert!(tellings.recv_timeout(Duration::from_millis(300)).is_err());
        std::fs::write(root.join("b.txt"), b"b").expect("written");
        assert_eq!(next(), ["a.txt", "b.txt"]);
        std::fs::remove_file(root.join("a.txt")).expect("removed");
        // (A file that is written is told of more than once; the list is
        // told when it differs, so the last telling is what counts.)
        let mut last = next();
        while let Ok(events) = tellings.recv_timeout(Duration::from_millis(300)) {
            if let [BackendEvent::SourceListed { items, .. }] = &events[..] {
                last = items.iter().map(|item| item.name.clone()).collect();
            }
        }
        assert_eq!(last, ["b.txt"]);
        // Asked to tell again, it tells what it has, changed or not.
        source.tell_again();
        assert_eq!(next(), ["b.txt"]);
        drop(source);
        let _ = std::fs::remove_dir_all(&root);
    }
}
