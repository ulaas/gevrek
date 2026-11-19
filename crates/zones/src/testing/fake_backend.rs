//! A backend for tests: folders are maps, displays are a list, and what a
//! real backend would hand to the system is written down. A test says
//! what happens (a file comes, a display goes) and the fake tells of it as
//! a real backend would: as events, and only of what is watched.

use crate::backend::{
    BackendError, BackendEvent, DragActions, DropPayload, FileOperation, IconHandle, IconRequest,
    IconSize, Location, WatchId, ZonesBackend,
};
use crate::geometry::{Display, ReservedArea};
use crate::ids::{DisplayId, ItemId};
use crate::item::{Item, ItemFacts};
use crate::paths;
use gevrek_model::geometry::{Point, Rect};
use std::collections::BTreeMap;

/// The desktop's own folder on the fake.
pub const DESKTOP: &str = "/desk";

#[derive(Debug, Default)]
pub struct FakeBackend {
    /// Every folder with what is in it, the desktop's among them.
    folders: BTreeMap<String, BTreeMap<ItemId, Item>>,
    /// What was put into the trash, by the path it had.
    trash: BTreeMap<String, Item>,
    watches: BTreeMap<WatchId, Location>,
    events: Vec<BackendEvent>,
    numbers: u64,
    displays: Vec<Display>,
    reserved: Vec<ReservedArea>,
    /// Items opened, in order; with the program, where one was named.
    pub opened: Vec<(ItemId, Option<String>)>,
    /// Menus shown.
    pub menus: Vec<(Vec<ItemId>, Point)>,
    /// Drags begun.
    pub drags: Vec<(Vec<ItemId>, DragActions)>,
    /// File operations carried out.
    pub performed: Vec<FileOperation>,
    /// What a drag over a zone carries now.
    pub dragged_over: Option<DropPayload>,
    /// The next file operation fails with this.
    pub fail_next: Option<BackendError>,
    /// Whether the system's own icons are written down and not put back.
    pub journal_open: bool,
    /// How often anything was asked of the backend through the trait.
    pub calls: usize,
}

impl FakeBackend {
    /// A desktop with one display of 1920 by 1080 and nothing on it.
    pub fn new() -> Self {
        let mut backend = Self::default();
        backend.folders.insert(DESKTOP.to_string(), BTreeMap::new());
        backend.displays.push(Display {
            primary: true,
            ..Display::new("fake-1", Rect::new(0, 0, 1920, 1080))
        });
        backend
    }

    /// How many watches are running.
    pub fn watches(&self) -> usize {
        self.watches.len()
    }

    fn number(&mut self) -> u64 {
        self.numbers += 1;
        self.numbers
    }

    /// The folder's own key, however the path is written.
    fn folder_key(&self, folder: &str) -> Option<String> {
        self.folders.keys().find(|key| paths::same_folder(key, folder)).cloned()
    }

    /// The location a watched folder is known by, as it was asked for.
    fn watched(&self, folder: &str) -> Option<Location> {
        self.watches
            .values()
            .find(|location| match location {
                Location::Desktop => paths::same_folder(folder, DESKTOP),
                Location::Directory { path } => paths::same_folder(folder, path),
            })
            .cloned()
    }

    fn folder_of(location: &Location) -> &str {
        match location {
            Location::Desktop => DESKTOP,
            Location::Directory { path } => path,
        }
    }

    fn find(&self, path: &str) -> Option<(String, Item)> {
        let folder = self.folder_key(paths::parent(path)?)?;
        let item = self.folders[&folder]
            .values()
            .find(|item| item.path.as_deref() == Some(path))?
            .clone();
        Some((folder, item))
    }

    /// Put an item into a folder, as if something outside had.
    pub fn add(&mut self, folder: &str, item: Item) {
        let key = self.folder_key(folder).unwrap_or_else(|| folder.to_string());
        self.folders.entry(key).or_default().insert(item.id.clone(), item.clone());
        if let Some(location) = self.watched(folder) {
            self.events.push(BackendEvent::ItemAdded { location, item });
        }
    }

    /// A file appears on the desktop.
    pub fn add_file(&mut self, name: &str) -> Item {
        let item = Item::file(&paths::join(DESKTOP, name));
        self.add(DESKTOP, item.clone());
        item
    }

    /// An empty folder is there (for a portal to show).
    pub fn add_folder(&mut self, path: &str) {
        self.folders.entry(path.to_string()).or_default();
    }

    /// An item goes, as if something outside had deleted it.
    pub fn remove(&mut self, id: &ItemId) -> bool {
        let folders: Vec<String> = self.folders.keys().cloned().collect();
        for folder in folders {
            if self.folders.get_mut(&folder).and_then(|items| items.remove(id)).is_some() {
                if let Some(location) = self.watched(&folder) {
                    self.events.push(BackendEvent::ItemRemoved { location, item: id.clone() });
                }
                return true;
            }
        }
        false
    }

    /// The facts of an item are others (it was written to).
    pub fn change(&mut self, id: &ItemId, facts: ItemFacts) {
        let folders: Vec<String> = self.folders.keys().cloned().collect();
        for folder in folders {
            let Some(item) = self.folders.get_mut(&folder).and_then(|items| items.get_mut(id)) else {
                continue;
            };
            item.facts = facts;
            let item = item.clone();
            if let Some(location) = self.watched(&folder) {
                self.events.push(BackendEvent::ItemChanged { location, item });
            }
            return;
        }
    }

    /// What is in a folder now.
    pub fn in_folder(&self, folder: &str) -> Vec<Item> {
        self.folder_key(folder)
            .map(|key| self.folders[&key].values().cloned().collect())
            .unwrap_or_default()
    }

    /// The displays are these now.
    pub fn set_displays(&mut self, displays: Vec<Display>) {
        self.displays = displays;
        self.displays_changed();
    }

    /// A display comes; one of that id that is there is replaced.
    pub fn plug(&mut self, display: Display) {
        self.displays.retain(|existing| existing.id != display.id);
        self.displays.push(display);
        self.displays_changed();
    }

    /// A display goes.
    pub fn unplug(&mut self, id: &DisplayId) {
        self.displays.retain(|display| &display.id != id);
        self.displays_changed();
    }

    /// The room kept at the displays' sides is this now.
    pub fn set_reserved(&mut self, reserved: Vec<ReservedArea>) {
        self.reserved = reserved;
        self.displays_changed();
    }

    fn displays_changed(&mut self) {
        self.events.push(BackendEvent::Displays {
            displays: self.displays.clone(),
            reserved: self.reserved.clone(),
        });
    }

    /// Put `item` into `folder` under a name nothing there has, and tell
    /// of it. Returns the path it got.
    fn arrive(&mut self, folder: &str, mut item: Item) -> String {
        let taken: Vec<String> = self.folders[folder].values().map(|item| item.name.clone()).collect();
        let mut name = item.name.clone();
        let mut count = 2;
        while taken.contains(&name) {
            name = match item.name.rsplit_once('.') {
                Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({count}).{extension}"),
                _ => format!("{} ({count})", item.name),
            };
            count += 1;
        }
        let path = paths::join(folder, &name);
        item.id = ItemId::new(path.clone());
        item.path = Some(path.clone());
        item.name = name;
        self.add(folder, item);
        path
    }

    fn missing(what: &str) -> BackendError {
        BackendError::NotFound { what: what.to_string() }
    }
}

impl ZonesBackend for FakeBackend {
    fn list(&mut self, location: &Location) -> Result<Vec<Item>, BackendError> {
        self.calls += 1;
        let folder = Self::folder_of(location);
        let key = self.folder_key(folder).ok_or_else(|| Self::missing(folder))?;
        Ok(self.folders[&key].values().cloned().collect())
    }

    fn watch(&mut self, location: &Location) -> Result<WatchId, BackendError> {
        self.calls += 1;
        let folder = Self::folder_of(location);
        if self.folder_key(folder).is_none() {
            return Err(Self::missing(folder));
        }
        let watch = WatchId(self.number());
        self.watches.insert(watch, location.clone());
        Ok(watch)
    }

    fn unwatch(&mut self, watch: WatchId) {
        self.calls += 1;
        self.watches.remove(&watch);
    }

    fn drain_events(&mut self) -> Vec<BackendEvent> {
        self.calls += 1;
        std::mem::take(&mut self.events)
    }

    fn facts(&mut self, item: &ItemId) -> Option<ItemFacts> {
        self.calls += 1;
        self.folders.values().find_map(|items| items.get(item)).map(|item| item.facts.clone())
    }

    fn request_icon(&mut self, item: &ItemId, _size: IconSize) -> IconRequest {
        self.calls += 1;
        let request = IconRequest(self.number());
        let known = self.folders.values().any(|items| items.contains_key(item));
        let icon = known.then_some(IconHandle(request.0));
        self.events.push(BackendEvent::IconReady { request, icon });
        request
    }

    fn request_thumbnail(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.calls += 1;
        // Only pictures have one here.
        let picture = self
            .facts(item)
            .and_then(|facts| facts.extension)
            .is_some_and(|extension| matches!(extension.as_str(), "png" | "jpg"));
        if picture {
            return self.request_icon(item, size);
        }
        let request = IconRequest(self.number());
        self.events.push(BackendEvent::IconReady { request, icon: None });
        request
    }

    fn open(&mut self, item: &ItemId) -> Result<(), BackendError> {
        self.calls += 1;
        self.open_with(item, None)
    }

    fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
        self.calls += 1;
        if self.facts(item).is_none() {
            return Err(Self::missing(item.as_str()));
        }
        self.opened.push((item.clone(), program.map(str::to_string)));
        Ok(())
    }

    fn context_menu(&mut self, items: &[ItemId], at: Point) -> Result<(), BackendError> {
        self.calls += 1;
        self.menus.push((items.to_vec(), at));
        Ok(())
    }

    fn perform(&mut self, operation: &FileOperation) -> Result<FileOperation, BackendError> {
        self.calls += 1;
        if let Some(error) = self.fail_next.take() {
            return Err(error);
        }
        let undo = match operation {
            FileOperation::Move { from, to_folder } => {
                let target = self.folder_key(to_folder).ok_or_else(|| Self::missing(to_folder))?;
                let (folder, item) = self.find(from).ok_or_else(|| Self::missing(from))?;
                self.remove(&item.id);
                let path = self.arrive(&target, item);
                FileOperation::Move { from: path, to_folder: folder }
            }
            FileOperation::Copy { from, to_folder } => {
                let target = self.folder_key(to_folder).ok_or_else(|| Self::missing(to_folder))?;
                let (_, item) = self.find(from).ok_or_else(|| Self::missing(from))?;
                FileOperation::Trash { path: self.arrive(&target, item) }
            }
            FileOperation::Rename { path, new_name } => {
                let (folder, item) = self.find(path).ok_or_else(|| Self::missing(path))?;
                if self.folders[&folder].values().any(|other| &other.name == new_name) {
                    return Err(BackendError::Exists { what: new_name.clone() });
                }
                let new_path = paths::join(&folder, new_name);
                let renamed = Item {
                    id: ItemId::new(new_path.clone()),
                    name: new_name.clone(),
                    path: Some(new_path.clone()),
                    facts: ItemFacts {
                        extension: crate::item::extension_of(new_name),
                        ..item.facts.clone()
                    },
                    ..item.clone()
                };
                if let Some(items) = self.folders.get_mut(&folder) {
                    items.remove(&item.id);
                    items.insert(renamed.id.clone(), renamed.clone());
                }
                if let Some(location) = self.watched(&folder) {
                    self.events.push(BackendEvent::ItemRenamed {
                        location,
                        from: item.id.clone(),
                        item: renamed,
                    });
                }
                FileOperation::Rename { path: new_path, new_name: item.name }
            }
            FileOperation::NewFolder { parent, name } => {
                let folder = self.folder_key(parent).ok_or_else(|| Self::missing(parent))?;
                let path = self.arrive(&folder, Item { name: name.clone(), ..Item::folder(&paths::join(parent, name)) });
                self.folders.entry(path.clone()).or_default();
                FileOperation::Trash { path }
            }
            FileOperation::Trash { path } => {
                let (_, item) = self.find(path).ok_or_else(|| Self::missing(path))?;
                self.remove(&item.id);
                self.trash.insert(path.clone(), item);
                FileOperation::Restore { path: path.clone() }
            }
            FileOperation::Restore { path } => {
                let item = self.trash.remove(path).ok_or_else(|| Self::missing(path))?;
                let folder = paths::parent(path)
                    .and_then(|folder| self.folder_key(folder))
                    .ok_or_else(|| Self::missing(path))?;
                FileOperation::Trash { path: self.arrive(&folder, item) }
            }
        };
        self.performed.push(operation.clone());
        Ok(undo)
    }

    fn drop_payload(&mut self) -> Option<DropPayload> {
        self.calls += 1;
        self.dragged_over.clone()
    }

    fn start_drag(&mut self, items: &[ItemId], allowed: DragActions) -> Result<(), BackendError> {
        self.calls += 1;
        self.drags.push((items.to_vec(), allowed));
        Ok(())
    }

    fn displays(&mut self) -> Vec<Display> {
        self.calls += 1;
        self.displays.clone()
    }

    fn reserved_areas(&mut self) -> Vec<ReservedArea> {
        self.calls += 1;
        self.reserved.clone()
    }

    fn journal_system_icons(&mut self) -> Result<(), BackendError> {
        self.calls += 1;
        self.journal_open = true;
        Ok(())
    }

    fn system_icons_to_restore(&mut self) -> bool {
        self.calls += 1;
        self.journal_open
    }

    fn restore_system_icons(&mut self) -> Result<(), BackendError> {
        self.calls += 1;
        self.journal_open = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_is_watched_is_told_of() {
        let mut backend = FakeBackend::new();
        backend.add_file("unseen.txt");
        assert_eq!(backend.drain_events(), []);
        let watch = backend.watch(&Location::Desktop).unwrap();
        let item = backend.add_file("seen.txt");
        assert_eq!(
            backend.drain_events(),
            [BackendEvent::ItemAdded { location: Location::Desktop, item: item.clone() }]
        );
        assert_eq!(backend.list(&Location::Desktop).unwrap().len(), 2);
        backend.unwatch(watch);
        backend.remove(&item.id);
        assert_eq!(backend.drain_events(), []);
        assert!(backend.watch(&Location::Directory { path: "/nowhere".to_string() }).is_err());
    }

    #[test]
    fn every_file_operation_says_what_takes_it_back_and_that_does() {
        let mut backend = FakeBackend::new();
        backend.add_folder("/archive");
        let item = backend.add_file("a.txt");
        let path = item.path.clone().unwrap();
        let before = (backend.in_folder(DESKTOP), backend.in_folder("/archive"));
        for operation in [
            FileOperation::Move { from: path.clone(), to_folder: "/archive".to_string() },
            FileOperation::Copy { from: path.clone(), to_folder: "/archive".to_string() },
            FileOperation::Rename { path: path.clone(), new_name: "b.md".to_string() },
            FileOperation::NewFolder { parent: DESKTOP.to_string(), name: "New".to_string() },
            FileOperation::Trash { path: path.clone() },
        ] {
            let undo = backend.perform(&operation).unwrap();
            assert_ne!(
                (backend.in_folder(DESKTOP), backend.in_folder("/archive")),
                before,
                "{operation:?} did nothing"
            );
            backend.perform(&undo).unwrap();
            assert_eq!(
                (backend.in_folder(DESKTOP), backend.in_folder("/archive")),
                before,
                "{undo:?} did not take {operation:?} back"
            );
        }
    }

    #[test]
    fn a_file_moved_onto_one_of_its_name_gets_another_and_the_way_back_knows() {
        let mut backend = FakeBackend::new();
        backend.add_folder("/archive");
        backend.add("/archive", Item::file("/archive/a.txt"));
        let item = backend.add_file("a.txt");
        let operation = FileOperation::Move { from: item.path.unwrap(), to_folder: "/archive".to_string() };
        let undo = backend.perform(&operation).unwrap();
        assert_eq!(
            undo,
            FileOperation::Move { from: "/archive/a (2).txt".to_string(), to_folder: DESKTOP.to_string() }
        );
        assert_ne!(Some(undo), operation.reversed());
        // A failure that was asked for happens once.
        backend.fail_next = Some(BackendError::Refused { what: "in use".to_string() });
        assert!(backend.perform(&operation).is_err());
        assert!(matches!(backend.perform(&operation), Err(BackendError::NotFound { .. })));
    }

    #[test]
    fn icons_come_as_handles_and_a_journal_is_open_until_put_back() {
        let mut backend = FakeBackend::new();
        let picture = backend.add_file("a.png");
        let text = backend.add_file("a.txt");
        let size = IconSize { size: 48, scale: 1.0 };
        let icon = backend.request_icon(&text.id, size);
        let thumbnail = backend.request_thumbnail(&picture.id, size);
        let none = backend.request_thumbnail(&text.id, size);
        let answers: Vec<(IconRequest, bool)> = backend
            .drain_events()
            .into_iter()
            .filter_map(|event| match event {
                BackendEvent::IconReady { request, icon } => Some((request, icon.is_some())),
                _ => None,
            })
            .collect();
        assert_eq!(answers, [(icon, true), (thumbnail, true), (none, false)]);
        assert!(!backend.system_icons_to_restore());
        backend.journal_system_icons().unwrap();
        assert!(backend.system_icons_to_restore());
        backend.restore_system_icons().unwrap();
        assert!(!backend.system_icons_to_restore());
    }
}
