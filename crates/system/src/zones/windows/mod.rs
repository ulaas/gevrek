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
//! - [`portal`]: a folder a portal shows, opened, read and watched off
//!   the caller's thread, and told as away when it cannot be reached.
//! - [`cloud`]: files a cloud keeps, which are never read for a picture.
//! - [`places`], [`recent`]: the person's Downloads folder, and the
//!   shell's record of the files that were opened lately (which is also
//!   where "when was this last opened" comes from).
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

pub mod cloud;
pub mod displays;
pub mod dnd;
pub mod explorer;
pub mod facts;
pub mod fileops;
pub mod icons;
pub mod launch;
pub mod places;
pub mod portal;
pub mod recent;
pub mod seen;
pub mod shell;
pub mod watch;

pub use displays::displays_of_state;
pub use icons::pixels;

use crate::icons::Picture;
use explorer::{Explorer, Keeper, ShellDesktop};
use gevrek_model::geometry::Point;
use crate::zones::smart::Smart;
use crate::zones::sources::Source;
use gevrek_zones::backend::{
    BackendError, BackendEvent, DragActions, DropPayload, FileOperation, IconHandle, IconRequest,
    IconSize, Location, WatchId, ZonesBackend,
};
use gevrek_zones::config::SmartSource;
use gevrek_zones::geometry::{Display, ReservedArea};
use gevrek_zones::ids::{DisplayId, ItemId};
use gevrek_zones::item::{Item, ItemFacts};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
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
    /// The file operations that end later, on a thread of their own.
    operations: crate::zones::operations::Operations,
    displays: displays::Displays,
    /// The window that is told of changes. Made when something is first
    /// watched or the displays are first asked for.
    window: Option<watch::Window>,
    /// The desktop is watched.
    desktop: Option<DesktopWatch>,
    /// The folders portals show, each opened and read on threads of its
    /// own.
    folders: BTreeMap<WatchId, portal::Portal>,
    /// How a folder is gone to.
    how: Arc<portal::How>,
    /// The person's folders smart zones look into.
    places: places::Places,
    /// When the person last opened each file, as the shell wrote it
    /// down: read from the Recent folder, and a fact of every item.
    opened: recent::Opened,
    /// What smart zones look through that is no plain folder.
    smart: Smart,
    /// What is known at every watched place: a file that is opened is a
    /// change of its item wherever it is shown.
    shown: Shown,
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
        let state = gevrek_model::paths::state_dir();
        Self::with(places::Places::of_session(&state), Box::new(ShellDesktop::new()), wake)
    }

    /// A backend that keeps its journal under another folder and talks to
    /// another Explorer (one in memory, or the real one read only): for
    /// tests and for looking.
    /// It knows no Downloads and no Recent folder (see [`Self::with`]
    /// for a backend that is handed some).
    pub fn at(
        state_dir: &Path,
        explorer: Box<dyn Explorer>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let places = places::Places { downloads: None, recent: None, state: state_dir.to_path_buf() };
        Self::with(places, explorer, wake)
    }

    /// A backend with these folders for downloads, for the shell's
    /// record of opened files and for what gevrek keeps: the person's
    /// own, or a test's.
    pub fn with(
        places: places::Places,
        explorer: Box<dyn Explorer>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let news = Arc::new(watch::News::default());
        let woken = Arc::clone(&wake);
        news.on_news(Box::new(move || woken()));
        let opened = recent::Opened::default();
        let telling = Arc::clone(&news);
        let smart = Smart::new(
            &places.state,
            Arc::new(crate::apps::installed),
            crate::start_menu::programs_folders(),
            Arc::new(move |events| telling.tell(events)),
        );
        let state_dir = &places.state;
        let telling = Arc::clone(&news);
        let operations = crate::zones::operations::Operations::new(Arc::new(move |events| telling.tell(events)));
        Self {
            news,
            types: shell::Types::knowing(opened.clone()),
            icons: icons::of_shell(Arc::clone(&smart.known), wake),
            files: fileops::FileOperations::default(),
            operations,
            displays: displays::Displays::default(),
            window: None,
            desktop: None,
            folders: BTreeMap::new(),
            how: Arc::new(portal::How { opened: opened.clone(), ..portal::How::default() }),
            opened,
            smart,
            shown: Shown::default(),
            numbers: 0,
            events: Vec::new(),
            owner: 0,
            dragged_over: None,
            dragged_out: None,
            keeper: Keeper::new(state_dir),
            explorer,
            places,
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
            log::debug!("zones: the displays or the room kept on them changed: {} displays, {} areas kept", displays.len(), reserved.len());
            self.events.push(BackendEvent::Displays { displays, reserved });
            self.news.wake();
        }
    }

    /// Where a display is on the screen, in pixels (x, y, width, height):
    /// where the surface that covers it goes.
    pub fn pixels_of(&mut self, display: &DisplayId) -> Option<(i32, i32, i32, i32)> {
        self.displays.pixels_of(display)
    }

    /// Windows' own name for a display (`\\.\DISPLAY1`), by which the
    /// shell's surfaces are placed. (Not the path the window manager
    /// knows it by: a surface asked for by that was opened on no
    /// display.)
    pub fn name_of(&mut self, display: &DisplayId) -> Option<String> {
        self.displays.name_of(display)
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

    /// The watched folders that cannot be reached now, each with why:
    /// for a word to the person in the zone that shows it.
    pub fn unreachable(&self) -> Vec<(Location, String)> {
        let mut away = Vec::new();
        for portal in self.folders.values() {
            if let crate::zones::reach::Reach::Away { why } = portal.reach() {
                away.push((locked(&portal.seen).location().clone(), why));
            }
        }
        away
    }

    /// Go once more to every watched folder that is away. For when
    /// something happened that no folder can tell of (a machine switched
    /// on again, a network joined), and for the person's own "try again".
    /// Returns at once; a folder that is back is told whole.
    pub fn retry_unreachable(&mut self) {
        for portal in self.folders.values() {
            portal.retry();
        }
    }

    /// Go to folders in another way than the system's own (a test that
    /// plays a machine which does not answer). For folders watched from
    /// now on.
    pub fn set_reach(&mut self, how: portal::How) {
        self.how = Arc::new(portal::How { opened: self.opened.clone(), ..how });
    }

    /// The window manager's state is this now (the JSON the shell has
    /// from it, the same it reads the displays out of). The programs
    /// that run are worked out of it for a zone that shows them
    /// (`SmartSource::RunningApps`), and told when they are others.
    pub fn set_wm_state(&mut self, state: &serde_json::Value) {
        self.smart.set_wm_state(state);
    }

    /// How a command reaches the window manager: the shell's line to
    /// it. Opening an item that is a program which runs sends the
    /// command that gives its window the keyboard.
    pub fn set_commander(&mut self, commander: impl FnMut(serde_json::Value) + 'static) {
        self.smart.set_commander(Box::new(commander));
    }

    /// Pin a program for the zone of launchers (`SmartSource::Pinned`),
    /// or take its pin away. `app` is the program's id, as the launcher
    /// knows it (an item `app:<id>` names it).
    pub fn pin_launcher(&mut self, app: &str, pin: bool) -> Result<(), BackendError> {
        self.smart.pin(app, pin)
    }

    /// The pinned programs, by their ids, in the person's order.
    pub fn pinned_launchers(&self) -> Vec<String> {
        self.smart.pins()
    }

    /// The pinned programs are these, in this order (after the person
    /// dragged one to another place).
    pub fn set_pinned_launchers(&mut self, pins: &[String]) -> Result<(), BackendError> {
        self.smart.set_pins(pins)
    }

    /// Every installed program as an item (`app:<id>`), by name: what
    /// the person chooses from to pin one. It asks the shell about every
    /// shortcut of the Start menu, which takes a moment: not for the
    /// thread that draws.
    pub fn installed_programs(&self) -> Vec<Item> {
        self.smart.installed_items()
    }

    /// Read the installed programs another way (a test's own list).
    pub fn set_programs(&mut self, installed: impl Fn() -> Vec<crate::apps::App> + Send + Sync + 'static) {
        self.smart.set_installed(Arc::new(installed));
    }

    /// How a file a cloud keeps stands with this machine (only in the
    /// cloud, always kept here, here for now); none for any other item.
    /// Read from the file's attributes, which fetches nothing. For a
    /// sign on its icon.
    pub fn cloud_state(&self, item: &ItemId) -> Option<cloud::CloudState> {
        cloud::state_of_path(&shell::plain(item.as_str()))
    }

    /// The shell's record of opened files is read from now on (once,
    /// from the first thing that is watched): for the list of recent
    /// files, and for when each shown file was last opened.
    fn read_recent(&mut self) {
        if self.smart.recent.is_some() {
            return;
        }
        let Some(folder) = self.places.recent.clone() else {
            return;
        };
        let (opened, shown, news) = (self.opened.clone(), self.shown.clone(), Arc::clone(&self.news));
        let from = folder.clone();
        let mut reader = recent::Reader::default();
        let mut types = shell::Types::knowing(opened.clone());
        let read = move || {
            let mut read = reader.read(&from, &opened, &mut types, recent::MOST);
            // A shortcut the shell still held is read again in a moment
            // (on the list's own thread): that it was let go of is not
            // always told, and the list would lack it until the folder
            // changed again.
            let mut opened_anew = std::mem::take(&mut read.opened_anew);
            for _ in 0..40 {
                if !read.held {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                read = reader.read(&from, &opened, &mut types, recent::MOST);
                opened_anew.append(&mut read.opened_anew);
            }
            // A file that was opened has another fact now, wherever a
            // zone shows it.
            news.tell(shown.reread(&opened_anew, &mut types));
            read.items
        };
        self.smart.recent = Some(Source::new(
            crate::zones::smart::told_as(SmartSource::Recent, &self.smart.recent_wanted),
            read,
            std::slice::from_ref(&folder),
            |path| path.extension().is_some_and(|ending| ending.eq_ignore_ascii_case("lnk")),
            self.smart_tell(),
        ));
    }

    /// The downloads folder is read and watched from now on.
    fn read_downloads(&mut self) {
        if self.smart.downloads.is_some() {
            return;
        }
        let Some(folder) = self.places.downloads.clone() else {
            return;
        };
        let location = Location::Directory { path: folder.to_string_lossy().into_owned() };
        let mut types = shell::Types::knowing(self.opened.clone());
        let read = move || crate::zones::smart::newest_first(shell::list(&location, &mut types).unwrap_or_default());
        self.smart.downloads = Some(Source::new(
            crate::zones::smart::told_as(SmartSource::Downloads, &self.smart.downloads_wanted),
            read,
            std::slice::from_ref(&folder),
            |_| true,
            self.smart_tell(),
        ));
    }

    fn smart_tell(&self) -> crate::zones::sources::Tell {
        let news = Arc::clone(&self.news);
        Arc::new(move |events| news.tell(events))
    }

    /// Keep at most this many bytes of pictures in memory
    /// (`crate::zones::pictures`).
    pub fn set_picture_budget(&mut self, bytes: usize) {
        self.icons.set_budget(bytes);
    }

    fn number(&mut self) -> u64 {
        self.numbers += 1;
        self.numbers
    }

    /// The hidden window, made when it is first needed. Without it the
    /// zones still show; they are only not told of changes.
    fn window(&mut self) -> Option<&watch::Window> {
        if self.window.is_none() {
            match watch::Window::open(Arc::clone(&self.news), shell::Types::knowing(self.opened.clone())) {
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

/// How a file operation came out, said in the log.
fn performed(operation: &FileOperation, done: Result<FileOperation, BackendError>) -> Result<FileOperation, BackendError> {
    match &done {
        Ok(undo) => log::debug!("zones: {operation}: done (taken back by {undo})"),
        Err(BackendError::Cancelled) => log::debug!("zones: {operation}: called off"),
        Err(error) => log::debug!("zones: {operation}: failed: {error}"),
    }
    done
}

fn locked(seen: &watch::Shared) -> std::sync::MutexGuard<'_, seen::Seen> {
    seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ZonesBackend for SystemBackend {
    fn list(&mut self, location: &Location) -> Result<Vec<Item>, BackendError> {
        // A folder that is watched is never gone to here: what is known
        // of it is the answer (nothing, while it is still being read),
        // and the rest is told when it is known. So a folder on a
        // machine that does not answer holds nobody.
        if let Location::Directory { .. } = location {
            if let Some(seen) = self.watched(location).first() {
                return Ok(locked(seen).items());
            }
        }
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
        self.read_recent();
        match location {
            Location::Desktop => {
                if let Some(desktop) = &mut self.desktop {
                    log::debug!("zones: the desktop is watched once more (watch {})", watch.0);
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
                    let types = shell::Types::knowing(self.opened.clone());
                    match watch::watch_desktop_folder(&folder, &seen, Arc::clone(&self.news), types) {
                        Ok(watching) => folders.push(watching),
                        Err(error) => log::warn!("zones: {error}"),
                    }
                }
                if !told && folders.is_empty() {
                    log::debug!("zones: the desktop cannot be watched: neither the shell nor its folders tell");
                    return Err(BackendError::Refused {
                        what: "the desktop cannot be watched".to_string(),
                    });
                }
                log::debug!(
                    "zones: the desktop is watched (watch {}): {} the shell, {} of its folders",
                    watch.0,
                    if told { "by" } else { "not by" },
                    folders.len()
                );
                self.shown.add(&seen);
                self.desktop = Some(DesktopWatch { seen, watches: vec![watch], _folders: folders });
            }
            Location::Directory { path } => {
                // Answered at once: the folder is gone to on a thread of
                // its own, and told when it was read (or as away).
                log::debug!("zones: {path} is watched (watch {}), gone to on a thread of its own", watch.0);
                let portal = portal::Portal::open(path, Arc::clone(&self.news), Arc::clone(&self.how));
                self.shown.add(&portal.seen);
                self.folders.insert(watch, portal);
            }
        }
        Ok(watch)
    }

    fn unwatch(&mut self, watch: WatchId) {
        if let Some(portal) = self.folders.remove(&watch) {
            log::debug!("zones: {} is no longer watched (watch {})", portal.path(), watch.0);
            return;
        }
        match self.smart.unwatch(watch) {
            // (The record of opened files is read on: it is a fact of
            // every item. The downloads are let go of.)
            Some(Some(SmartSource::Downloads)) => {
                self.smart.downloads = None;
                return;
            }
            Some(_) => return,
            None => {}
        }
        if let Some(desktop) = &mut self.desktop {
            desktop.watches.retain(|known| *known != watch);
            if desktop.watches.is_empty() {
                log::debug!("zones: the desktop is no longer watched");
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
            log::debug!("zones: Windows says its displays changed");
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
        if Smart::is_program(item) {
            return self.smart.item(item).map(|item| item.facts);
        }
        shell::item_by_id(item, &mut self.types).map(|item| item.facts)
    }

    fn request_icon(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.icons.request(item, pixels(size), false)
    }

    fn request_thumbnail(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.icons.request(item, pixels(size), true)
    }

    fn open(&mut self, item: &ItemId) -> Result<(), BackendError> {
        if item.as_str().starts_with(crate::zones::programs::RUNNING) {
            return self.smart.focus(item);
        }
        if Smart::is_program(item) {
            let app = self.smart.app(item).ok_or_else(|| BackendError::NotFound { what: item.to_string() })?;
            // (On a thread of its own: the shell may take its time to
            // start a program, and says itself what goes wrong.)
            let started = std::thread::Builder::new().name("gevrek-zones-start".to_string()).spawn(move || {
                match crate::apps::start(&app) {
                    Ok(_) => log::debug!("zones: {} was started", app.name),
                    Err(error) => log::warn!("zones: {} did not start: {error}", app.name),
                }
            });
            return started.map(|_| ()).map_err(|error| BackendError::Refused { what: error.to_string() });
        }
        launch::open(item)
    }

    fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
        if Smart::is_program(item) {
            return Err(BackendError::Unsupported);
        }
        launch::open_with(item, program)
    }

    fn context_menu(&mut self, items: &[ItemId], at: Point) -> Result<(), BackendError> {
        // (A program is no item of the shell's: it has no menu there.
        // What can be done with one is the renderer's to offer.)
        if items.iter().any(Smart::is_program) {
            log::debug!("zones: a program has no menu of the shell's");
            return Err(BackendError::Unsupported);
        }
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
        performed(operation, self.files.perform(operation))
    }

    fn begin_operation(&mut self, request: u64, operation: &FileOperation) -> bool {
        // (A copy of the file operations as they are set now: it shows
        // what this one shows, and remembers what this one remembers.)
        let (mut files, operation) = (self.files.clone(), operation.clone());
        self.operations.begin(
            request,
            Box::new(move |watched| {
                files.watched = Some(Arc::clone(watched));
                log::debug!("zones: {operation}");
                performed(&operation, files.perform(&operation))
            }),
        )
    }

    fn cancel_operation(&mut self, request: u64) {
        self.operations.cancel(request);
    }

    fn drop_payload(&mut self) -> Option<DropPayload> {
        self.dragged_over.clone()
    }

    fn start_drag(&mut self, items: &[ItemId], allowed: DragActions) -> Result<(), BackendError> {
        if items.iter().any(Smart::is_program) {
            return Err(BackendError::Unsupported);
        }
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

    fn begin_listing(&mut self, location: &Location) -> bool {
        let Location::Directory { path } = location else {
            // (The desktop is this machine's own and is listed at once.)
            return false;
        };
        let mut watched = false;
        for portal in self.folders.values() {
            if facts::same_path(&portal.path(), path) {
                portal.read_again();
                watched = true;
            }
        }
        if !watched {
            log::debug!("zones: {path} is read once, on a thread of its own");
            // Not watched: read once, on a thread of its own, and told.
            let (news, location, how) = (Arc::clone(&self.news), location.clone(), Arc::clone(&self.how));
            let started = std::thread::Builder::new().name("gevrek-zones-list".to_string()).spawn(move || {
                let _com = crate::com::Com::new();
                let mut types = shell::Types::knowing(how.opened.clone());
                news.tell(vec![match shell::list(&location, &mut types) {
                    Ok(items) => BackendEvent::Listed { location, items },
                    Err(error) => BackendEvent::ListingFailed { location, error },
                }]);
            });
            return started.is_ok();
        }
        true
    }

    fn list_source(&mut self, source: &SmartSource) -> Result<Vec<Item>, BackendError> {
        self.smart.list(source)
    }

    fn watch_source(&mut self, source: &SmartSource) -> Result<WatchId, BackendError> {
        match source {
            SmartSource::Recent => {
                self.read_recent();
                if self.smart.recent.is_none() {
                    return Err(BackendError::NotFound { what: "the folder of recent files".to_string() });
                }
            }
            SmartSource::Downloads => {
                self.read_downloads();
                if self.smart.downloads.is_none() {
                    return Err(BackendError::NotFound { what: "the folder of downloads".to_string() });
                }
            }
            _ => {}
        }
        let watch = WatchId(self.number());
        self.smart.watch(watch, source)?;
        Ok(watch)
    }
}

/// What is known at every watched place (the desktop, the folders of
/// portals), for the one thing that changes an item without touching its
/// file: the person opening it.
#[derive(Clone, Default)]
struct Shown(Arc<Mutex<Vec<Weak<Mutex<seen::Seen>>>>>);

impl Shown {
    fn add(&self, seen: &watch::Shared) {
        let mut places = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        places.retain(|place| place.strong_count() > 0);
        places.push(Arc::downgrade(seen));
    }

    /// Read these files' facts again wherever they are shown; the answer
    /// is what the core is to be told.
    fn reread(&self, paths: &[String], types: &mut shell::Types) -> Vec<BackendEvent> {
        let places: Vec<watch::Shared> = {
            let places = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            places.iter().filter_map(Weak::upgrade).collect()
        };
        let mut events = Vec::new();
        for place in places {
            let mut seen = locked(&place);
            for path in paths {
                let id = ItemId::new(path.clone());
                if !seen.knows(&id) {
                    continue;
                }
                if let Some(item) = shell::item_if_listed(&id, types) {
                    events.extend(seen.apply(seen::Change::Came(item)));
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests;
