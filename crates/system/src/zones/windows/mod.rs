//! The zones on Windows: `gevrek_zones::backend::ZonesBackend`, answered
//! by the shell of Windows.
//!
//! gevrek runs beside Explorer here, and everything a zone shows is the
//! shell's: what is on the desktop, what things are called, their
//! pictures, what a double click does, the menu, how files are moved.
//! So the shell is asked for all of it, through the interfaces it gives
//! every program. Nothing runs inside Explorer and none of its windows is
//! touched.
//!
//! - [`shell`]: what is at a place, as the shell's namespace has it, and
//!   the facts of each item. [`facts`] is the part of that which is plain
//!   arithmetic.
//! - [`icons`]: icons and small pictures by handle, made off the thread
//!   that draws.
//! - [`launch`]: opening, "open with", and the shell's own menu.
//! - [`fileops`]: copy, move, rename, new folder, the Recycle Bin and
//!   back, each with what takes it back.
//! - [`watch`], [`seen`]: being told of changes, and telling the core
//!   each thing once.
//! - [`displays`]: the displays by the ids the window manager gives them.
//! - [`dnd`]: what a drag carries.
//! - [`explorer`]: Explorer's own icons, under a journal.
//!
//! # For the one who draws
//!
//! - [`SystemBackend::new`] takes what wakes the host: it is called, from
//!   any thread, whenever `drain_events` has something to give.
//! - A picture that was answered (`BackendEvent::IconReady`) is fetched
//!   with [`SystemBackend::picture`]: plain pixels (`crate::icons::Picture`,
//!   red, green, blue and solidness, not multiplied), already at the
//!   display's scale ([`pixels`] says how large for a size).
//! - The displays come from the window manager:
//!   [`displays_of_state`] reads its state, [`SystemBackend::set_displays`]
//!   takes them. [`SystemBackend::pixels_of`] says where a display is on
//!   the screen, for the surface that covers it.
//! - The window the zones are drawn in is told with
//!   [`SystemBackend::set_owner`]; the menu and what the shell shows
//!   while it works with files belong to it. The menu can also be asked
//!   for at an exact point of the screen with [`launch::show_menu`].
//!
//! # For drag and drop
//!
//! This half knows what is carried; `gevrek_shellkit` has the window
//! that takes drops and the loop that runs while something is dragged.
//!
//! - **Out of a zone.** The core asks `start_drag`; the object to drag is
//!   then fetched once with [`SystemBackend::take_drag`] and handed to
//!   `DoDragDrop` ([`DragOut::object`], with [`dnd::effects_of`] of
//!   [`DragOut::allowed`]). [`dnd::data_object`] makes such an object
//!   for any items, without a backend.
//! - **Onto a zone.** When a drag comes over the surface
//!   (`IDropTarget::DragEnter`), [`dnd::payload_of`] reads its object
//!   into what the core understands, and [`SystemBackend::set_drop`]
//!   keeps it for the core's `drop_payload`; `set_drop(None)` when the
//!   drag leaves or ends. [`dnd::actions_of`] turns the effects Windows
//!   names into the core's actions. Where only the bytes of the shell's
//!   own kind are at hand, [`dnd::items_of_id_list`] says which items
//!   they name.
//!
//! # Explorer's icons
//!
//! `journal_system_icons`, `system_icons_to_restore` and
//! `restore_system_icons` of the trait are the journal ([`explorer`]).
//! Putting the icons away is a call of its own that nothing here makes by
//! itself: [`SystemBackend::hide_system_icons`].

pub mod displays;
pub mod dnd;
pub mod explorer;
pub mod facts;
pub mod fileops;
pub mod icons;
pub mod launch;
pub mod seen;
pub mod shell;
pub mod watch;

pub use displays::displays_of_state;
pub use icons::pixels;

use crate::icons::Picture;
use explorer::{Explorer, Keeper, ShellDesktop};
use gevrek_model::geometry::Point;
use gevrek_zones::backend::{
    BackendError, BackendEvent, DragActions, DropPayload, FileOperation, IconHandle, IconRequest,
    IconSize, Location, WatchId, ZonesBackend,
};
use gevrek_zones::geometry::{Display, ReservedArea};
use gevrek_zones::ids::{DisplayId, ItemId};
use gevrek_zones::item::{Item, ItemFacts};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// What a drag out of a zone carries, for the one who runs the drag.
pub struct DragOut {
    /// The shell's data object for the items.
    pub object: IDataObject,
    pub items: Vec<ItemId>,
    /// What may be done with them.
    pub allowed: DragActions,
}

/// The watching of the desktop.
struct DesktopWatch {
    /// What is known to be on it.
    seen: watch::Shared,
    /// The watches that were asked for (they share the one watching).
    watches: Vec<WatchId>,
    /// The file system's telling of the desktop's own folders. Ends with
    /// this.
    _folders: Vec<watch::Folder>,
}

/// The backend of the zones on Windows.
pub struct SystemBackend {
    news: Arc<watch::News>,
    types: shell::Types,
    icons: icons::Icons,
    files: fileops::FileOperations,
    displays: displays::Displays,
    /// The window that is told of changes. Made when something is first
    /// watched or the displays are first asked for.
    window: Option<watch::Window>,
    /// The desktop is watched.
    desktop: Option<DesktopWatch>,
    folders: BTreeMap<WatchId, watch::Folder>,
    numbers: u64,
    /// What is to be told besides what the watchers and the pictures tell.
    events: Vec<BackendEvent>,
    /// The window the zones are drawn in; 0 while there is none.
    owner: isize,
    dragged_over: Option<DropPayload>,
    dragged_out: Option<DragOut>,
    keeper: Keeper,
    explorer: Box<dyn Explorer>,
}

impl SystemBackend {
    /// The backend of the desktop this program runs on. `wake` is called
    /// (from any thread) when there is something to tell: the moment to
    /// call `drain_events`, which is `Host::pump`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::at(&gevrek_model::paths::state_dir(), Box::new(ShellDesktop::new()), wake)
    }

    /// A backend that keeps its journal under another folder and talks to
    /// another Explorer (one in memory, or the real one read only): for
    /// tests and for looking.
    pub fn at(
        state_dir: &Path,
        explorer: Box<dyn Explorer>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let news = Arc::new(watch::News::default());
        let woken = Arc::clone(&wake);
        news.on_news(Box::new(move || woken()));
        Self {
            news,
            types: shell::Types::default(),
            icons: icons::Icons::new(wake),
            files: fileops::FileOperations::default(),
            displays: displays::Displays::default(),
            window: None,
            desktop: None,
            folders: BTreeMap::new(),
            numbers: 0,
            events: Vec::new(),
            owner: 0,
            dragged_over: None,
            dragged_out: None,
            keeper: Keeper::new(state_dir),
            explorer,
        }
    }

    /// The pixels of a picture that was answered with this handle: four
    /// bytes each (red, green, blue, how solid), not multiplied.
    pub fn picture(&mut self, icon: IconHandle) -> Option<&Picture> {
        self.icons.picture(icon)
    }

    /// The window the zones are drawn in (its `HWND`). The shell's menu
    /// and what the shell shows while it works with files belong to it.
    pub fn set_owner(&mut self, window: isize) {
        self.owner = window;
        self.files.owner = window;
    }

    /// Nothing of the shell's is shown while files are worked with, not
    /// even how far a long copy is. For tests.
    pub fn set_quiet(&mut self, quiet: bool) {
        self.files.quiet = quiet;
    }

    /// The displays and the room kept on them are these now, as the
    /// window manager says (see [`displays_of_state`]). Told to the core
    /// where they changed. From the first call on, Windows itself is not
    /// asked any more which displays there are (only where each one is,
    /// to the pixel).
    pub fn set_displays(&mut self, displays: Vec<Display>, reserved: Vec<ReservedArea>) {
        if self.displays.tell(displays.clone(), reserved.clone()) {
            self.events.push(BackendEvent::Displays { displays, reserved });
            self.news.wake();
        }
    }

    /// Where a display is on the screen, in pixels (x, y, width, height):
    /// where the surface that covers it goes.
    pub fn pixels_of(&mut self, display: &DisplayId) -> Option<(i32, i32, i32, i32)> {
        self.displays.pixels_of(display)
    }

    /// What the drag over a zone carries now; `None` when it left.
    pub fn set_drop(&mut self, payload: Option<DropPayload>) {
        self.dragged_over = payload;
    }

    /// What the drag that `start_drag` began carries, once: for the one
    /// who runs the drag.
    pub fn take_drag(&mut self) -> Option<DragOut> {
        self.dragged_out.take()
    }

    /// Put Explorer's own icons away, because the zones draw every item
    /// now. Refused unless the desktop was written down first
    /// (`journal_system_icons`); taken back by `restore_system_icons`.
    ///
    /// This is the one call that changes something of Explorer's. Nothing
    /// in this crate makes it by itself.
    pub fn hide_system_icons(&mut self) -> Result<(), BackendError> {
        self.keeper.hide(self.explorer.as_mut())
    }

    fn number(&mut self) -> u64 {
        self.numbers += 1;
        self.numbers
    }

    /// The hidden window, made when it is first needed. Without it the
    /// zones still show; they are only not told of changes.
    fn window(&mut self) -> Option<&watch::Window> {
        if self.window.is_none() {
            match watch::Window::open(Arc::clone(&self.news)) {
                Ok(window) => self.window = Some(window),
                Err(error) => log::warn!("zones: {error}"),
            }
        }
        self.window.as_ref()
    }

    /// What is known of the watched places that are `location`.
    fn watched(&self, location: &Location) -> Vec<watch::Shared> {
        let mut found = Vec::new();
        if let (Location::Desktop, Some(desktop)) = (location, &self.desktop) {
            found.push(Arc::clone(&desktop.seen));
        }
        if let Location::Directory { path } = location {
            for folder in self.folders.values() {
                let same = match locked(&folder.seen).location() {
                    Location::Directory { path: watched } => facts::same_path(watched, path),
                    Location::Desktop => false,
                };
                if same {
                    found.push(Arc::clone(&folder.seen));
                }
            }
        }
        found
    }
}

fn locked(seen: &watch::Shared) -> std::sync::MutexGuard<'_, seen::Seen> {
    seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ZonesBackend for SystemBackend {
    fn list(&mut self, location: &Location) -> Result<Vec<Item>, BackendError> {
        let items = shell::list(location, &mut self.types)?;
        // What the core knows from now on is what changes are measured
        // against.
        for seen in self.watched(location) {
            locked(&seen).listed(&items);
        }
        Ok(items)
    }

    fn watch(&mut self, location: &Location) -> Result<WatchId, BackendError> {
        let watch = WatchId(self.number());
        match location {
            Location::Desktop => {
                if let Some(desktop) = &mut self.desktop {
                    desktop.watches.push(watch);
                    return Ok(watch);
                }
                let seen: watch::Shared = Arc::new(Mutex::new(seen::Seen::new(Location::Desktop)));
                // The shell, for what is no file and what programs tell
                // it; the file system, for the files of the desktop's two
                // folders. Either alone still tells most of it, so one
                // that cannot be had is done without.
                let mut told = false;
                match self.window().map(|window| window.listen_to_desktop(&seen)) {
                    Some(Ok(())) => told = true,
                    Some(Err(error)) => log::warn!("zones: {error}"),
                    None => {}
                }
                let mut folders = Vec::new();
                for folder in shell::desktop_folders() {
                    match watch::watch_desktop_folder(&folder, &seen, Arc::clone(&self.news)) {
                        Ok(watching) => folders.push(watching),
                        Err(error) => log::warn!("zones: {error}"),
                    }
                }
                if !told && folders.is_empty() {
                    return Err(BackendError::Refused {
                        what: "the desktop cannot be watched".to_string(),
                    });
                }
                self.desktop = Some(DesktopWatch { seen, watches: vec![watch], _folders: folders });
            }
            Location::Directory { path } => {
                let folder = watch::watch_folder(path, Arc::clone(&self.news))?;
                self.folders.insert(watch, folder);
            }
        }
        Ok(watch)
    }

    fn unwatch(&mut self, watch: WatchId) {
        if self.folders.remove(&watch).is_some() {
            return;
        }
        if let Some(desktop) = &mut self.desktop {
            desktop.watches.retain(|known| *known != watch);
            if desktop.watches.is_empty() {
                self.desktop = None;
                if let Some(window) = &self.window {
                    window.stop_listening_to_desktop();
                }
            }
        }
    }

    fn drain_events(&mut self) -> Vec<BackendEvent> {
        let mut events = self.news.take();
        // A picture of something that changed, went or has another name
        // is not its picture any more.
        for event in &events {
            match event {
                BackendEvent::ItemChanged { item, .. } => self.icons.forget(&item.id),
                BackendEvent::ItemRemoved { item, .. } => self.icons.forget(item),
                BackendEvent::ItemRenamed { from, .. } => self.icons.forget(from),
                _ => {}
            }
        }
        if self.news.displays_changed() && self.displays.system_changed() {
            events.push(BackendEvent::Displays {
                displays: self.displays.displays(),
                reserved: self.displays.reserved(),
            });
        }
        events.append(&mut self.events);
        events.extend(self.icons.drain());
        events
    }

    fn facts(&mut self, item: &ItemId) -> Option<ItemFacts> {
        shell::item_by_id(item, &mut self.types).map(|item| item.facts)
    }

    fn request_icon(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.icons.request(item, size, false)
    }

    fn request_thumbnail(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.icons.request(item, size, true)
    }

    fn open(&mut self, item: &ItemId) -> Result<(), BackendError> {
        launch::open(item)
    }

    fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
        launch::open_with(item, program)
    }

    fn context_menu(&mut self, items: &[ItemId], at: Point) -> Result<(), BackendError> {
        // The point is the core's, in logical units; the menu opens at
        // pixels of the screen. Which display's scale counts is decided
        // with the pointer, which is where a menu is asked for.
        let mut pointer = POINT::default();
        // SAFETY: the point is ours to receive.
        let pointer = unsafe { GetCursorPos(&mut pointer) }
            .is_ok()
            .then_some((pointer.x, pointer.y));
        let at = self.displays.to_pixels(at, pointer);
        launch::show_menu(self.owner, items, at)
    }

    fn perform(&mut self, operation: &FileOperation) -> Result<FileOperation, BackendError> {
        self.files.perform(operation)
    }

    fn drop_payload(&mut self) -> Option<DropPayload> {
        self.dragged_over.clone()
    }

    fn start_drag(&mut self, items: &[ItemId], allowed: DragActions) -> Result<(), BackendError> {
        let object = dnd::data_object(items)?;
        self.dragged_out = Some(DragOut { object, items: items.to_vec(), allowed });
        Ok(())
    }

    fn displays(&mut self) -> Vec<Display> {
        // (From here on Windows says when its displays change.)
        self.window();
        self.displays.displays()
    }

    fn reserved_areas(&mut self) -> Vec<ReservedArea> {
        self.displays.reserved()
    }

    fn desktop_folder(&mut self) -> Option<String> {
        // The person's own, which is the first of the two.
        shell::desktop_folders().into_iter().next()
    }

    fn journal_system_icons(&mut self) -> Result<(), BackendError> {
        self.keeper.journal(self.explorer.as_mut())
    }

    fn system_icons_to_restore(&mut self) -> bool {
        self.keeper.to_restore()
    }

    fn restore_system_icons(&mut self) -> Result<(), BackendError> {
        self.keeper.restore(self.explorer.as_mut())
    }
}

#[cfg(test)]
mod tests;
