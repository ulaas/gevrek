//! Telling of what changes in the folders zones show, without looking
//! again and again: the kernel tells (inotify, through the `notify`
//! crate), on a thread of the watcher's own, which wakes whoever hosts
//! the zones.
//!
//! Two halves:
//!
//! - [`Watcher`] holds the kernel's watches and gathers what it says as
//!   [`Change`]s, in order.
//! - [`Folders`] knows which folders are watched for whom and what was
//!   in them, and makes of the changes what the core is told. It is
//!   plain data and can be tried without a kernel.
//!
//! What the kernel says is rougher than what the core wants to hear:
//!
//! - A rename comes as two halves ("moved from", "moved to") with one
//!   number. They become one `ItemRenamed`. A half alone is a file that
//!   left the folder or came into it.
//! - Programs save by writing a hidden file and renaming it over the
//!   real one. That is a change of the real one, not a new item.
//! - A file being written changes many times a second; one drain tells
//!   of it once.
//! - When the kernel lost track (its queue ran over, the folder itself
//!   went), the folder is listed anew and told whole.

use gevrek_zones::backend::{BackendEvent, Location, WatchId};
use gevrek_zones::item::Item;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

/// One thing the kernel told of a watched folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Created(PathBuf),
    Removed(PathBuf),
    /// Written to, or its rights or times changed.
    Changed(PathBuf),
    /// The first half of a rename, with the number its other half has.
    MovedFrom(PathBuf, Option<usize>),
    MovedTo(PathBuf, Option<usize>),
    /// Something was missed, in this folder or (without one) anywhere.
    Lost(Option<PathBuf>),
}

/// How long the second half of a rename is waited for at most.
const OTHER_HALF: std::time::Duration = std::time::Duration::from_millis(25);

/// The kernel's watches.
pub struct Watcher {
    inner: notify::RecommendedWatcher,
    changes: Receiver<Change>,
}

/// What `notify` tells, as changes.
fn changes_of(event: notify::Event) -> Vec<Change> {
    use notify::event::{EventKind, ModifyKind, RenameMode};
    if event.need_rescan() {
        return vec![Change::Lost(event.paths.first().cloned())];
    }
    let tracker = event.attrs.tracker();
    let mut paths = event.paths.into_iter();
    let Some(path) = paths.next() else {
        return Vec::new();
    };
    match event.kind {
        EventKind::Create(_) => vec![Change::Created(path)],
        EventKind::Remove(_) => vec![Change::Removed(path)],
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => vec![Change::MovedFrom(path, tracker)],
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => vec![Change::MovedTo(path, tracker)],
        // Both halves in one, which is told in addition to the halves:
        // they were told already.
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => Vec::new(),
        EventKind::Modify(ModifyKind::Name(_)) => vec![Change::Lost(path.parent().map(Path::to_path_buf))],
        EventKind::Modify(_) => vec![Change::Changed(path)],
        // Opening, reading and closing change nothing that is shown.
        EventKind::Access(_) => Vec::new(),
        EventKind::Any | EventKind::Other => Vec::new(),
    }
}

impl Watcher {
    /// A watcher that calls `wake` (on its own thread) whenever it has
    /// something to tell.
    pub fn new(wake: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let (tell, changes): (Sender<Change>, _) = std::sync::mpsc::channel();
        let inner = notify::recommended_watcher(move |told: notify::Result<notify::Event>| {
            let found = match told {
                Ok(event) => changes_of(event),
                // An error of the watching itself: what it would have
                // told is not known.
                Err(error) => {
                    log::debug!("watching: {error}");
                    vec![Change::Lost(error.paths.first().cloned())]
                }
            };
            if found.is_empty() {
                return;
            }
            for change in found {
                let _ = tell.send(change);
            }
            wake();
        })?;
        Ok(Self { inner, changes })
    }

    /// Watch a folder itself, not the folders in it.
    pub fn add(&mut self, folder: &Path) -> notify::Result<()> {
        notify::Watcher::watch(&mut self.inner, folder, notify::RecursiveMode::NonRecursive)
    }

    pub fn remove(&mut self, folder: &Path) {
        let _ = notify::Watcher::unwatch(&mut self.inner, folder);
    }

    /// What was told since this was last asked, in order.
    ///
    /// The two halves of a rename are told one after the other by the
    /// watcher's thread, and whoever was woken by the first may be here
    /// before the second is. So where the last thing told is a first
    /// half, its other half is waited for, for a moment: the kernel
    /// queues both at once, so it is either all but there or (the file
    /// left the folder) never comes.
    pub fn take(&mut self) -> Vec<Change> {
        let mut changes: Vec<Change> = self.changes.try_iter().collect();
        while matches!(changes.last(), Some(Change::MovedFrom(..))) {
            match self.changes.recv_timeout(OTHER_HALF) {
                Ok(change) => changes.push(change),
                Err(_) => break,
            }
            changes.extend(self.changes.try_iter());
        }
        changes
    }
}

/// One watch that was asked for.
#[derive(Debug, Clone)]
struct Watched {
    /// As it was asked for: events name it so.
    location: Location,
    folder: PathBuf,
}

/// The folders that are watched, and what is known to be in them.
#[derive(Debug, Default)]
pub struct Folders {
    watches: BTreeMap<WatchId, Watched>,
    /// The paths of the items each folder was last known to hold.
    known: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// The folders that were read anew since this was last asked: the
    /// kernel lost track of them, or they changed or went themselves.
    shaken: Vec<PathBuf>,
}

/// What reading the system gives [`Folders`]: the item at a path (none
/// where nothing that is shown is there), and everything in a folder.
pub trait Look {
    fn item(&self, path: &Path) -> Option<Item>;
    fn list(&self, folder: &Path) -> Vec<Item>;
    /// The id an item at this path has or had.
    fn id(&self, path: &Path) -> gevrek_zones::ids::ItemId;
}

impl Folders {
    /// Note a watch. The answer is whether the folder was not watched
    /// before, and so has to be given to the kernel.
    pub fn add(&mut self, watch: WatchId, location: Location, folder: PathBuf, look: &impl Look) -> bool {
        let new = !self.known.contains_key(&folder);
        if new {
            let there = look.list(&folder).into_iter().filter_map(|item| item.path.map(PathBuf::from));
            self.known.insert(folder.clone(), there.collect());
        }
        self.watches.insert(watch, Watched { location, folder });
        new
    }

    /// Note a watch of a folder that was just read (off this thread):
    /// `items` is what the reading found.
    pub fn adopt(&mut self, watch: WatchId, location: Location, folder: PathBuf, items: &[Item]) {
        let there = items.iter().filter_map(|item| item.path.as_ref().map(PathBuf::from)).collect();
        self.known.insert(folder.clone(), there);
        self.watches.insert(watch, Watched { location, folder });
    }

    /// The folders that were read anew since this was last asked. One of
    /// them may be gone: whoever watches it for a portal goes to it again.
    pub fn take_shaken(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.shaken)
    }

    /// Forget a watch. The answer is its folder when nobody watches that
    /// any more, and the kernel need not either.
    pub fn remove(&mut self, watch: WatchId) -> Option<PathBuf> {
        let gone = self.watches.remove(&watch)?;
        if self.watches.values().any(|other| other.folder == gone.folder) {
            return None;
        }
        self.known.remove(&gone.folder);
        Some(gone.folder)
    }

    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    /// The locations a folder is watched as.
    fn locations(&self, folder: &Path) -> Vec<Location> {
        let mut found: Vec<Location> = Vec::new();
        for watched in self.watches.values().filter(|watched| watched.folder == folder) {
            if !found.contains(&watched.location) {
                found.push(watched.location.clone());
            }
        }
        found
    }

    /// Make of the kernel's changes what the core is told.
    pub fn translate(&mut self, changes: Vec<Change>, look: &impl Look) -> Vec<BackendEvent> {
        let mut out = Vec::new();
        let mut changes = changes.into_iter().peekable();
        while let Some(change) = changes.next() {
            match change {
                Change::Created(path) | Change::Changed(path) => self.appeared(&path, look, &mut out),
                Change::Removed(path) => self.went(&path, look, &mut out),
                Change::MovedFrom(from, number) => {
                    // The other half follows at once where there is one.
                    let pair = matches!(changes.peek(), Some(Change::MovedTo(_, other)) if number.is_some() && *other == number);
                    match changes.next_if(|_| pair) {
                        Some(Change::MovedTo(to, _)) => self.moved(&from, &to, look, &mut out),
                        _ => self.went(&from, look, &mut out),
                    }
                }
                Change::MovedTo(path, _) => self.appeared(&path, look, &mut out),
                Change::Lost(folder) => {
                    let folders: Vec<PathBuf> = match folder {
                        Some(folder) if self.known.contains_key(&folder) => vec![folder],
                        Some(inside) => inside
                            .parent()
                            .filter(|folder| self.known.contains_key(*folder))
                            .map(|folder| vec![folder.to_path_buf()])
                            .unwrap_or_else(|| self.known.keys().cloned().collect()),
                        None => self.known.keys().cloned().collect(),
                    };
                    for folder in folders {
                        self.relist(&folder, look, &mut out);
                    }
                }
            }
        }
        once(out)
    }

    /// The watched folder a path lies directly in, if any. A path that
    /// is a watched folder itself is told apart by the caller.
    fn folder_of(&self, path: &Path) -> Option<PathBuf> {
        let folder = path.parent()?;
        self.known.contains_key(folder).then(|| folder.to_path_buf())
    }

    /// Something is at `path` that was not, or is other than it was.
    fn appeared(&mut self, path: &Path, look: &impl Look, out: &mut Vec<BackendEvent>) {
        if self.known.contains_key(path) {
            // The watched folder itself changed (its rights, say): what
            // is in it may read differently now.
            return self.relist(path, look, out);
        }
        let Some(folder) = self.folder_of(path) else {
            return;
        };
        let Some(item) = look.item(path) else {
            // Nothing to show there (a hidden file, or gone again). If
            // something was shown, it is not any more.
            return self.went(path, look, out);
        };
        let was_known = !self.known.entry(folder.clone()).or_default().insert(path.to_path_buf());
        for location in self.locations(&folder) {
            out.push(if was_known {
                BackendEvent::ItemChanged { location, item: item.clone() }
            } else {
                BackendEvent::ItemAdded { location, item: item.clone() }
            });
        }
    }

    /// What was at `path` is not there any more.
    fn went(&mut self, path: &Path, look: &impl Look, out: &mut Vec<BackendEvent>) {
        if self.known.contains_key(path) {
            return self.relist(path, look, out);
        }
        let Some(folder) = self.folder_of(path) else {
            return;
        };
        if !self.known.entry(folder.clone()).or_default().remove(path) {
            return;
        }
        for location in self.locations(&folder) {
            out.push(BackendEvent::ItemRemoved { location, item: look.id(path) });
        }
    }

    /// What was at `from` is at `to` now.
    fn moved(&mut self, from: &Path, to: &Path, look: &impl Look, out: &mut Vec<BackendEvent>) {
        let same_folder = from.parent() == to.parent();
        let from_known = self.folder_of(from).is_some_and(|folder| self.known[&folder].contains(from));
        let to_known = self.folder_of(to).is_some_and(|folder| self.known[&folder].contains(to));
        let item = look.item(to);
        // One item with another name, in one folder: the only case that
        // is a rename to the core. Everything else is said as what left
        // and what came (or changed: a save by renaming over the file).
        if let (true, true, false, Some(item), Some(folder)) = (same_folder, from_known, to_known, &item, self.folder_of(to)) {
            let known = self.known.entry(folder.clone()).or_default();
            known.remove(from);
            known.insert(to.to_path_buf());
            for location in self.locations(&folder) {
                out.push(BackendEvent::ItemRenamed { location, from: look.id(from), item: item.clone() });
            }
            return;
        }
        self.went(from, look, out);
        self.appeared(to, look, out);
    }

    /// Tell a folder whole, as it is now.
    fn relist(&mut self, folder: &Path, look: &impl Look, out: &mut Vec<BackendEvent>) {
        let items = look.list(folder);
        let there = items.iter().filter_map(|item| item.path.as_ref().map(PathBuf::from)).collect();
        self.known.insert(folder.to_path_buf(), there);
        if !self.shaken.iter().any(|known| known == folder) {
            self.shaken.push(folder.to_path_buf());
        }
        for location in self.locations(folder) {
            out.push(BackendEvent::Listed { location, items: items.clone() });
        }
    }
}

/// The events with what one drain need not say twice left out: a change
/// of an item that the same drain already told as it is now (as new, as
/// changed, as renamed, or in a listing). Every event carries the item
/// as it was read during the drain, so the earlier one said everything.
fn once(events: Vec<BackendEvent>) -> Vec<BackendEvent> {
    let mut out: Vec<BackendEvent> = Vec::with_capacity(events.len());
    for event in events {
        if let BackendEvent::ItemChanged { location, item } = &event {
            let last = out.iter().rev().find(|earlier| touches(earlier, location, &item.id));
            if last.is_some_and(|earlier| !matches!(earlier, BackendEvent::ItemRemoved { .. })) {
                continue;
            }
        }
        out.push(event);
    }
    out
}

/// Whether an event tells the listener at `location` of the item `id`.
fn touches(event: &BackendEvent, location: &Location, id: &gevrek_zones::ids::ItemId) -> bool {
    match event {
        BackendEvent::ItemAdded { location: at, item } | BackendEvent::ItemChanged { location: at, item } => {
            at == location && &item.id == id
        }
        BackendEvent::ItemRemoved { location: at, item } => at == location && item == id,
        BackendEvent::ItemRenamed { location: at, from, item } => at == location && (from == id || &item.id == id),
        BackendEvent::Listed { location: at, .. } => at == location,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::ids::ItemId;
    use std::cell::RefCell;

    /// A pretend file system: the paths that are there.
    #[derive(Default)]
    struct Pretend(RefCell<BTreeSet<PathBuf>>);

    impl Pretend {
        fn put(&self, path: &str) {
            self.0.borrow_mut().insert(PathBuf::from(path));
        }
        fn take(&self, path: &str) {
            self.0.borrow_mut().remove(Path::new(path));
        }
    }

    impl Look for Pretend {
        fn item(&self, path: &Path) -> Option<Item> {
            let hidden = path.file_name()?.to_str()?.starts_with('.');
            if hidden || !self.0.borrow().contains(path) {
                return None;
            }
            Some(Item::file(path.to_str()?))
        }
        fn list(&self, folder: &Path) -> Vec<Item> {
            self.0
                .borrow()
                .iter()
                .filter(|path| path.parent() == Some(folder))
                .filter_map(|path| self.item(path))
                .collect()
        }
        fn id(&self, path: &Path) -> ItemId {
            ItemId::new(path.to_string_lossy())
        }
    }

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    fn desk() -> (Pretend, Folders) {
        let files = Pretend::default();
        files.put("/desk/old.txt");
        let mut folders = Folders::default();
        assert!(folders.add(WatchId(1), Location::Desktop, path("/desk"), &files));
        (files, folders)
    }

    /// The events as short words, to compare.
    fn said(events: &[BackendEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                BackendEvent::ItemAdded { item, .. } => format!("added {}", item.name),
                BackendEvent::ItemRemoved { item, .. } => format!("removed {}", gevrek_zones::paths::file_name(item.as_str())),
                BackendEvent::ItemChanged { item, .. } => format!("changed {}", item.name),
                BackendEvent::ItemRenamed { from, item, .. } => {
                    format!("renamed {} to {}", gevrek_zones::paths::file_name(from.as_str()), item.name)
                }
                BackendEvent::Listed { items, .. } => format!("listed {}", items.len()),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn what_comes_goes_and_changes_is_told_once() {
        let (files, mut folders) = desk();
        // A download: made, written to many times.
        files.put("/desk/new.bin");
        let events = folders.translate(
            vec![
                Change::Created(path("/desk/new.bin")),
                Change::Changed(path("/desk/new.bin")),
                Change::Changed(path("/desk/new.bin")),
                Change::Changed(path("/desk/old.txt")),
                Change::Changed(path("/desk/new.bin")),
                Change::Changed(path("/desk/old.txt")),
            ],
            &files,
        );
        assert_eq!(said(&events), ["added new.bin", "changed old.txt"]);
        // Later writes are changes of what is known.
        let events = folders.translate(vec![Change::Changed(path("/desk/new.bin"))], &files);
        assert_eq!(said(&events), ["changed new.bin"]);
        files.take("/desk/new.bin");
        let events = folders.translate(vec![Change::Removed(path("/desk/new.bin"))], &files);
        assert_eq!(said(&events), ["removed new.bin"]);
        // Told twice by the kernel, told once; what was never known is
        // not told as gone; a file made and gone at once never was.
        let events = folders.translate(
            vec![
                Change::Removed(path("/desk/new.bin")),
                Change::Removed(path("/desk/never.txt")),
                Change::Created(path("/desk/blink.tmp")),
                Change::Removed(path("/desk/blink.tmp")),
                Change::Created(path("/elsewhere/x.txt")),
            ],
            &files,
        );
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn the_two_halves_of_a_rename_become_one() {
        let (files, mut folders) = desk();
        files.take("/desk/old.txt");
        files.put("/desk/new.txt");
        let events = folders.translate(
            vec![
                Change::MovedFrom(path("/desk/old.txt"), Some(7)),
                Change::MovedTo(path("/desk/new.txt"), Some(7)),
            ],
            &files,
        );
        assert_eq!(said(&events), ["renamed old.txt to new.txt"]);
        match &events[0] {
            BackendEvent::ItemRenamed { location, from, item } => {
                assert_eq!(location, &Location::Desktop);
                assert_eq!(from.as_str(), "/desk/old.txt");
                assert_eq!(item.id.as_str(), "/desk/new.txt");
            }
            other => panic!("{other:?}"),
        }
        // A half alone: the file left the folder, or came into it.
        files.take("/desk/new.txt");
        files.put("/desk/arrived.txt");
        let events = folders.translate(
            vec![
                Change::MovedFrom(path("/desk/new.txt"), Some(8)),
                Change::MovedTo(path("/desk/arrived.txt"), Some(9)),
            ],
            &files,
        );
        assert_eq!(said(&events), ["removed new.txt", "added arrived.txt"]);
    }

    #[test]
    fn a_save_by_renaming_over_the_file_is_a_change_and_hidden_files_are_not_items() {
        let (files, mut folders) = desk();
        // An editor writes `.old.txt.swp` and renames it over `old.txt`.
        files.put("/desk/.old.txt.swp");
        let events = folders.translate(vec![Change::Created(path("/desk/.old.txt.swp"))], &files);
        assert!(events.is_empty(), "{events:?}");
        files.take("/desk/.old.txt.swp");
        let events = folders.translate(
            vec![
                Change::Changed(path("/desk/.old.txt.swp")),
                Change::MovedFrom(path("/desk/.old.txt.swp"), Some(3)),
                Change::MovedTo(path("/desk/old.txt"), Some(3)),
            ],
            &files,
        );
        assert_eq!(said(&events), ["changed old.txt"]);
        // An item renamed over another: one goes, the other changed.
        files.put("/desk/draft.txt");
        folders.translate(vec![Change::Created(path("/desk/draft.txt"))], &files);
        files.take("/desk/draft.txt");
        let events = folders.translate(
            vec![
                Change::MovedFrom(path("/desk/draft.txt"), Some(4)),
                Change::MovedTo(path("/desk/old.txt"), Some(4)),
            ],
            &files,
        );
        assert_eq!(said(&events), ["removed draft.txt", "changed old.txt"]);
        // Hidden by a rename, it goes; shown by one, it comes.
        files.take("/desk/old.txt");
        files.put("/desk/.old.txt");
        let events = folders.translate(
            vec![
                Change::MovedFrom(path("/desk/old.txt"), Some(5)),
                Change::MovedTo(path("/desk/.old.txt"), Some(5)),
            ],
            &files,
        );
        assert_eq!(said(&events), ["removed old.txt"]);
        files.take("/desk/.old.txt");
        files.put("/desk/old.txt");
        let events = folders.translate(
            vec![
                Change::MovedFrom(path("/desk/.old.txt"), Some(6)),
                Change::MovedTo(path("/desk/old.txt"), Some(6)),
            ],
            &files,
        );
        assert_eq!(said(&events), ["added old.txt"]);
    }

    #[test]
    fn what_was_missed_is_made_up_for_by_telling_the_folder_whole() {
        let (files, mut folders) = desk();
        files.put("/desk/unseen.txt");
        let events = folders.translate(vec![Change::Lost(None)], &files);
        assert_eq!(said(&events), ["listed 2"]);
        // Known from the listing: a write is a change, not a new item.
        let events = folders.translate(vec![Change::Changed(path("/desk/unseen.txt"))], &files);
        assert_eq!(said(&events), ["changed unseen.txt"]);
        // The folder itself went.
        files.take("/desk/unseen.txt");
        files.take("/desk/old.txt");
        let events = folders.translate(vec![Change::Removed(path("/desk"))], &files);
        assert_eq!(said(&events), ["listed 0"]);
    }

    #[test]
    fn a_folder_watched_twice_is_told_to_each_and_let_go_with_the_last() {
        let (files, mut folders) = desk();
        let as_folder = Location::Directory { path: "/desk".to_string() };
        assert!(!folders.add(WatchId(2), as_folder.clone(), path("/desk"), &files));
        files.put("/desk/a.txt");
        let events = folders.translate(vec![Change::Created(path("/desk/a.txt"))], &files);
        assert_eq!(
            events,
            [
                BackendEvent::ItemAdded { location: Location::Desktop, item: Item::file("/desk/a.txt") },
                BackendEvent::ItemAdded { location: as_folder, item: Item::file("/desk/a.txt") },
            ]
        );
        assert_eq!(folders.remove(WatchId(1)), None);
        assert_eq!(folders.remove(WatchId(2)), Some(path("/desk")));
        assert!(folders.is_empty());
        assert_eq!(folders.remove(WatchId(2)), None);
        assert!(folders.translate(vec![Change::Created(path("/desk/b.txt"))], &files).is_empty());
    }

    /// Wait until the kernel has told something and things are quiet.
    fn told(watcher: &mut Watcher, woken: &Receiver<()>) -> Vec<Change> {
        let mut changes = Vec::new();
        let mut wait = std::time::Duration::from_secs(10);
        while woken.recv_timeout(wait).is_ok() {
            changes.extend(watcher.take());
            wait = std::time::Duration::from_millis(300);
        }
        changes.extend(watcher.take());
        changes
    }

    #[test]
    // (inotify's own: the two halves of a rename, paired. FSEvents does not
    // pair them: on macOS the change is `Lost`, and the folder is read again.)
    #[cfg_attr(target_os = "macos", ignore = "inotify's own")]
    fn the_kernel_tells_of_a_file_made_renamed_and_removed() {
        // On the system's own temporary folder: a folder of Windows
        // mounted into WSL tells nothing.
        let root = crate::zones::linux::testing::folder("watch");
        let root = std::fs::canonicalize(&root).expect("real");
        let (wake, woken) = std::sync::mpsc::channel();
        let mut watcher = Watcher::new(move || {
            let _ = wake.send(());
        })
        .expect("a watcher");
        watcher.add(&root).expect("watched");

        std::fs::write(root.join("a.txt"), "a").expect("written");
        let changes = told(&mut watcher, &woken);
        assert_eq!(changes.first(), Some(&Change::Created(root.join("a.txt"))), "{changes:?}");
        assert!(changes.iter().all(|change| matches!(change, Change::Created(_) | Change::Changed(_))));

        std::fs::rename(root.join("a.txt"), root.join("b.txt")).expect("renamed");
        let changes = told(&mut watcher, &woken);
        match &changes[..] {
            [Change::MovedFrom(from, one), Change::MovedTo(to, other)] => {
                assert_eq!((from, to), (&root.join("a.txt"), &root.join("b.txt")));
                assert!(one.is_some() && one == other);
            }
            other => panic!("{other:?}"),
        }

        std::fs::remove_file(root.join("b.txt")).expect("removed");
        assert_eq!(told(&mut watcher, &woken), [Change::Removed(root.join("b.txt"))]);

        // Let go: nothing more is told.
        watcher.remove(&root);
        std::fs::write(root.join("c.txt"), "c").expect("written");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(watcher.take().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
