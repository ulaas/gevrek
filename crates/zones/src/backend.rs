//! What the zones ask of a system: one trait, implemented by Windows, by
//! Linux and by a backend in memory (`testing::FakeBackend`).
//!
//! The core itself calls nothing here. It is told what happened as
//! `BackendEvent`s and says what it wants as `BackendRequest`s; `host`
//! carries both between the core and a backend. So the core runs in a test
//! with no backend at all, and a backend is only ever called from the one
//! place.
//!
//! No pixels pass through: an icon is asked for and comes back as a
//! handle the renderer gives to whatever draws. Surfaces and their effects
//! (blur, levels) are not here either; they are the shell's.

use crate::config::SmartSource;
use crate::geometry::{Display, ReservedArea};
use crate::ids::ItemId;
use crate::item::{Item, ItemFacts};
use crate::paths;
use gevrek_model::geometry::Point;
use serde::{Deserialize, Serialize};

/// Where items are listed from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum Location {
    /// The desktop: on Windows the shell's own (the person's folder, the
    /// public one, and what is no file), on Linux the XDG Desktop folder.
    Desktop,
    /// A folder a portal shows.
    Directory { path: String },
}

impl std::fmt::Display for Location {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Location::Desktop => formatter.write_str("the desktop"),
            Location::Directory { path } => formatter.write_str(path),
        }
    }
}

/// A watch that was started, to end it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WatchId(pub u64);

/// An icon or thumbnail that was asked for; the answer names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IconRequest(pub u64);

/// A picture the backend holds, by a number of its own. The renderer asks
/// the backend's drawing side for it; the core only passes it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IconHandle(pub u64);

/// How large a picture is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IconSize {
    /// The side in logical units.
    pub size: u32,
    /// How many pixels a logical unit is on the display it is for.
    pub scale: f64,
}

/// Something done to files, as text paths. What a portal does when
/// something is dropped on it, and what a rule that moves files asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum FileOperation {
    Copy { from: String, to_folder: String },
    Move { from: String, to_folder: String },
    Rename { path: String, new_name: String },
    NewFolder { parent: String, name: String },
    /// Into the system's trash, never deleted outright.
    Trash { path: String },
    /// Out of the trash, back to where it was.
    Restore { path: String },
}

impl FileOperation {
    /// What takes this back, had it gone exactly as asked. The backend
    /// knows better once it has done it (a file may get another name where
    /// one of its name is), and says so; this is what is assumed until
    /// then.
    pub fn reversed(&self) -> Option<FileOperation> {
        Some(match self {
            FileOperation::Copy { from, to_folder } => FileOperation::Trash {
                path: paths::join(to_folder, paths::file_name(from)),
            },
            FileOperation::Move { from, to_folder } => FileOperation::Move {
                from: paths::join(to_folder, paths::file_name(from)),
                to_folder: paths::parent(from)?.to_string(),
            },
            FileOperation::Rename { path, new_name } => FileOperation::Rename {
                path: paths::join(paths::parent(path)?, new_name),
                new_name: paths::file_name(path).to_string(),
            },
            FileOperation::NewFolder { parent, name } => FileOperation::Trash {
                path: paths::join(parent, name),
            },
            FileOperation::Trash { path } => FileOperation::Restore { path: path.clone() },
            FileOperation::Restore { path } => FileOperation::Trash { path: path.clone() },
        })
    }
}

/// As the log says it: what is done, to what.
impl std::fmt::Display for FileOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileOperation::Copy { from, to_folder } => write!(formatter, "copying {from} into {to_folder}"),
            FileOperation::Move { from, to_folder } => write!(formatter, "moving {from} into {to_folder}"),
            FileOperation::Rename { path, new_name } => write!(formatter, "renaming {path} to {new_name}"),
            FileOperation::NewFolder { parent, name } => write!(formatter, "making the folder {name} in {parent}"),
            FileOperation::Trash { path } => write!(formatter, "putting {path} in the trash"),
            FileOperation::Restore { path } => write!(formatter, "bringing {path} back from the trash"),
        }
    }
}

/// What is carried by something dragged over a zone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DropPayload {
    /// Items of the desktop or of a portal, when the drag began in a zone.
    pub items: Vec<ItemId>,
    /// Files from elsewhere (a file manager), as paths.
    pub paths: Vec<String>,
    /// Text or a link, when it is neither.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// What the source lets be done with it.
    pub allowed: DragActions,
}

impl DropPayload {
    /// What it carries, in words for the log: how many of each, never
    /// the text itself.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.items.is_empty() {
            parts.push(crate::timed::count(self.items.len(), "item", "items"));
        }
        if !self.paths.is_empty() {
            parts.push(crate::timed::count(self.paths.len(), "file", "files"));
        }
        if self.text.is_some() {
            parts.push("text".to_string());
        }
        if parts.is_empty() {
            "nothing".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// What may be done with what is dragged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DragActions {
    pub copy: bool,
    pub moving: bool,
    pub link: bool,
}

impl DragActions {
    pub const ALL: DragActions = DragActions { copy: true, moving: true, link: true };
}

/// Why the backend could not do something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum BackendError {
    /// The item, file or folder is not there.
    NotFound { what: String },
    /// Something of that name is there already.
    Exists { what: String },
    /// The system said no (rights, a file in use).
    Refused { what: String },
    /// This system cannot do it at all.
    Unsupported,
    /// It was called off before it ended (`BackendRequest::Cancel`).
    Cancelled,
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::NotFound { what } => write!(formatter, "{what} is not there"),
            BackendError::Exists { what } => write!(formatter, "{what} is there already"),
            BackendError::Refused { what } => write!(formatter, "the system refused: {what}"),
            BackendError::Unsupported => write!(formatter, "not possible on this system"),
            BackendError::Cancelled => write!(formatter, "called off"),
        }
    }
}

impl std::error::Error for BackendError {}

/// What happened outside, told to the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BackendEvent {
    /// Everything at a location, as it is now (at the start, and whenever
    /// the backend cannot say what changed).
    Listed { location: Location, items: Vec<Item> },
    ItemAdded { location: Location, item: Item },
    ItemRemoved { location: Location, item: ItemId },
    /// An item has another name, and with it perhaps another id.
    ItemRenamed { location: Location, from: ItemId, item: Item },
    /// The facts of an item changed (it was written to).
    ItemChanged { location: Location, item: Item },
    /// The displays are others, or other than they were, or the room kept
    /// on them is.
    Displays { displays: Vec<Display>, reserved: Vec<ReservedArea> },
    /// Another workspace is shown. (This one comes from the window
    /// manager, by way of whoever hosts the core.)
    Workspace { name: Option<String> },
    /// A file operation the core asked for is done, or failed.
    OperationDone {
        request: u64,
        /// What takes it back, as the backend knows it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        undo: Option<FileOperation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<BackendError>,
    },
    /// A picture that was asked for is there (or there is none). For the
    /// renderer; the core passes over it.
    IconReady {
        request: IconRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<IconHandle>,
    },
    /// A location could not be listed: the folder is not there, a share
    /// does not answer, the system said no. Its zone stays and says so;
    /// a later `Listed` for the same location makes it good again.
    ListingFailed { location: Location, error: BackendError },
    /// Everything a smart zone's source has, as it is now, in the
    /// source's own order (the files used last: the newest first). Told
    /// whole each time it changes. The programs that run are known to
    /// whoever hosts the core rather than to the backend, and may be told
    /// by them with this same event.
    SourceListed { source: SmartSource, items: Vec<Item> },
    /// A source could not be asked.
    SourceFailed { source: SmartSource, error: BackendError },
    /// The screen is shown to others or recorded, or no longer is. From
    /// whoever knows: the compositor, where gevrek is one.
    ScreenShared { shared: bool },
    /// A file operation was begun and ends later (`begin_operation`
    /// said so). Told by whoever carries the request to the backend; its
    /// end is the `OperationDone` of the same request.
    OperationBegun { request: u64 },
    /// How far an operation that ends later has come, in whatever the
    /// backend counts (bytes, files); a `total` of 0 is "not known".
    OperationProgress { request: u64, done: u64, total: u64 },
}

/// What the core wants of the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum BackendRequest {
    /// List a folder and tell of every change in it from now on.
    Watch { directory: String },
    Unwatch { directory: String },
    /// Carry out a file operation and answer with `OperationDone`.
    Perform { request: u64, operation: FileOperation },
    /// List a watched folder again (it was not there, and may be now).
    Relist { directory: String },
    /// Say what a source has, and tell again whenever that changes.
    WatchSource { source: SmartSource },
    UnwatchSource { source: SmartSource },
    /// Say again what a watched source has.
    ListSource { source: SmartSource },
    /// Call off a file operation that has not ended. The backend answers
    /// with `OperationDone` all the same: with `Cancelled`, or with its
    /// result if it had ended already.
    Cancel { request: u64 },
}

/// The system, as far as zones need it.
pub trait ZonesBackend {
    /// What is at a location now.
    fn list(&mut self, location: &Location) -> Result<Vec<Item>, BackendError>;

    /// Tell of every change at a location from now on. The backend is
    /// told by the system (it does not look again and again); what it was
    /// told is handed over by `drain_events`.
    fn watch(&mut self, location: &Location) -> Result<WatchId, BackendError>;

    fn unwatch(&mut self, watch: WatchId);

    /// What happened since this was last called, in the order it happened.
    /// Whoever hosts the core calls it when the backend wakes it.
    fn drain_events(&mut self) -> Vec<BackendEvent>;

    /// The facts of one item, read now.
    fn facts(&mut self, item: &ItemId) -> Option<ItemFacts>;

    /// Ask for an item's icon. It is made off the thread that draws and
    /// arrives as `BackendEvent::IconReady`.
    fn request_icon(&mut self, item: &ItemId, size: IconSize) -> IconRequest;

    /// Ask for a small picture of what a file holds, the same way. Where
    /// there is none, the answer carries no handle and the icon is shown.
    fn request_thumbnail(&mut self, item: &ItemId, size: IconSize) -> IconRequest;

    /// Open an item as a double click would.
    fn open(&mut self, item: &ItemId) -> Result<(), BackendError>;

    /// Open an item with a program; without one, let the person choose.
    fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError>;

    /// Show the system's own menu for these items at a point of the
    /// desktop (on Linux, where there is no shell to ask, the backend's).
    fn context_menu(&mut self, items: &[ItemId], at: Point) -> Result<(), BackendError>;

    /// Carry out a file operation; the answer is what takes it back.
    fn perform(&mut self, operation: &FileOperation) -> Result<FileOperation, BackendError>;

    /// What the drag that is over a zone now carries, if any.
    fn drop_payload(&mut self) -> Option<DropPayload>;

    /// Begin a drag of items out of a zone.
    fn start_drag(&mut self, items: &[ItemId], allowed: DragActions) -> Result<(), BackendError>;

    fn displays(&mut self) -> Vec<Display>;

    /// The room panels keep at the displays' sides.
    fn reserved_areas(&mut self) -> Vec<ReservedArea>;

    /// The folder the desktop's own files lie in (the person's Desktop
    /// folder): where a file dropped on the desktop from elsewhere is
    /// copied or moved to. `None` where the backend cannot say.
    fn desktop_folder(&mut self) -> Option<String> {
        None
    }

    /// Write down how the system's own desktop icons are, before anything
    /// of them is changed (Windows: Explorer's view and where its icons
    /// are). Nothing to do where the system draws none.
    fn journal_system_icons(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    /// Whether something is written down that has not been put back (the
    /// last run ended without doing so).
    fn system_icons_to_restore(&mut self) -> bool {
        false
    }

    /// Put the system's own icons back as they were written down.
    fn restore_system_icons(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    /// List a location without making the caller wait: the answer comes
    /// later through `drain_events`, as `Listed` or `ListingFailed`. For
    /// folders that may be slow (a share, a drive that sleeps). A backend
    /// that does not do this says `false`, and is asked with `list`
    /// instead.
    fn begin_listing(&mut self, _location: &Location) -> bool {
        false
    }

    /// What a smart zone's source has now, in the source's own order.
    /// The desktop and plain folders are never asked for here (they are
    /// locations); a backend that does not know a source says so.
    fn list_source(&mut self, _source: &SmartSource) -> Result<Vec<Item>, BackendError> {
        Err(BackendError::Unsupported)
    }

    /// Tell whenever what a source has changes, as
    /// `BackendEvent::SourceListed` with all of it.
    fn watch_source(&mut self, _source: &SmartSource) -> Result<WatchId, BackendError> {
        Err(BackendError::Unsupported)
    }

    /// Begin a file operation that may take long (a large copy, a move to
    /// another drive) without making the caller wait: it is carried out
    /// off the caller's thread and its end comes through `drain_events`
    /// as `BackendEvent::OperationDone` with this `request`, as `perform`
    /// would have answered; `OperationProgress` may come before. A
    /// backend that does not do this says `false` and is asked with
    /// `perform` instead.
    fn begin_operation(&mut self, _request: u64, _operation: &FileOperation) -> bool {
        false
    }

    /// Call off an operation that was begun. Its `OperationDone` still
    /// comes: with `BackendError::Cancelled`, having put back what it
    /// could, or with its result if it was too late.
    fn cancel_operation(&mut self, _request: u64) {}

    /// Whether the screen is shown to others or recorded now; `None`
    /// where the system gives no sign of it (then only
    /// `BackendEvent::ScreenShared` from elsewhere, or the person, says).
    fn screen_shared(&mut self) -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_takes_an_operation_back_takes_it_back() {
        let folder = "/home/ada/Desktop";
        for (operation, back) in [
            (
                FileOperation::Move { from: format!("{folder}/a.txt"), to_folder: "/archive".to_string() },
                FileOperation::Move { from: "/archive/a.txt".to_string(), to_folder: folder.to_string() },
            ),
            (
                FileOperation::Copy { from: format!("{folder}/a.txt"), to_folder: "/archive".to_string() },
                FileOperation::Trash { path: "/archive/a.txt".to_string() },
            ),
            (
                FileOperation::Rename { path: format!("{folder}/a.txt"), new_name: "b.txt".to_string() },
                FileOperation::Rename { path: format!("{folder}/b.txt"), new_name: "a.txt".to_string() },
            ),
            (
                FileOperation::NewFolder { parent: folder.to_string(), name: "New".to_string() },
                FileOperation::Trash { path: format!("{folder}/New") },
            ),
            (
                FileOperation::Trash { path: format!("{folder}/a.txt") },
                FileOperation::Restore { path: format!("{folder}/a.txt") },
            ),
        ] {
            assert_eq!(operation.reversed(), Some(back), "{operation:?}");
        }
        // A move is its own way back, twice over.
        let there = FileOperation::Move {
            from: "C:\\Users\\ada\\Desktop\\a.txt".to_string(),
            to_folder: "D:\\Old".to_string(),
        };
        assert_eq!(there.reversed().unwrap().reversed().unwrap(), there);
    }

    #[test]
    fn events_and_requests_travel_as_json() {
        let event = BackendEvent::ItemRemoved {
            location: Location::Directory { path: "/downloads".to_string() },
            item: ItemId::new("/downloads/a.txt"),
        };
        let text = serde_json::to_string(&event).unwrap();
        assert_eq!(
            text,
            r#"{"event":"item_removed","location":{"at":"directory","path":"/downloads"},"item":"/downloads/a.txt"}"#
        );
        assert_eq!(serde_json::from_str::<BackendEvent>(&text).unwrap(), event);
        let request = BackendRequest::Perform {
            request: 3,
            operation: FileOperation::Trash { path: "/a".to_string() },
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<BackendRequest>(&text).unwrap(), request);
        let source = BackendRequest::WatchSource { source: SmartSource::Recent };
        assert_eq!(
            serde_json::to_string(&source).unwrap(),
            r#"{"ask":"watch_source","source":{"from":"recent"}}"#
        );
        let failed = BackendEvent::ListingFailed {
            location: Location::Directory { path: "//share/films".to_string() },
            error: BackendError::NotFound { what: "//share/films".to_string() },
        };
        let text = serde_json::to_string(&failed).unwrap();
        assert_eq!(serde_json::from_str::<BackendEvent>(&text).unwrap(), failed);
        assert_eq!(
            serde_json::to_string(&BackendEvent::ScreenShared { shared: true }).unwrap(),
            r#"{"event":"screen_shared","shared":true}"#
        );
    }
}
