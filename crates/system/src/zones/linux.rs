//! The zones on Linux: what `gevrek_zones::backend::ZonesBackend` asks,
//! answered by the freedesktop specifications. There is no shell to ask
//! on Linux as there is on Windows; every desktop reads the same files
//! by the same rules, and so does this.
//!
//! | Module | What it reads or does | By |
//! |---|---|---|
//! | [`places`] | the home, the base directories, the desktop folder | Base Directory, `xdg-user-dirs` |
//! | [`uris`] | files as addresses; what a drag carries | RFC 8089, RFC 2483 |
//! | [`mime`] | what kind of thing a file is, and that kind's icon | Shared MIME-info |
//! | [`entries`] | launchers lying in a folder, and whether they may start | Desktop Entry |
//! | [`associations`] | which program opens what | MIME Applications |
//! | [`thumbs`] | small pictures of files, read and made | Thumbnail Managing Standard |
//! | [`trash`] | throwing away and putting back | Trash |
//! | [`files`] | copy, move, rename, new folder | |
//! | [`watch`] | what changes in a folder, told by the kernel | inotify |
//! | [`portal`] | a folder a portal shows, gone to off the caller's thread | |
//! | [`recent`] | the files opened lately | Desktop Bookmark (`recently-used.xbel`) |
//!
//! [`System`] is what is known of the session (its places, its MIME
//! database, its icon themes) and reads items and pictures; it is shared
//! with the thread that makes pictures. [`SystemBackend`] is the backend
//! itself.
//!
//! # What the trait does not carry, and where it is instead
//!
//! - **Pixels.** `request_icon` answers with a handle; the renderer gets
//!   the picture with [`SystemBackend::picture`].
//! - **The context menu.** Nothing draws a menu for us here. The trait's
//!   `context_menu` works out what can be done with the items and keeps
//!   it; the renderer takes it with [`SystemBackend::take_context_menu`]
//!   and draws it. Likewise "open with" without a program:
//!   [`SystemBackend::take_program_choice`].
//! - **Starting programs.** The shell starts programs through the window
//!   manager (so they get the session's environment). The backend only
//!   works out *what* to start ([`Launch`]) and hands it to whatever the
//!   shell set with [`SystemBackend::set_launcher`].
//! - **Displays.** The compositor knows them; the shell has them from the
//!   window manager's state. [`displays_of_state`] reads that state and
//!   [`SystemBackend::set_displays`] takes the result.
//!
//! # For the surface's side of drag and drop
//!
//! The data device (`wl_data_device`) is the surface's, in shellkit.
//! What it needs from here:
//!
//! - A drag enters with a list of kinds it offers: [`uris::wanted_kind`]
//!   picks the one to ask for. When its data is read:
//!   [`uris::payload`]`(kind, data, allowed)` makes the `DropPayload`,
//!   and [`SystemBackend::set_drop`] keeps it for the core
//!   (`drop_payload`); `set_drop(None)` when the drag leaves or ends.
//!   For a drag that began in a zone of ours no data need be read:
//!   [`uris::payload_of_items`].
//! - The core begins a drag with `start_drag`: the surface takes it with
//!   [`SystemBackend::take_drag`], offers [`DragOut::kinds`] and answers
//!   each request for data with [`DragOut::data`].

pub mod associations;
pub mod entries;
pub mod files;
pub mod md5;
pub mod mime;
pub mod places;
pub mod portal;
pub mod recent;
pub mod thumbs;
pub mod trash;
pub mod uris;
pub mod watch;

pub use associations::{Associations, Program};
pub use places::Places;

use crate::apps::{self, App, Opened};
use crate::icons::Picture;
use crate::zones::reach::Reach;
use crate::zones::smart::Smart;
use crate::zones::sources::Source;
use entries::{DesktopFile, Entry};
use gevrek_model::geometry::{Point, Rect};
use gevrek_zones::backend::{
    BackendError, BackendEvent, DragActions, DropPayload, FileOperation, IconHandle, IconRequest,
    IconSize, Location, WatchId, ZonesBackend,
};
use gevrek_zones::config::SmartSource;
use gevrek_zones::geometry::{Display, Edge, ReservedArea};
use gevrek_zones::ids::ItemId;
use gevrek_zones::item::{Item, ItemFacts, ItemKind};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Make a folder, and those above it that are missing, for the person
/// alone (0700): the trash, the thumbnails and gevrek's own notes are
/// nobody else's business.
pub(crate) fn private_folder(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

/// Write a file for the person alone (0600), whole or not at all: into a
/// file beside it first, which then takes its place in one step.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = WRITTEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .ok_or(std::io::ErrorKind::InvalidInput)?
        .to_string_lossy();
    let beside = path.with_file_name(format!(".{name}.{}.{count}.tmp", std::process::id()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&beside)
        .and_then(|mut file| {
            std::io::Write::write_all(&mut file, bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&beside, path));
    if written.is_err() {
        if let Err(error) = std::fs::remove_file(&beside) {
            log::debug!("zones: {} was left behind: {error}", beside.display());
        }
    }
    written
}

/// Open a file to read it for the zones' own purposes (to tell what kind
/// of file it is, to make a small picture of it) without that counting
/// as the file being used: the kernel is asked not to move the file's
/// time of last access (`O_NOATIME`). A rule that asks whether a file
/// was left untouched for some days asks about the person, not about a
/// zone that drew it.
///
/// The kernel only grants this to the file's owner; for anybody else's
/// file it is opened in the plain way.
pub(crate) fn open_quietly(path: &Path) -> std::io::Result<std::fs::File> {
    // (The number of `O_NOATIME` on the processors gevrek is built for;
    // on any other the plain way is taken.)
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv64"
    ))]
    {
        const NO_ACCESS_TIME: i32 = 0o1000000;
        if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(NO_ACCESS_TIME)
            .open(path)
        {
            return Ok(file);
        }
    }
    std::fs::File::open(path)
}

/// A time as seconds since 1970.
pub(crate) fn seconds(time: Option<std::time::SystemTime>) -> Option<u64> {
    time?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs())
}

/// The side of a picture in pixels: the logical size at the display's
/// scale, within what is drawn.
pub fn pixels(size: IconSize) -> u32 {
    let scale = if size.scale.is_finite() && size.scale > 0.0 {
        size.scale
    } else {
        1.0
    };
    ((f64::from(size.size) * scale).round() as u32).clamp(8, 1024)
}

/// An item as it was read, with what the item itself does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    pub item: Item,
    /// The names its icon may have in a theme, the best first. One that
    /// is a path is that picture.
    pub icon_names: Vec<String>,
    /// What it is as a desktop entry, where its name says it is one.
    pub entry: Option<DesktopFile>,
    /// Its rights, as the system keeps them (`0o644`).
    pub mode: u32,
}

/// What is known of the session, and what is read with it. Nothing in
/// it changes after it is made, so the thread that makes pictures shares
/// it.
pub struct System {
    pub places: Places,
    /// gevrek's own folder for what it keeps (the launcher's counts, the
    /// zones' pins).
    pub state: PathBuf,
    pub mimes: mime::MimeDatabase,
    /// The person's languages for names, the most wanted first.
    pub locales: Vec<String>,
    /// The number the system knows the person by.
    pub uid: Option<u32>,
    /// The icon theme the desktop uses, where it says.
    pub icon_theme: Option<String>,
    /// The folders icon themes are in, and those of loose pictures.
    pub icon_dirs: Vec<PathBuf>,
    pub pixmaps: Vec<PathBuf>,
    /// Which file an icon's name led to, by size: a theme is a great many
    /// folders, and a folder of text files asks for one name over and over.
    found_icons: Mutex<HashMap<(String, u32), Option<PathBuf>>>,
}

impl System {
    /// The session this program runs in.
    pub fn of_session() -> Self {
        let mut system = Self::at(Places::of_session());
        system.state = gevrek_model::paths::state_dir();
        system.locales = apps::locales();
        system.icon_theme = crate::icons::desktop_theme();
        system
    }

    /// A session at these places: its MIME database and its icons are
    /// those of the places' data directories.
    pub fn at(places: Places) -> Self {
        let data = places.all_data_dirs();
        let mut icon_dirs = vec![places.home.join(".icons")];
        icon_dirs.extend(data.iter().map(|dir| dir.join("icons")));
        Self {
            mimes: mime::MimeDatabase::load(&data),
            locales: Vec::new(),
            uid: entries::own_uid(&places),
            icon_theme: None,
            icon_dirs,
            pixmaps: data.iter().map(|dir| dir.join("pixmaps")).collect(),
            found_icons: Mutex::new(HashMap::new()),
            state: places.state_home.join("gevrek"),
            places,
        }
    }

    /// The folder a location is.
    pub fn folder_of(&self, location: &Location) -> PathBuf {
        match location {
            Location::Desktop => self.places.desktop(),
            Location::Directory { path } => PathBuf::from(path),
        }
    }

    /// What is at `path`, read now. `None` where nothing is there, and
    /// for what a desktop does not show: names that begin with a dot,
    /// and the copies editors leave (`name~`).
    pub fn describe(&self, path: &Path) -> Option<Described> {
        let file_name = path.file_name()?.to_string_lossy();
        if file_name.starts_with('.') || file_name.ends_with('~') {
            return None;
        }
        let own = std::fs::symlink_metadata(path).ok()?;
        let is_link = own.file_type().is_symlink();
        // What a link leads to, where it leads anywhere.
        let data = if is_link {
            std::fs::metadata(path).ok()
        } else {
            Some(own.clone())
        };
        let is_folder = data.as_ref().is_some_and(|data| data.is_dir());
        let mime = self.mimes.of_file(path);
        let mut item = Item {
            id: uris::item_id(path),
            kind: if is_folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            },
            name: file_name.to_string(),
            path: path.to_str().map(str::to_string),
            facts: ItemFacts {
                extension: if is_folder {
                    None
                } else {
                    gevrek_zones::item::extension_of(&file_name)
                },
                size: data
                    .as_ref()
                    .filter(|data| data.is_file())
                    .map(|data| data.len()),
                created: seconds(own.created().ok()),
                modified: seconds(data.as_ref().unwrap_or(&own).modified().ok()),
                accessed: seconds(data.as_ref().unwrap_or(&own).accessed().ok()),
                shortcut_target: None,
                mime: Some(mime.clone()),
            },
        };
        let mut icon_names = self.mimes.icon_names(&mime);
        if is_link {
            item.kind = ItemKind::Shortcut;
            item.facts.shortcut_target = std::fs::read_link(path).ok().map(|target| {
                // (As a whole path, wherever the link is read from.)
                path.parent()
                    .map_or(target.clone(), |folder| folder.join(&target))
                    .to_string_lossy()
                    .into_owned()
            });
        }
        let mut entry = None;
        if file_name.ends_with(".desktop") && !is_folder {
            let seen = entries::look(&self.places, self.uid, &self.locales, path);
            if let DesktopFile::Trusted(trusted) = &seen {
                item.name = trusted.name().to_string();
                if !trusted.icon().is_empty() {
                    icon_names.insert(0, trusted.icon().to_string());
                }
                match trusted {
                    Entry::Application { app, .. } => {
                        item.kind = ItemKind::Launcher;
                        item.facts.shortcut_target = app.command.first().cloned();
                    }
                    Entry::Link { url, .. } => {
                        item.kind = ItemKind::Shortcut;
                        item.facts.shortcut_target = Some(url.clone());
                    }
                }
            }
            entry = Some(seen);
        }
        Some(Described {
            item,
            icon_names,
            entry,
            mode: data.unwrap_or(own).mode() & 0o7777,
        })
    }

    /// The item at `path`.
    pub fn item(&self, path: &Path) -> Option<Item> {
        self.describe(path).map(|described| described.item)
    }

    /// The items of a folder, by name.
    pub fn list(&self, folder: &Path) -> std::io::Result<Vec<Item>> {
        let mut items: Vec<Item> = std::fs::read_dir(folder)?
            .flatten()
            .filter_map(|entry| self.item(&entry.path()))
            .collect();
        items.sort_by(|one, other| one.id.cmp(&other.id));
        Ok(items)
    }

    /// The file of the first of these icon names a theme has, for a
    /// picture of `pixels` on a side.
    pub fn icon_file(&self, names: &[String], pixels: u32) -> Option<PathBuf> {
        names.iter().find_map(|name| {
            let wanted = (name.clone(), pixels);
            if let Some(known) = self
                .found_icons
                .lock()
                .ok()
                .and_then(|found| found.get(&wanted).cloned())
            {
                return known;
            }
            let file = crate::icons::find_in(
                &self.icon_dirs,
                &self.pixmaps,
                self.icon_theme.as_deref(),
                name,
                pixels,
            );
            if let Ok(mut found) = self.found_icons.lock() {
                found.insert(wanted, file.clone());
            }
            file
        })
    }

    /// The icon of what is at `path`: a launcher's own, else that of its
    /// type. It reads files: not for the thread that draws.
    pub fn icon(&self, path: &Path, pixels: u32) -> Option<Picture> {
        let described = self.describe(path)?;
        crate::icons::load(&self.icon_file(&described.icon_names, pixels)?, pixels)
    }

    /// What a thumbnail is made of or found by, for the file at `path`.
    fn thumbnail_source(&self, path: &Path) -> Option<(String, String, u64, u64)> {
        let data = std::fs::metadata(path).ok().filter(|data| data.is_file())?;
        // (By the path as it is shown: that is the address a file
        // manager showing the same folder made its thumbnail for.)
        Some((
            uris::uri_of(path),
            self.mimes.of_file(path),
            seconds(data.modified().ok())?,
            data.len(),
        ))
    }

    /// The small picture of what the file at `path` holds, where there
    /// is one or one can be made. Not for the thread that draws.
    pub fn thumbnail(&self, path: &Path, pixels: u32) -> Option<Picture> {
        let (uri, mime, modified, size) = self.thumbnail_source(path)?;
        let source = thumbs::Source {
            path,
            uri: &uri,
            mime: &mime,
            modified,
            size,
        };
        thumbs::picture(&self.places, &source, pixels)
    }

    /// The file in the thumbnail store that is the current small picture
    /// of the file at `path`, where one is there.
    pub fn thumbnail_file(&self, path: &Path, pixels: u32) -> Option<PathBuf> {
        let (uri, _, modified, size) = self.thumbnail_source(path)?;
        thumbs::find(&self.places, &uri, modified, size, pixels)
    }

    /// The program that opens the file at `path` when it is just opened.
    pub fn default_program(&self, path: &Path) -> Option<Program> {
        Associations::load(&self.places, &self.locales)
            .default_for(&self.mimes, &self.mimes.of_file(path))
    }
}

impl watch::Look for System {
    fn item(&self, path: &Path) -> Option<Item> {
        System::item(self, path)
    }

    fn list(&self, folder: &Path) -> Vec<Item> {
        System::list(self, folder).unwrap_or_default()
    }

    fn id(&self, path: &Path) -> ItemId {
        uris::item_id(path)
    }
}

/// One program to start, worked out and ready: no shell reads any of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// What the program is called, for a message.
    pub name: String,
    /// The file to run; empty for an application that is only started
    /// over the session's bus.
    pub program: String,
    pub args: Vec<String>,
    /// The folder to start it in, where its entry names one.
    pub directory: Option<String>,
    /// It wants a terminal to run in.
    pub terminal: bool,
    /// Its name on the session's bus, where it is started by a message
    /// there (`DBusActivatable`); `program` is then what to do where the
    /// bus does not answer.
    pub bus_name: Option<String>,
    /// The addresses it is started with, for the bus's `Open`.
    pub addresses: Vec<String>,
}

/// Start a program directly, as a child of this process: what a backend
/// does until the shell gives it its own way ([`SystemBackend::set_launcher`]).
/// A program that wants a terminal is refused: which terminal is the
/// shell's to know.
pub fn start_directly(launch: &Launch) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    if launch.program.is_empty() || launch.terminal {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "needs the session's bus or a terminal",
        ));
    }
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = &launch.directory {
        command.current_dir(directory);
    }
    let mut child = command.spawn()?;
    // Waited for on a thread of its own, so that it does not stay behind
    // as a dead entry of the process table when it ends.
    let waited = std::thread::Builder::new()
        .name("gevrek-zones-child".to_string())
        .spawn(move || {
            if let Err(error) = child.wait() {
                log::debug!("zones: the end of a program that was started is not known: {error}");
            }
        });
    if let Err(error) = waited {
        log::debug!(
            "zones: {} is not waited for, and stays behind when it ends: {error}",
            launch.program
        );
    }
    log::debug!("zones: {} was started directly", launch.program);
    Ok(())
}

/// What can be done with the items a menu was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Open each as a double click would (`ZonesBackend::open`).
    Open,
    /// Open with one of these (`open_with` with the program's `id`); the
    /// default is the first. Offered when the items are of one type.
    OpenWith { programs: Vec<Program> },
    /// Some of the items are launchers that were not allowed to start
    /// ([`SystemBackend::allow_launching`]).
    AllowLaunching,
    /// One item: give it another name (`FileOperation::Rename`).
    Rename,
    /// `FileOperation::Trash` for each.
    MoveToTrash,
    /// `FileOperation::NewFolder` in this folder.
    NewFolder { parent: String },
    /// Show [`ContextMenu::properties`].
    Properties,
}

/// What a properties window shows of one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Properties {
    pub item: Item,
    /// Its rights (`0o644`).
    pub mode: u32,
    /// For a desktop entry: whether it may be started.
    pub trusted: Option<bool>,
    /// The program it is opened with.
    pub opens_with: Option<String>,
}

/// The menu for some items at a point: what the renderer draws, since
/// there is no shell to draw it.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    pub at: Point,
    pub items: Vec<ItemId>,
    /// In the order a menu has them.
    pub actions: Vec<MenuAction>,
    pub properties: Vec<Properties>,
}

/// The programs the person may choose from to open an item ("open with"
/// without a program named).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramChoice {
    pub item: ItemId,
    pub programs: Vec<Program>,
}

/// A drag of items out of a zone, for the surface to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragOut {
    pub items: Vec<ItemId>,
    pub allowed: DragActions,
}

impl DragOut {
    /// The kinds of data to offer, the best first.
    pub fn kinds(&self) -> &'static [&'static str] {
        &uris::OFFERED
    }

    /// The data for one of the kinds.
    pub fn data(&self, kind: &str) -> Option<Vec<u8>> {
        uris::data_for(kind, &self.items)
    }
}

/// What the window manager's state (the JSON the shell has from it) says
/// of the displays: each with the id that stays the same wherever it is
/// plugged in (`stable_id`, made of its EDID; `port:<connector>` where
/// the state has none), its rectangle and scale, and the room kept at
/// its sides (what its work area leaves of it).
pub fn displays_of_state(state: &serde_json::Value) -> (Vec<Display>, Vec<ReservedArea>) {
    let mut displays = Vec::new();
    let mut reserved = Vec::new();
    let number = |value: &serde_json::Value| value.as_f64().unwrap_or(0.0).round() as i32;
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        let said = |key: &str| monitor[key].as_str().filter(|text| !text.is_empty());
        let Some(id) = said("stable_id").map(str::to_string).or_else(|| {
            said("connector")
                .or_else(|| said("name"))
                .map(|port| format!("port:{port}"))
        }) else {
            continue;
        };
        let rect = Rect::new(
            number(&monitor["position"]["x"]),
            number(&monitor["position"]["y"]),
            number(&monitor["size"]["width"]),
            number(&monitor["size"]["height"]),
        );
        if rect.width <= 0 || rect.height <= 0 {
            continue;
        }
        let scale = monitor["scale"]
            .as_f64()
            .filter(|scale| *scale > 0.0)
            .unwrap_or(1.0);
        let mut display = Display::new(&id, rect);
        display.scale = scale;
        display.primary = monitor["is_primary"].as_bool().unwrap_or(false);
        // The work area where the state has one, else the sides it names.
        let work = Rect::new(
            number(&monitor["work_position"]["x"]),
            number(&monitor["work_position"]["y"]),
            number(&monitor["work_size"]["width"]),
            number(&monitor["work_size"]["height"]),
        );
        let sides = if work.width > 0 && work.height > 0 {
            [
                (Edge::Left, work.x - rect.x),
                (Edge::Top, work.y - rect.y),
                (Edge::Right, rect.x + rect.width - work.x - work.width),
                (Edge::Bottom, rect.y + rect.height - work.y - work.height),
            ]
        } else {
            [
                (Edge::Left, number(&monitor["reserved_left"])),
                (Edge::Top, number(&monitor["reserved_top"])),
                (Edge::Right, number(&monitor["reserved_right"])),
                (Edge::Bottom, number(&monitor["reserved_bottom"])),
            ]
        };
        for (edge, thickness) in sides {
            if thickness > 0 {
                reserved.push(ReservedArea {
                    display: display.id.clone(),
                    edge,
                    thickness,
                });
            }
        }
        displays.push(display);
    }
    (displays, reserved)
}

/// The pictures of items by handle: what is common to both systems
/// (`super::pictures`), with the icon themes and the thumbnail store
/// asked for each.
type Pictures = super::pictures::Pictures;

fn pictures_of(
    system: Arc<System>,
    programs: Arc<Mutex<crate::zones::programs::Known>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Pictures {
    Pictures::with(
        move |id, pixels, thumbnail| {
            if Smart::is_program(&ItemId::new(id)) {
                // (A program holds no picture of what is in it. Its icon
                // is the one it names: a file, or a name in the themes.)
                if thumbnail {
                    return None;
                }
                let icon = programs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .icon_of(id)?;
                let file = if icon.starts_with('/') {
                    PathBuf::from(icon)
                } else {
                    system.icon_file(&[icon], pixels)?
                };
                return crate::icons::load(&file, pixels);
            }
            let path = uris::path_of(id)?;
            if thumbnail {
                system.thumbnail(&path, pixels)
            } else {
                system.icon(&path, pixels)
            }
        },
        wake,
    )
}

/// How the shell starts a program.
type Launcher = Box<dyn FnMut(&Launch) -> std::io::Result<()>>;

/// The backend of the zones on Linux.
pub struct SystemBackend {
    system: Arc<System>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// Made when the first folder is watched.
    watcher: Option<watch::Watcher>,
    folders: watch::Folders,
    /// The folders portals show, each gone to on threads of its own.
    portals: BTreeMap<WatchId, portal::Portal>,
    /// How a folder is gone to.
    how: Arc<portal::How>,
    /// What the threads that go to folders found, and how they say it.
    answers: std::sync::mpsc::Receiver<portal::Answer>,
    answer: std::sync::mpsc::Sender<portal::Answer>,
    /// What smart zones look through that is no plain folder.
    smart: Smart,
    /// What the lists of smart zones told since the last drain, and the
    /// file operations that end later.
    told: Arc<Mutex<Vec<BackendEvent>>>,
    /// The file operations that end later, on a thread of their own.
    operations: crate::zones::operations::Operations,
    /// What is to be told besides what the kernel and the pictures tell.
    events: Vec<BackendEvent>,
    numbers: u64,
    pictures: Pictures,
    displays: Vec<Display>,
    reserved: Vec<ReservedArea>,
    launcher: Launcher,
    trash: Option<trash::Trash>,
    menu: Option<ContextMenu>,
    choice: Option<ProgramChoice>,
    dragged_over: Option<DropPayload>,
    dragged_out: Option<DragOut>,
}

/// Carry out one file operation; the answer is what takes it back, with
/// the names the files really got. `tick` is told how far a copy is (in
/// bytes) and may call it off; nothing of a copy that was called off is
/// left, and a file that was being moved is where it was.
fn carry_out(
    trash: Option<&trash::Trash>,
    operation: &FileOperation,
    tick: files::Tick<'_>,
) -> Result<FileOperation, BackendError> {
    log::debug!("zones: {operation}");
    let done = carry_out_now(trash, operation, tick);
    match &done {
        Ok(undo) => log::debug!("zones: {operation}: done (taken back by {undo})"),
        Err(BackendError::Cancelled) => log::debug!("zones: {operation}: called off"),
        Err(error) => log::debug!("zones: {operation}: failed: {error}"),
    }
    done
}

fn carry_out_now(
    trash: Option<&trash::Trash>,
    operation: &FileOperation,
    tick: files::Tick<'_>,
) -> Result<FileOperation, BackendError> {
    // (Without knowing who the person is, no trash can is theirs.)
    let can = || trash.ok_or(BackendError::Unsupported);
    match operation {
        FileOperation::Copy { from, to_folder } => {
            files::copy_told(Path::new(from), Path::new(to_folder), tick)
                .map(|copy| FileOperation::Trash { path: text(&copy) })
                .map_err(|error| refusal(error, from))
        }
        FileOperation::Move { from, to_folder } => {
            let back = Path::new(from)
                .parent()
                .map(text)
                .ok_or_else(|| BackendError::NotFound { what: from.clone() })?;
            files::move_into_told(Path::new(from), Path::new(to_folder), tick)
                .map(|moved| FileOperation::Move {
                    from: text(&moved),
                    to_folder: back,
                })
                .map_err(|error| refusal(error, from))
        }
        FileOperation::Rename { path, new_name } => {
            let old = Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            files::rename(Path::new(path), new_name)
                .map(|renamed| FileOperation::Rename {
                    path: text(&renamed),
                    new_name: old,
                })
                .map_err(|error| match error.kind() {
                    std::io::ErrorKind::AlreadyExists => BackendError::Exists {
                        what: new_name.clone(),
                    },
                    _ => refusal(error, path),
                })
        }
        FileOperation::NewFolder { parent, name } => files::new_folder(Path::new(parent), name)
            .map(|made| FileOperation::Trash { path: text(&made) })
            .map_err(|error| refusal(error, parent)),
        FileOperation::Trash { path } => can()?
            .put(Path::new(path))
            .map(|_| FileOperation::Restore { path: path.clone() })
            .map_err(|error| refusal(error, path)),
        FileOperation::Restore { path } => can()?
            .restore(Path::new(path))
            .map(|_| FileOperation::Trash { path: path.clone() })
            .map_err(|error| refusal(error, path)),
    }
}

/// What an error of the file system is to the core.
fn refusal(error: std::io::Error, what: &str) -> BackendError {
    match error.kind() {
        std::io::ErrorKind::NotFound => BackendError::NotFound {
            what: what.to_string(),
        },
        std::io::ErrorKind::AlreadyExists => BackendError::Exists {
            what: what.to_string(),
        },
        std::io::ErrorKind::Unsupported => BackendError::Unsupported,
        std::io::ErrorKind::Interrupted => BackendError::Cancelled,
        _ => BackendError::Refused {
            what: format!("{what}: {error}"),
        },
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl SystemBackend {
    /// The backend of the session this program runs in. `wake` is called
    /// (from any thread) when there is something to tell: the moment to
    /// call `drain_events`, which is `Host::pump`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with(System::of_session(), wake)
    }

    /// A backend of a session that is described by hand (a test, a
    /// trial under another folder).
    pub fn with(system: System, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let system = Arc::new(system);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let trash = system
            .uid
            .map(|uid| trash::Trash::new(system.places.data_home.join("Trash"), uid));
        let told: Arc<Mutex<Vec<BackendEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let (telling, woken) = (Arc::clone(&told), Arc::clone(&wake));
        let reading = Arc::clone(&system);
        let data = system.places.all_data_dirs();
        let smart = Smart::new(
            &system.state,
            Arc::new(move || apps::installed_in(&reading.places.all_data_dirs(), &reading.locales)),
            data.iter().map(|dir| dir.join("applications")).collect(),
            Arc::new(move |events| {
                telling
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend(events);
                woken();
            }),
        );
        let (answer, answers) = std::sync::mpsc::channel();
        let (telling, woken) = (Arc::clone(&told), Arc::clone(&wake));
        let operations = crate::zones::operations::Operations::new(Arc::new(move |events| {
            telling
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(events);
            woken();
        }));
        Self {
            operations,
            pictures: pictures_of(
                Arc::clone(&system),
                Arc::clone(&smart.known),
                Arc::clone(&wake),
            ),
            system,
            wake,
            watcher: None,
            folders: watch::Folders::default(),
            portals: BTreeMap::new(),
            how: Arc::new(portal::How::default()),
            answers,
            answer,
            smart,
            told,
            events: Vec::new(),
            numbers: 0,
            displays: Vec::new(),
            reserved: Vec::new(),
            launcher: Box::new(start_directly),
            trash,
            menu: None,
            choice: None,
            dragged_over: None,
            dragged_out: None,
        }
    }

    /// What is known of the session.
    pub fn system(&self) -> &System {
        &self.system
    }

    /// The pixels of a picture that was answered with this handle: four
    /// bytes each (red, green, blue, how solid), not multiplied.
    pub fn picture(&mut self, icon: IconHandle) -> Option<&Picture> {
        self.pictures.picture(icon)
    }

    /// How programs are started from now on: the shell's own way, through
    /// the window manager.
    pub fn set_launcher(&mut self, launcher: impl FnMut(&Launch) -> std::io::Result<()> + 'static) {
        self.launcher = Box::new(launcher);
    }

    /// The displays and the room kept on them are these now (see
    /// [`displays_of_state`]). Told to the core where they changed.
    pub fn set_displays(&mut self, displays: Vec<Display>, reserved: Vec<ReservedArea>) {
        if self.displays == displays && self.reserved == reserved {
            return;
        }
        log::debug!(
            "zones: the displays or the room kept on them changed: {} displays, {} areas kept",
            displays.len(),
            reserved.len()
        );
        self.displays = displays.clone();
        self.reserved = reserved.clone();
        self.events
            .push(BackendEvent::Displays { displays, reserved });
    }

    /// The watched folders that cannot be reached now, each with why:
    /// for a word to the person in the zone that shows it.
    pub fn unreachable(&self) -> Vec<(Location, String)> {
        self.portals
            .values()
            .filter_map(|portal| match &portal.reach {
                Reach::Away { why } => Some((portal.location.clone(), why.clone())),
                _ => None,
            })
            .collect()
    }

    /// Go once more to every watched folder that is away. For when
    /// something happened that no folder can tell of (a mount that came
    /// back), and for the person's own "try again". Returns at once; a
    /// folder that is back is told whole.
    pub fn retry_unreachable(&mut self) {
        let away: Vec<WatchId> = self
            .portals
            .iter()
            .filter(|(_, portal)| portal.reach.is_away())
            .map(|(watch, _)| *watch)
            .collect();
        for watch in away {
            self.go_to(watch);
        }
    }

    /// Go to folders in another way than the system's own (a test that
    /// plays a mount which does not answer). For folders watched from
    /// now on.
    pub fn set_reach(&mut self, how: portal::How) {
        self.how = Arc::new(how);
    }

    /// Keep at most this many bytes of pictures in memory
    /// (`crate::zones::pictures`).
    pub fn set_picture_budget(&mut self, bytes: usize) {
        self.pictures.set_budget(bytes);
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
    /// the person chooses from to pin one. It reads every desktop entry:
    /// not for the thread that draws.
    pub fn installed_programs(&self) -> Vec<Item> {
        self.smart.installed_items()
    }

    /// Read the installed programs another way (a test's own list).
    pub fn set_programs(&mut self, installed: impl Fn() -> Vec<App> + Send + Sync + 'static) {
        self.smart.set_installed(Arc::new(installed));
    }

    /// Go to a watched folder, on threads of its own. One going at a
    /// time: asked for while one is under way, another follows it.
    fn go_to(&mut self, watch: WatchId) {
        let Some(portal) = self.portals.get_mut(&watch) else {
            return;
        };
        if portal.opening {
            portal.again = true;
            return;
        }
        portal.opening = true;
        // What the kernel told of it so far is not told on any more: the
        // folder is read anew, and told whole.
        portal.watcher = None;
        self.folders.remove(watch);
        let gone = portal::go(
            watch,
            portal.folder.clone(),
            Arc::clone(&self.system),
            Arc::clone(&self.how),
            self.answer.clone(),
            Arc::clone(&self.wake),
        );
        log::debug!(
            "zones: going to {} (watch {}) on a thread of its own",
            portal.folder.display(),
            watch.0
        );
        if let Err(error) = gone {
            log::debug!(
                "zones: {} cannot be gone to: no thread: {error}",
                portal.folder.display()
            );
            portal.opening = false;
            let reach = Reach::Away {
                why: format!("{}: no thread to read it on: {error}", text(&portal.folder)),
            };
            self.events
                .extend(crate::zones::reach::told(&portal.location, &reach));
            portal.reach = reach;
        }
    }

    /// Take what a thread that went to a folder found.
    fn settle(&mut self, answer: portal::Answer, events: &mut Vec<BackendEvent>) {
        let (watch, outcome) = match (answer.watch, answer.outcome) {
            (_, portal::Outcome::Once { location, listed }) => {
                events.push(match listed {
                    Ok(items) => BackendEvent::Listed { location, items },
                    Err(error) => BackendEvent::ListingFailed { location, error },
                });
                return;
            }
            (None, _) => return,
            (Some(watch), outcome) => (watch, outcome),
        };
        // (Let go of while its thread was under way.)
        let Some(portal) = self.portals.get_mut(&watch) else {
            return;
        };
        let (reach, done) = match outcome {
            portal::Outcome::Slow => (
                Reach::Away {
                    why: format!("{} does not answer", text(&portal.folder)),
                },
                false,
            ),
            portal::Outcome::There { watcher, items } => {
                portal.watcher = Some(watcher);
                portal.sentinel = None;
                self.folders.adopt(
                    watch,
                    portal.location.clone(),
                    portal.folder.clone(),
                    &items,
                );
                events.push(BackendEvent::Listed {
                    location: portal.location.clone(),
                    items,
                });
                (Reach::There, true)
            }
            portal::Outcome::Away { why, sentinel } => {
                portal.watcher = None;
                portal.sentinel = sentinel;
                (Reach::Away { why }, true)
            }
            portal::Outcome::Once { .. } => return,
        };
        if portal.reach != reach {
            match &reach {
                Reach::There => log::debug!(
                    "zones: {} is there (watch {})",
                    portal.folder.display(),
                    watch.0
                ),
                Reach::Away { why } => log::debug!(
                    "zones: {} is away (watch {}): {why}",
                    portal.folder.display(),
                    watch.0
                ),
                Reach::Opening => {}
            }
            events.extend(crate::zones::reach::told(&portal.location, &reach));
        }
        portal.reach = reach;
        if !done {
            return;
        }
        portal.opening = false;
        if std::mem::take(&mut portal.again) {
            self.go_to(watch);
        }
    }

    /// The list of the files opened lately, read and watched from now on.
    fn read_recent(&mut self) {
        if self.smart.recent.is_some() {
            return;
        }
        let system = Arc::clone(&self.system);
        self.smart.recent = Some(Source::new(
            crate::zones::smart::told_as(SmartSource::Recent, &self.smart.recent_wanted),
            move || recent::read(&system, recent::MOST),
            std::slice::from_ref(&self.system.places.data_home),
            |path| {
                path.file_name()
                    .is_some_and(|name| name == "recently-used.xbel")
            },
            self.smart_tell(),
        ));
    }

    /// The downloads folder, read and watched from now on.
    fn read_downloads(&mut self, folder: PathBuf) {
        if self.smart.downloads.is_some() {
            return;
        }
        let (system, from) = (Arc::clone(&self.system), folder.clone());
        self.smart.downloads = Some(Source::new(
            crate::zones::smart::told_as(SmartSource::Downloads, &self.smart.downloads_wanted),
            move || crate::zones::smart::newest_first(system.list(&from).unwrap_or_default()),
            std::slice::from_ref(&folder),
            |_| true,
            self.smart_tell(),
        ));
    }

    fn smart_tell(&self) -> crate::zones::sources::Tell {
        let (told, wake) = (Arc::clone(&self.told), Arc::clone(&self.wake));
        Arc::new(move |events| {
            told.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(events);
            wake();
        })
    }

    /// The menu that was last asked for with `context_menu`, once.
    pub fn take_context_menu(&mut self) -> Option<ContextMenu> {
        self.menu.take()
    }

    /// The programs to choose from that `open_with` without a program
    /// asked for, once.
    pub fn take_program_choice(&mut self) -> Option<ProgramChoice> {
        self.choice.take()
    }

    /// What the drag over a zone carries now; `None` when it left.
    pub fn set_drop(&mut self, payload: Option<DropPayload>) {
        self.dragged_over = payload;
    }

    /// The drag the core began with `start_drag`, once.
    pub fn take_drag(&mut self) -> Option<DragOut> {
        self.dragged_out.take()
    }

    /// The person allows a launcher to be started (see [`entries`] for
    /// what that takes and does).
    pub fn allow_launching(&mut self, item: &ItemId) -> Result<(), BackendError> {
        let path = self.path_of(item)?;
        entries::allow(&self.system.places, self.system.uid, &path)
            .map_err(|error| refusal(error, &text(&path)))?;
        self.changed(item);
        Ok(())
    }

    /// The file an item is, where it is there.
    fn path_of(&self, item: &ItemId) -> Result<PathBuf, BackendError> {
        uris::path_of_item(item)
            .filter(|path| path.symlink_metadata().is_ok())
            .ok_or_else(|| BackendError::NotFound {
                what: item.to_string(),
            })
    }

    /// An item is other than it was by something done here that the
    /// kernel may not tell of in a way that shows it (its rights): its
    /// pictures are made anew.
    fn changed(&mut self, item: &ItemId) {
        self.pictures.forget(item);
    }

    /// Hand the programs an entry means to the launcher.
    fn start(&mut self, app: &App, exec: &str, opened: &[Opened]) -> Result<(), BackendError> {
        let mut commands = apps::commands_for(exec, &app.name, &app.icon, &app.entry, opened);
        if commands.is_empty() {
            if !app.dbus_activatable {
                return Err(BackendError::Refused {
                    what: format!("{} names no program", app.name),
                });
            }
            // Only over the bus: one start, with everything.
            commands.push(Vec::new());
        }
        for command in commands {
            let launch = Launch {
                name: app.name.clone(),
                program: command.first().cloned().unwrap_or_default(),
                args: command.iter().skip(1).cloned().collect(),
                directory: app.directory.clone(),
                terminal: app.terminal,
                bus_name: app.dbus_activatable.then(|| app.id.clone()),
                addresses: opened.iter().map(|opened| opened.address.clone()).collect(),
            };
            (self.launcher)(&launch).map_err(|error| refusal(error, &app.name))?;
            log::debug!(
                "zones: {} is started ({})",
                app.name,
                if launch.program.is_empty() {
                    "over the bus"
                } else {
                    &launch.program
                }
            );
        }
        Ok(())
    }

    /// Open something with the program for its type.
    fn open_as(&mut self, mime: &str, opened: Opened) -> Result<(), BackendError> {
        let program = Associations::load(&self.system.places, &self.system.locales)
            .default_for(&self.system.mimes, mime)
            .ok_or_else(|| BackendError::NotFound {
                what: format!("a program that opens {mime}"),
            })?;
        log::debug!(
            "zones: {} is opened with {}, the program for {mime}",
            opened.file,
            program.app.name
        );
        self.start(&program.app, &program.exec, &[opened])
    }

    /// What can be done with these items, for a menu at `at`.
    pub fn menu_for(&self, items: &[ItemId], at: Point) -> ContextMenu {
        let system = &self.system;
        let described: Vec<Described> = items
            .iter()
            .filter_map(|item| system.describe(&uris::path_of_item(item)?))
            .collect();
        let associations = Associations::load(&system.places, &system.locales);
        let mime_of = |described: &Described| described.item.facts.mime.clone().unwrap_or_default();
        let untrusted =
            |described: &Described| matches!(described.entry, Some(DesktopFile::Untrusted));
        let trusted =
            |described: &Described| matches!(described.entry, Some(DesktopFile::Trusted(_)));
        let mut actions = Vec::new();
        if let Some(first) = described.first() {
            actions.push(MenuAction::Open);
            // One type throughout, and nothing that starts itself.
            let one_type = described
                .iter()
                .all(|other| mime_of(other) == mime_of(first));
            if one_type && !described.iter().any(trusted) {
                let programs = associations.candidates(&system.mimes, &mime_of(first));
                if !programs.is_empty() {
                    actions.push(MenuAction::OpenWith { programs });
                }
            }
            if described.iter().any(untrusted) {
                actions.push(MenuAction::AllowLaunching);
            }
            if described.len() == 1 {
                actions.push(MenuAction::Rename);
            }
            actions.push(MenuAction::MoveToTrash);
        }
        // A new folder goes where the items are; on the bare ground, onto
        // the desktop.
        let parent = described
            .first()
            .and_then(|first| Path::new(first.item.path.as_deref()?).parent().map(text))
            .unwrap_or_else(|| text(&system.places.desktop()));
        actions.push(MenuAction::NewFolder { parent });
        if !described.is_empty() {
            actions.push(MenuAction::Properties);
        }
        let properties = described
            .iter()
            .map(|described| Properties {
                item: described.item.clone(),
                mode: described.mode,
                trusted: described.entry.as_ref().and_then(|entry| match entry {
                    DesktopFile::Trusted(_) => Some(true),
                    DesktopFile::Untrusted => Some(false),
                    DesktopFile::NotAnEntry => None,
                }),
                opens_with: if trusted(described) {
                    None
                } else {
                    associations
                        .default_for(&system.mimes, &mime_of(described))
                        .map(|program| program.name)
                },
            })
            .collect();
        ContextMenu {
            at,
            items: described
                .iter()
                .map(|described| described.item.id.clone())
                .collect(),
            actions,
            properties,
        }
    }
}

impl ZonesBackend for SystemBackend {
    fn list(&mut self, location: &Location) -> Result<Vec<Item>, BackendError> {
        let folder = self.system.folder_of(location);
        // A watched folder that is being gone to, or is away, is not
        // gone to here as well: nothing is known of it, and what is in
        // it is told when it is. So a mount that does not answer holds
        // nobody.
        if let Location::Directory { .. } = location {
            let mut watched = self
                .portals
                .values()
                .filter(|portal| portal.folder == folder)
                .peekable();
            if watched.peek().is_some() && !watched.any(|portal| portal.reach == Reach::There) {
                return Ok(Vec::new());
            }
        }
        self.system
            .list(&folder)
            .map_err(|error| refusal(error, &text(&folder)))
    }

    fn watch(&mut self, location: &Location) -> Result<WatchId, BackendError> {
        let folder = self.system.folder_of(location);
        if let Location::Directory { .. } = location {
            // Answered at once: the folder is gone to on a thread of its
            // own, and told when it was read (or as away).
            self.numbers += 1;
            let watch = WatchId(self.numbers);
            log::debug!("zones: {} is watched (watch {})", folder.display(), watch.0);
            self.portals
                .insert(watch, portal::Portal::new(location.clone(), folder));
            self.go_to(watch);
            return Ok(watch);
        }
        if !folder.is_dir() {
            log::debug!(
                "zones: {} is not watched: it is no folder",
                folder.display()
            );
            return Err(BackendError::NotFound {
                what: text(&folder),
            });
        }
        self.numbers += 1;
        let watch = WatchId(self.numbers);
        if !self
            .folders
            .add(watch, location.clone(), folder.clone(), &*self.system)
        {
            return Ok(watch);
        }
        let wake = Arc::clone(&self.wake);
        let begun = match self.watcher.take() {
            Some(watcher) => Ok(watcher),
            None => watch::Watcher::new(move || wake()),
        }
        .and_then(|mut watcher| {
            let added = watcher.add(&folder);
            // (Kept also where this folder could not be added: it still
            // holds the others.)
            self.watcher = Some(watcher);
            added
        });
        begun.map(|()| watch).map_err(|error| {
            log::debug!("zones: {} cannot be watched: {error}", folder.display());
            self.folders.remove(watch);
            BackendError::Refused {
                what: format!("{}: {error}", text(&folder)),
            }
        })
    }

    fn unwatch(&mut self, watch: WatchId) {
        // (A portal's own watcher and its threads' answers go with it.)
        if let Some(portal) = self.portals.remove(&watch) {
            log::debug!(
                "zones: {} is no longer watched (watch {})",
                portal.folder.display(),
                watch.0
            );
            self.folders.remove(watch);
            return;
        }
        match self.smart.unwatch(watch) {
            Some(Some(SmartSource::Recent)) => {
                self.smart.recent = None;
                return;
            }
            Some(Some(SmartSource::Downloads)) => {
                self.smart.downloads = None;
                return;
            }
            Some(_) => return,
            None => {}
        }
        if let (Some(folder), Some(watcher)) = (self.folders.remove(watch), &mut self.watcher) {
            log::debug!(
                "zones: {} is no longer watched (watch {})",
                folder.display(),
                watch.0
            );
            watcher.remove(&folder);
        }
        if self.folders.is_empty() {
            // The kernel's watches and their thread go with the last.
            self.watcher = None;
        }
    }

    fn drain_events(&mut self) -> Vec<BackendEvent> {
        let mut events = std::mem::take(&mut self.events);
        events.append(
            &mut self
                .told
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        // What the threads that went to folders found.
        while let Ok(answer) = self.answers.try_recv() {
            self.settle(answer, &mut events);
        }
        let mut changes = self
            .watcher
            .as_mut()
            .map(watch::Watcher::take)
            .unwrap_or_default();
        for portal in self.portals.values_mut() {
            if let Some(watcher) = &mut portal.watcher {
                changes.extend(watcher.take());
            }
        }
        // A folder that is away may be back: the folder above it told.
        let mut again: Vec<WatchId> = self
            .portals
            .iter_mut()
            .filter_map(|(watch, portal)| portal.may_be_back().then_some(*watch))
            .collect();
        if !changes.is_empty() {
            let told = self.folders.translate(changes, &*self.system);
            // What changed or went has other pictures now, or none.
            for event in &told {
                match event {
                    BackendEvent::ItemChanged { item, .. } => self.pictures.forget(&item.id),
                    BackendEvent::ItemRemoved { item, .. } => self.pictures.forget(item),
                    BackendEvent::ItemRenamed { from, .. } => self.pictures.forget(from),
                    _ => {}
                }
            }
            events.extend(told);
            // A watched folder that was read anew may itself be gone
            // (deleted, moved, its drive pulled out): it is gone to
            // again, which says so or tells it whole.
            for shaken in self.folders.take_shaken() {
                again.extend(
                    self.portals
                        .iter()
                        .filter(|(_, portal)| portal.folder == shaken)
                        .map(|(watch, _)| *watch),
                );
            }
        }
        for watch in again {
            self.go_to(watch);
        }
        events.extend(self.pictures.drain());
        events
    }

    fn facts(&mut self, item: &ItemId) -> Option<ItemFacts> {
        if Smart::is_program(item) {
            return self.smart.item(item).map(|item| item.facts);
        }
        self.system
            .item(&uris::path_of_item(item)?)
            .map(|item| item.facts)
    }

    fn request_icon(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.pictures.request(item, pixels(size), false)
    }

    fn request_thumbnail(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
        self.pictures.request(item, pixels(size), true)
    }

    fn open(&mut self, item: &ItemId) -> Result<(), BackendError> {
        if item.as_str().starts_with(crate::zones::programs::RUNNING) {
            return self.smart.focus(item);
        }
        if Smart::is_program(item) {
            let app = self.smart.app(item).ok_or_else(|| BackendError::NotFound {
                what: item.to_string(),
            })?;
            // (The entry was read with nothing to open: its command is
            // ready as it is.)
            let launch = Launch {
                name: app.name.clone(),
                program: app.command.first().cloned().unwrap_or_default(),
                args: app.command.iter().skip(1).cloned().collect(),
                directory: app.directory.clone(),
                terminal: app.terminal,
                bus_name: app.dbus_activatable.then(|| app.id.clone()),
                addresses: Vec::new(),
            };
            return (self.launcher)(&launch).map_err(|error| refusal(error, &app.name));
        }
        let path = self.path_of(item)?;
        let described = self
            .system
            .describe(&path)
            .ok_or_else(|| BackendError::NotFound {
                what: item.to_string(),
            })?;
        match described.entry {
            Some(DesktopFile::Trusted(Entry::Application { app, exec })) => {
                self.start(&app, &exec, &[])
            }
            Some(DesktopFile::Trusted(Entry::Link { url, .. })) => match uris::path_of(&url) {
                // A link to a file is that file, opened as any.
                Some(file) => self.open(&uris::item_id(&file)),
                None => {
                    let scheme = url.split(':').next().unwrap_or_default().to_lowercase();
                    self.open_as(
                        &format!("x-scheme-handler/{scheme}"),
                        Opened {
                            file: url.clone(),
                            address: url,
                        },
                    )
                }
            },
            // Never started, and not opened as text either: the person
            // is to see that it is a launcher they have not allowed.
            Some(DesktopFile::Untrusted) => Err(BackendError::Refused {
                what: format!(
                    "{} is a launcher that was not allowed to start",
                    described.item.name
                ),
            }),
            Some(DesktopFile::NotAnEntry) | None => {
                let mime = described
                    .item
                    .facts
                    .mime
                    .unwrap_or_else(|| mime::UNKNOWN.to_string());
                self.open_as(
                    &mime,
                    Opened {
                        file: text(&path),
                        address: uris::uri_of(&path),
                    },
                )
            }
        }
    }

    fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
        if Smart::is_program(item) {
            return Err(BackendError::Unsupported);
        }
        let path = self.path_of(item)?;
        let associations = Associations::load(&self.system.places, &self.system.locales);
        match program {
            Some(id) => {
                let program = associations
                    .program(id)
                    .ok_or_else(|| BackendError::NotFound {
                        what: id.to_string(),
                    })?;
                let opened = Opened {
                    file: text(&path),
                    address: uris::uri_of(&path),
                };
                self.start(&program.app, &program.exec, &[opened])
            }
            None => {
                let programs =
                    associations.candidates(&self.system.mimes, &self.system.mimes.of_file(&path));
                self.choice = Some(ProgramChoice {
                    item: item.clone(),
                    programs,
                });
                (self.wake)();
                Ok(())
            }
        }
    }

    fn context_menu(&mut self, items: &[ItemId], at: Point) -> Result<(), BackendError> {
        // (A program is no file: what can be done with one is the
        // renderer's to offer.)
        if items.iter().any(Smart::is_program) {
            log::debug!("zones: a program has no menu of the system's");
            return Err(BackendError::Unsupported);
        }
        self.menu = Some(self.menu_for(items, at));
        (self.wake)();
        Ok(())
    }

    fn perform(&mut self, operation: &FileOperation) -> Result<FileOperation, BackendError> {
        carry_out(self.trash.as_ref(), operation, &mut |_, _| Ok(()))
    }

    fn begin_operation(&mut self, request: u64, operation: &FileOperation) -> bool {
        let (trash, operation) = (self.trash.clone(), operation.clone());
        self.operations.begin(
            request,
            Box::new(move |watched| {
                carry_out(trash.as_ref(), &operation, &mut |done, total| {
                    watched.progress(done, total);
                    if watched.called_off() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            "called off",
                        ));
                    }
                    Ok(())
                })
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
        let there: Vec<ItemId> = items
            .iter()
            .filter(|item| self.path_of(item).is_ok())
            .cloned()
            .collect();
        if there.len() < items.len() {
            log::debug!(
                "zones: {} of the {} items dragged are no files that are there",
                items.len() - there.len(),
                items.len()
            );
        }
        if there.is_empty() {
            return Err(BackendError::NotFound {
                what: "anything to drag".to_string(),
            });
        }
        self.dragged_out = Some(DragOut {
            items: there,
            allowed,
        });
        (self.wake)();
        Ok(())
    }

    fn displays(&mut self) -> Vec<Display> {
        self.displays.clone()
    }

    fn reserved_areas(&mut self) -> Vec<ReservedArea> {
        self.reserved.clone()
    }

    /// The XDG Desktop folder. (None where its name is not text: the
    /// core's file operations carry paths as text.)
    fn desktop_folder(&mut self) -> Option<String> {
        self.system.places.desktop().to_str().map(str::to_string)
    }

    fn begin_listing(&mut self, location: &Location) -> bool {
        let Location::Directory { path } = location else {
            // (The desktop is this machine's own and is listed at once.)
            return false;
        };
        let folder = PathBuf::from(path);
        let watched: Vec<WatchId> = self
            .portals
            .iter()
            .filter(|(_, portal)| portal.folder == folder)
            .map(|(watch, _)| *watch)
            .collect();
        if watched.is_empty() {
            // Not watched: read once, on a thread of its own, and told.
            return portal::once(
                location.clone(),
                folder,
                Arc::clone(&self.system),
                self.answer.clone(),
                Arc::clone(&self.wake),
            );
        }
        for watch in watched {
            let Some(portal) = self.portals.get_mut(&watch) else {
                continue;
            };
            // While it is being gone to anyway, that going is the
            // answer.
            if portal.opening {
                continue;
            }
            // (Away for the same reason as before is told again:
            // somebody asked.)
            if portal.reach.is_away() {
                portal.reach = Reach::Opening;
            }
            self.go_to(watch);
        }
        true
    }

    fn list_source(&mut self, source: &SmartSource) -> Result<Vec<Item>, BackendError> {
        self.smart.list(source)
    }

    fn watch_source(&mut self, source: &SmartSource) -> Result<WatchId, BackendError> {
        match source {
            SmartSource::Recent => self.read_recent(),
            SmartSource::Downloads => {
                let folder = self.system.places.downloads();
                if !folder.is_dir() {
                    return Err(BackendError::NotFound {
                        what: text(&folder),
                    });
                }
                self.read_downloads(folder);
            }
            _ => {}
        }
        self.numbers += 1;
        let watch = WatchId(self.numbers);
        self.smart.watch(watch, source)?;
        Ok(watch)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;

    /// A folder of a test's own, new and empty, in the system's place
    /// for temporary files (never under the home, and on a file system
    /// the kernel tells of).
    pub(crate) fn folder(name: &str) -> PathBuf {
        static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!(
            "gevrek-zones-{name}-{}-{count}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::canonicalize(&folder).expect("a real folder")
    }
}

#[cfg(test)]
mod tests;
