//! Being told what changes: on the desktop, in the folders zones show,
//! and among the displays.
//!
//! Nothing is looked at by the clock. Three things tell:
//!
//! - **The file system**, of a folder (the `notify` crate:
//!   `ReadDirectoryChangesW`), on that crate's thread: of a folder a
//!   portal shows, and of the two folders the desktop is made of.
//! - **The shell**, of the desktop (`SHChangeNotifyRegister`): of what
//!   the file system cannot tell. That is everything that is no file
//!   (the Recycle Bin filling up, This PC being switched on in the
//!   desktop's settings), and what programs say to the shell rather than
//!   do to a file. The shell tells a window by a message, so there is a
//!   window nobody sees, on a thread of its own that sleeps in its queue
//!   until a message comes.
//! - **Windows**, of the displays: the same hidden window is told when a
//!   display comes, goes or changes, and when the room panels keep does.
//!
//! The file system and the shell often tell of the same change, each in
//! its own words. What is told is measured against what was known
//! (`seen`), so the core hears each thing once; it is laid down in
//! `News`, and whoever hosts the core is woken and takes it from there
//! with `drain_events`.
//!
//! (The shell could tell of files as well, and is asked to. On the
//! Windows this was written on it did not, for a folder nobody else was
//! watching; the file system always does.)

use super::seen::{Change, Seen};
use super::shell::{self, desktop_folders, on_the_desktop, Pidl, Types};
use crate::com::Com;
use gevrek_zones::backend::{BackendError, BackendEvent, Location};
use gevrek_zones::ids::ItemId;
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILClone, SHCNRF_InterruptLevel, SHCNRF_NewDelivery, SHCNRF_ShellLevel,
    SHChangeNotification_Lock, SHChangeNotification_Unlock, SHChangeNotifyDeregister,
    SHChangeNotifyEntry, SHChangeNotifyRegister, SHCNE_ATTRIBUTES, SHCNE_CREATE, SHCNE_DELETE,
    SHCNE_ID, SHCNE_INTERRUPT, SHCNE_MKDIR, SHCNE_RENAMEFOLDER, SHCNE_RENAMEITEM, SHCNE_RMDIR,
    SHCNE_UPDATEDIR, SHCNE_UPDATEIMAGE, SHCNE_UPDATEITEM, SHCNRF_SOURCE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, HMENU, MSG, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

/// The shell tells of a change on the desktop.
const TOLD: u32 = WM_APP + 1;
/// Begin and end listening to the shell.
const LISTEN: u32 = WM_APP + 2;
const DEAFEN: u32 = WM_APP + 3;
/// What `WM_SETTINGCHANGE` carries when the room for windows changed
/// (`SPI_SETWORKAREA`).
const WORK_AREA: usize = 0x2F;

/// What happened and was not handed over yet, and how the host is woken.
#[derive(Default)]
pub struct News {
    events: Mutex<Vec<BackendEvent>>,
    wake: Mutex<Option<Box<dyn Fn() + Send>>>,
    /// Windows said its displays changed, or the room on them.
    displays: AtomicBool,
    /// How often the shell told of a change, whatever came of it.
    heard: std::sync::atomic::AtomicUsize,
}

impl News {
    /// How the host is woken, from now on.
    pub fn on_news(&self, wake: Box<dyn Fn() + Send>) {
        *self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(wake);
    }

    /// There is something to drain.
    pub fn wake(&self) {
        if let Some(wake) = self
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            wake();
        }
    }

    pub fn tell(&self, events: Vec<BackendEvent>) {
        if events.is_empty() {
            return;
        }
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(events);
        self.wake();
    }

    pub fn take(&self) -> Vec<BackendEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// How often the shell has told of a change since the start.
    pub fn heard(&self) -> usize {
        self.heard.load(Ordering::SeqCst)
    }

    /// Whether Windows said its displays changed since this was last
    /// asked.
    pub fn displays_changed(&self) -> bool {
        self.displays.swap(false, Ordering::SeqCst)
    }
}

/// What is known of a watched place, shared between the one who lists it
/// (the host's thread) and the one who is told of changes (a watcher's).
pub type Shared = Arc<Mutex<Seen>>;

fn locked(seen: &Shared) -> std::sync::MutexGuard<'_, Seen> {
    seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------------
// A folder, by the file system.

/// A folder that is watched. The watching ends with this.
pub struct Folder {
    pub seen: Shared,
    /// Said when the watching ends: the file system's own end of it comes
    /// a moment later, and what it still tells until then is not told on.
    /// Held while a change is told, so that once the watching has ended
    /// nothing more is told, also of a change that was being read then.
    /// (Nothing else is waited for while it is held.)
    ended: Arc<Mutex<bool>>,
    _watcher: notify::RecommendedWatcher,
}

impl Drop for Folder {
    fn drop(&mut self) {
        *self
            .ended
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }
}

/// Watch a folder a zone shows: every change in it (not below it) is
/// told to `news`.
pub fn watch_folder(path: &str, news: Arc<News>) -> Result<Folder, BackendError> {
    let location = Location::Directory {
        path: path.to_string(),
    };
    let seen: Shared = Arc::new(Mutex::new(Seen::new(location)));
    watch_into(path, seen, news, Types::default(), || {})
}

/// Watch one of the folders the desktop is made of: what changes in it
/// is a change of the desktop, measured against what is known of that.
pub fn watch_desktop_folder(
    folder: &str,
    desktop: &Shared,
    news: Arc<News>,
    types: Types,
) -> Result<Folder, BackendError> {
    watch_into(folder, Arc::clone(desktop), news, types, || {})
}

/// Watch `path`; what changes there is a change at the place `seen` is
/// about. `lost` is called (on the watcher's thread) when the folder
/// itself seems to be gone: it was removed, or the watching of it failed
/// (a drive pulled out, a machine that stopped answering).
pub fn watch_into(
    path: &str,
    seen: Shared,
    news: Arc<News>,
    mut types: Types,
    lost: impl Fn() + Send + 'static,
) -> Result<Folder, BackendError> {
    let folder = shell::plain(path);
    if !std::path::Path::new(&folder).is_dir() {
        return Err(BackendError::NotFound { what: folder });
    }
    let location = locked(&seen).location().clone();
    let (told, mut renamed_from) = (Arc::clone(&seen), None::<ItemId>);
    let itself = folder.clone();
    let ended = Arc::new(Mutex::new(false));
    let over = Arc::clone(&ended);
    let handler = move |event: notify::Result<notify::Event>| {
        if *over.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) {
            return;
        }
        // (The thread is the crate's own; the shell is asked for names
        // and types on it.)
        let _com = Com::new();
        let mut changes = Vec::new();
        let again = |types: &mut Types| shell::list(&location, types).ok().map(Change::Relisted);
        match event {
            // What was missed cannot be known: everything is read anew.
            Err(_) => {
                lost();
                changes.extend(again(&mut types));
            }
            Ok(event) if event.need_rescan() => changes.extend(again(&mut types)),
            // (Windows tells of a watched folder's own going this way.)
            Ok(event)
                if matches!(event.kind, EventKind::Remove(_))
                    && event.paths.first().is_some_and(|path| {
                        super::facts::same_path(&path.to_string_lossy(), &itself)
                    }) =>
            {
                lost();
            }
            Ok(event) => {
                let id = |index: usize| {
                    event
                        .paths
                        .get(index)
                        .map(|path| ItemId::new(path.to_string_lossy().into_owned()))
                };
                // (Only what a listing would show: a hidden file is none.)
                let mut item =
                    |index: usize| id(index).and_then(|id| shell::item_if_listed(&id, &mut types));
                // It is there, to be shown; or it is there and hidden
                // now, which is a going. What is not there at all any
                // more is left to the telling of its going or renaming
                // that follows: said here, a file that is written and
                // renamed at once would go and come instead of keeping
                // its place.
                let mut there = |index: usize| match item(index) {
                    Some(item) => Some(Change::Came(item)),
                    None => id(index)
                        .filter(|id| std::path::Path::new(id.as_str()).exists())
                        .map(Change::Went),
                };
                match event.kind {
                    EventKind::Create(_) => changes.extend(there(0)),
                    EventKind::Remove(_) => changes.extend(id(0).map(Change::Went)),
                    EventKind::Modify(ModifyKind::Name(RenameMode::From)) => renamed_from = id(0),
                    EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                        changes.extend(match renamed_from.take() {
                            Some(from) => Some(Change::Renamed {
                                from,
                                item: item(0),
                            }),
                            None => there(0),
                        });
                    }
                    EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                        changes.extend(id(0).map(|from| Change::Renamed {
                            from,
                            item: item(1),
                        }));
                    }
                    EventKind::Modify(_) => changes.extend(there(0)),
                    _ => {}
                }
            }
        }
        // Told while what is known is held: so a listing of the whole
        // place and a change of one item reach the core in the order
        // they were measured in.
        let mut seen = locked(&told);
        let mut events = Vec::new();
        for change in changes {
            events.extend(seen.apply(change));
        }
        // (Ended while the change was read, as the shell can be slow to
        // say what an item is: it is no more news.)
        let over = over.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*over {
            news.tell(events);
        }
    };
    let refused = |error: notify::Error| BackendError::Refused {
        what: format!("{folder}: {error}"),
    };
    let mut watcher = notify::recommended_watcher(handler).map_err(refused)?;
    watcher
        .watch(std::path::Path::new(&folder), RecursiveMode::NonRecursive)
        .map_err(refused)?;
    Ok(Folder {
        seen,
        ended,
        _watcher: watcher,
    })
}

// ---------------------------------------------------------------------
// The desktop and the displays, by a window nobody sees.

/// What the hidden window's thread works with.
struct Listening {
    news: Arc<News>,
    /// The desktop is listened to: what is known of it, and the number
    /// the shell gave the listening.
    desktop: Option<(Shared, u32)>,
    /// The folders whose files are the desktop's.
    folders: Vec<String>,
    /// What is no file belongs to it as well (it is the desktop of the
    /// shell that is listened to, not one folder).
    whole: bool,
    types: Types,
}

/// What the window's thread is handed to listen: what is known of the
/// place, the one folder to listen to instead of the desktop, and where
/// to say whether the shell agreed.
type Asked = (Shared, Option<String>, mpsc::Sender<bool>);

thread_local! {
    static LISTENING: RefCell<Option<Listening>> = const { RefCell::new(None) };
}

/// The hidden window and its thread. Both end with this.
pub struct Window {
    handle: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Window {
    /// Make the window. From then on `news` is told when Windows says
    /// its displays changed.
    pub fn open(news: Arc<News>, types: Types) -> Result<Self, BackendError> {
        let (made, handle) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("gevrek-zones-watch".to_string())
            .spawn(move || {
                let _com = Com::new();
                // SAFETY: the window is made, used and destroyed on this
                // thread.
                let window = unsafe { make_window() };
                let _ = made.send(window.map(|window| window.0 as isize));
                let Some(window) = window else {
                    return;
                };
                LISTENING.with(|listening| {
                    *listening.borrow_mut() = Some(Listening {
                        news,
                        desktop: None,
                        folders: Vec::new(),
                        whole: true,
                        types,
                    });
                });
                let mut message = MSG::default();
                // SAFETY: the plain loop of a window's thread. It sleeps
                // in `GetMessageW` until there is a message.
                unsafe {
                    while GetMessageW(&mut message, HWND::default(), 0, 0).as_bool() {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                    deafen();
                    let _ = DestroyWindow(window);
                }
                LISTENING.with(|listening| *listening.borrow_mut() = None);
            })
            .map_err(|error| BackendError::Refused {
                what: format!("no thread: {error}"),
            })?;
        match handle.recv() {
            Ok(Some(handle)) => Ok(Self {
                handle,
                thread: Some(thread),
            }),
            _ => Err(BackendError::Refused {
                what: "no window to be told of changes by".to_string(),
            }),
        }
    }

    fn post(&self, message: u32, with: isize) -> bool {
        // SAFETY: a message to a window of this process.
        unsafe {
            PostMessageW(
                HWND(self.handle as *mut _),
                message,
                WPARAM(0),
                LPARAM(with),
            )
        }
        .is_ok()
    }

    /// Have the shell tell of the desktop from now on, measured against
    /// `seen`.
    pub fn listen_to_desktop(&self, seen: &Shared) -> Result<(), BackendError> {
        self.listen(seen, None)
    }

    /// Have the shell tell of one folder instead, as if that were all the
    /// desktop is. So the shell's telling is tried on a folder of a
    /// test's own, and the desktop of whoever runs the test is left out
    /// of it.
    #[cfg(test)]
    fn listen_to_folder(&self, seen: &Shared, folder: &str) -> Result<(), BackendError> {
        self.listen(seen, Some(folder.to_string()))
    }

    fn listen(&self, seen: &Shared, only: Option<String>) -> Result<(), BackendError> {
        let refused = || BackendError::Refused {
            what: "the desktop cannot be watched".to_string(),
        };
        let (answer, answered) = mpsc::channel();
        // (The window's thread takes it over and gives it back.)
        let handed: *mut Asked = Box::into_raw(Box::new((Arc::clone(seen), only, answer)));
        if !self.post(LISTEN, handed as isize) {
            // SAFETY: the message was not sent; the box is still ours.
            drop(unsafe { Box::from_raw(handed) });
            return Err(refused());
        }
        // The call returns when the shell has noted whom to tell: what
        // is listed after it is then never missed between the two. (The
        // window's thread does nothing but this; it answers at once.)
        match answered.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(true) => Ok(()),
            _ => Err(refused()),
        }
    }

    pub fn stop_listening_to_desktop(&self) {
        self.post(DEAFEN, 0);
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.post(WM_CLOSE, 0);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

unsafe fn make_window() -> Option<HWND> {
    static CLASS: std::sync::Once = std::sync::Once::new();
    let class = w!("Gevrek_Zones_Watch");
    let module: HINSTANCE = GetModuleHandleW(None).ok()?.into();
    CLASS.call_once(|| {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: class,
            ..Default::default()
        });
    });
    // A window of the desktop like any other, only never shown: Windows
    // tells of its displays to such windows alone (one that exists for
    // messages only is not told).
    CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class,
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        HWND::default(),
        HMENU::default(),
        module,
        None,
    )
    .ok()
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        TOLD => {
            told(wparam, lparam);
            LRESULT(0)
        }
        LISTEN => {
            // SAFETY: the pointer is the box `Window::listen` gave.
            let (seen, only, answer) = *Box::from_raw(lparam.0 as *mut Asked);
            let _ = answer.send(listen(window, seen, only));
            LRESULT(0)
        }
        DEAFEN => {
            deafen();
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            displays_changed();
            DefWindowProcW(window, message, wparam, lparam)
        }
        WM_SETTINGCHANGE if wparam.0 == WORK_AREA => {
            displays_changed();
            DefWindowProcW(window, message, wparam, lparam)
        }
        WM_CLOSE => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

fn displays_changed() {
    LISTENING.with(|listening| {
        if let Some(listening) = listening.borrow().as_ref() {
            listening.news.displays.store(true, Ordering::SeqCst);
            listening.news.wake();
        }
    });
}

/// Ask the shell to tell this window of the desktop (or of the one
/// folder `only`).
unsafe fn listen(window: HWND, seen: Shared, only: Option<String>) -> bool {
    deafen();
    let whole = only.is_none();
    let folders = only.map_or_else(desktop_folders, |folder| vec![folder]);
    // The desktop of the shell, and its two folders by their paths: a
    // change is told for whichever of them the one who made it named,
    // and being told twice does no harm (`seen`).
    let mut places: Vec<Pidl> = Vec::new();
    if whole {
        places.extend(desktop_itself());
    }
    places.extend(folders.iter().filter_map(|folder| Pidl::parse(folder)));
    let entries: Vec<SHChangeNotifyEntry> = places
        .iter()
        .map(|place| SHChangeNotifyEntry {
            pidl: place.as_ptr() as *mut ITEMIDLIST,
            fRecursive: false.into(),
        })
        .collect();
    let what = SHCNE_CREATE.0
        | SHCNE_DELETE.0
        | SHCNE_MKDIR.0
        | SHCNE_RMDIR.0
        | SHCNE_RENAMEITEM.0
        | SHCNE_RENAMEFOLDER.0
        | SHCNE_UPDATEITEM.0
        | SHCNE_UPDATEDIR.0
        | SHCNE_ATTRIBUTES.0
        | SHCNE_UPDATEIMAGE.0;
    let number = SHChangeNotifyRegister(
        window,
        SHCNRF_SOURCE(SHCNRF_ShellLevel.0 | SHCNRF_InterruptLevel.0 | SHCNRF_NewDelivery.0),
        what as i32,
        TOLD,
        entries.len() as i32,
        entries.as_ptr(),
    );
    if number == 0 {
        log::warn!("zones: the shell does not tell of the desktop's changes");
        return false;
    }
    LISTENING.with(|listening| {
        if let Some(listening) = listening.borrow_mut().as_mut() {
            listening.desktop = Some((seen, number));
            listening.folders = folders;
            listening.whole = whole;
        }
    });
    true
}

/// The shell's name for the desktop itself: the list with nothing in it.
fn desktop_itself() -> Option<Pidl> {
    let empty = ITEMIDLIST::default();
    // SAFETY: a list that ends at once is a list; the copy is ours.
    unsafe { Pidl::from_raw(ILClone(&empty)) }
}

unsafe fn deafen() {
    LISTENING.with(|listening| {
        if let Some((_, number)) = listening
            .borrow_mut()
            .as_mut()
            .and_then(|listening| listening.desktop.take())
        {
            let _ = SHChangeNotifyDeregister(number);
        }
    });
}

/// The shell told of a change: read what it was, measure it against
/// what is known, tell the core.
unsafe fn told(wparam: WPARAM, lparam: LPARAM) {
    let mut lists: *mut *mut ITEMIDLIST = std::ptr::null_mut();
    let mut what = 0i32;
    let lock = SHChangeNotification_Lock(
        HANDLE(wparam.0 as *mut _),
        lparam.0 as u32,
        Some(&mut lists),
        Some(&mut what),
    );
    if lock.is_invalid() {
        return;
    }
    // (The lists are the shell's and live while the lock is held: what
    // is needed of them is copied.)
    let copy = |index: usize| {
        if lists.is_null() {
            return None;
        }
        let list = *lists.add(index);
        if list.is_null() {
            None
        } else {
            Pidl::from_raw(ILClone(list))
        }
    };
    let (first, second) = (copy(0), copy(1));
    let _ = SHChangeNotification_Unlock(lock);
    let what = what as u32 & !SHCNE_INTERRUPT.0;

    // What the thread works with is taken out for the length of the work
    // and not held: asking the shell can let another message in, and
    // that one must find everything free.
    let Some((seen, folders, whole, news, mut types)) = LISTENING.with(|listening| {
        if let Some(listening) = listening.borrow().as_ref() {
            listening.news.heard.fetch_add(1, Ordering::SeqCst);
        }
        let mut listening = listening.borrow_mut();
        let listening = listening.as_mut()?;
        let (seen, _) = listening.desktop.clone()?;
        Some((
            seen,
            listening.folders.clone(),
            listening.whole,
            Arc::clone(&listening.news),
            std::mem::take(&mut listening.types),
        ))
    }) else {
        return;
    };
    let place = locked(&seen).location().clone();
    // What lies at the place that is listened to.
    let on_desktop = |id: &str| {
        if whole {
            on_the_desktop(id, &folders)
        } else {
            super::facts::is_path(id) && on_the_desktop(id, &folders)
        }
    };
    let change = {
        let types = &mut types;
        // The id a list names, when what it names lies on the desktop.
        let id = |list: &Option<Pidl>| {
            list.as_ref()
                .and_then(Pidl::parsing_name)
                .filter(|name| on_desktop(name))
                .map(ItemId::new)
        };
        let mut item = |list: &Option<Pidl>| {
            let list = list.as_ref()?;
            let item = shell::item_by_pidl(list, types)?;
            on_desktop(item.id.as_str()).then_some(item)
        };
        let is = |event: SHCNE_ID| what & event.0 != 0;
        if is(SHCNE_RENAMEITEM) || is(SHCNE_RENAMEFOLDER) {
            let to = item(&second);
            match id(&first) {
                Some(from) => Some(Change::Renamed { from, item: to }),
                None => to.map(Change::Came),
            }
        } else if is(SHCNE_CREATE) || is(SHCNE_MKDIR) {
            item(&first).map(Change::Came)
        } else if is(SHCNE_DELETE) || is(SHCNE_RMDIR) {
            id(&first).map(Change::Went)
        } else if is(SHCNE_UPDATEITEM) || is(SHCNE_ATTRIBUTES) {
            item(&first).map(Change::Touched)
        } else if is(SHCNE_UPDATEIMAGE) {
            Some(Change::Pictures)
        } else if is(SHCNE_UPDATEDIR) {
            // "Look again": only when it is the desktop that is meant,
            // not a folder that lies on it.
            let meant = first
                .as_ref()
                .and_then(Pidl::parsing_name)
                .unwrap_or_default();
            let the_desktop = whole
                && (meant.is_empty() || !super::facts::is_path(&meant) && !meant.contains('\\'))
                || folders
                    .iter()
                    .any(|folder| super::facts::same_path(folder, &meant));
            the_desktop
                .then(|| shell::list(&place, types).ok())
                .flatten()
                .map(Change::Relisted)
        } else {
            None
        }
    };
    LISTENING.with(|listening| {
        if let Some(listening) = listening.borrow_mut().as_mut() {
            listening.types = types;
        }
    });
    if let Some(change) = change {
        let events = locked(&seen).apply(change);
        news.tell(events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::item::ItemKind;
    use std::time::{Duration, Instant};

    /// A folder of this test's own under the system's folder for
    /// temporary files, by its long name. Nothing outside it is touched.
    fn folder(name: &str) -> String {
        let folder =
            std::env::temp_dir().join(format!("gevrek-zones-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::canonicalize(&folder)
            .expect("the folder")
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .to_string()
    }

    /// Wait until the events told so far hold one that `wanted` takes;
    /// what was told is kept in `all`.
    fn until(
        news: &News,
        wakes: &mpsc::Receiver<()>,
        all: &mut Vec<BackendEvent>,
        wanted: impl Fn(&BackendEvent) -> bool,
    ) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            all.extend(news.take());
            if all.iter().any(&wanted) {
                return;
            }
            let left = end.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "not told in time; told so far: {all:#?}");
            // (Nothing is looked at by the clock: the wait ends when the
            // watcher wakes it.)
            let _ = wakes.recv_timeout(left);
        }
    }

    #[test]
    fn a_folder_tells_what_comes_is_renamed_changes_and_goes() {
        let root = folder("watch");
        let news = Arc::new(News::default());
        let (woken, wakes) = mpsc::channel();
        news.on_news(Box::new(move || {
            let _ = woken.send(());
        }));
        let watched = watch_folder(&root, Arc::clone(&news)).expect("watched");
        let at = Location::Directory { path: root.clone() };
        locked(&watched.seen).listed(&shell::list(&at, &mut Types::default()).expect("listed"));
        let path = |name: &str| format!("{root}\\{name}");
        let mut all = Vec::new();

        std::fs::write(path("a.txt"), b"one").expect("written");
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemAdded { location, item }
                if location == &at && item.id.as_str() == path("a.txt") && item.kind == ItemKind::File)
        });

        std::fs::rename(path("a.txt"), path("b.txt")).expect("renamed");
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemRenamed { from, item, .. }
                if from.as_str() == path("a.txt") && item.id.as_str() == path("b.txt"))
        });

        std::fs::write(path("b.txt"), b"one and more").expect("written");
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemChanged { item, .. }
                if item.id.as_str() == path("b.txt") && item.facts.size == Some(12))
        });

        std::fs::create_dir(path("Folder")).expect("made");
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemAdded { item, .. }
                if item.id.as_str() == path("Folder") && item.kind == ItemKind::Folder)
        });
        // What happens inside a folder that lies there is not this
        // folder's business; its going is.
        std::fs::write(path("Folder\\deep.txt"), b"deep").expect("written");
        std::fs::remove_file(path("Folder\\deep.txt")).expect("removed");
        std::fs::remove_dir(path("Folder")).expect("removed");
        std::fs::remove_file(path("b.txt")).expect("removed");
        until(
            &news,
            &wakes,
            &mut all,
            |event| matches!(event, BackendEvent::ItemRemoved { item, .. } if item.as_str() == path("b.txt")),
        );
        until(
            &news,
            &wakes,
            &mut all,
            |event| matches!(event, BackendEvent::ItemRemoved { item, .. } if item.as_str() == path("Folder")),
        );
        assert!(
            !all.iter()
                .any(|event| format!("{event:?}").contains("deep.txt")),
            "what is below the folder was told: {all:#?}"
        );
        // Each coming and going once.
        let count = |wanted: &dyn Fn(&BackendEvent) -> bool| {
            all.iter().filter(|event| wanted(event)).count()
        };
        assert_eq!(
            count(&|event| matches!(event, BackendEvent::ItemAdded { .. })),
            2,
            "{all:#?}"
        );
        assert_eq!(
            count(&|event| matches!(event, BackendEvent::ItemRemoved { .. })),
            2,
            "{all:#?}"
        );
        assert_eq!(
            count(&|event| matches!(event, BackendEvent::ItemRenamed { .. })),
            1,
            "{all:#?}"
        );

        // Ended, nothing more is told.
        drop(watched);
        std::fs::write(path("late.txt"), b"late").expect("written");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(news.take(), []);
        assert!(watch_folder(&path("not there"), Arc::clone(&news)).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Say to the shell what a program says when it has done something
    /// to a file (Explorer and the shell's own file operations do).
    fn say(what: SHCNE_ID, first: &str, second: Option<&str>) {
        use windows::Win32::UI::Shell::{SHChangeNotify, SHCNF_FLUSH, SHCNF_PATHW};
        let first = windows::core::HSTRING::from(first);
        let second = second.map(windows::core::HSTRING::from);
        // SAFETY: both texts end with a zero and live until the call
        // returns, which is when the shell has passed it on (`FLUSH`).
        unsafe {
            SHChangeNotify(
                what,
                SHCNF_PATHW | SHCNF_FLUSH,
                Some(first.as_ptr() as *const _),
                second.as_ref().map(|second| second.as_ptr() as *const _),
            );
        }
    }

    /// The shell's own telling, on a folder of the test's: what it is
    /// told with is read, measured and told on.
    #[test]
    fn the_shell_tells_of_a_folder_it_is_asked_about() {
        let root = folder("shell-told");
        let news = Arc::new(News::default());
        let (woken, wakes) = mpsc::channel();
        news.on_news(Box::new(move || {
            let _ = woken.send(());
        }));
        let window = Window::open(Arc::clone(&news), Types::default()).expect("a window");
        let at = Location::Directory { path: root.clone() };
        let seen: Shared = Arc::new(Mutex::new(Seen::new(at.clone())));
        window.listen_to_folder(&seen, &root).expect("listening");
        let path = |name: &str| format!("{root}\\{name}");
        let mut all = Vec::new();
        // The shell takes a moment until it tells one who just asked.
        // Said again until it is heard once (the folder is empty, and
        // "look again" finds nothing to tell on).
        let patience = Instant::now() + Duration::from_secs(20);
        while news.heard() == 0 {
            assert!(Instant::now() < patience, "the shell never told");
            say(SHCNE_UPDATEDIR, &root, None);
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(news.take(), []);

        std::fs::write(path("a.txt"), b"one").expect("written");
        say(SHCNE_CREATE, &path("a.txt"), None);
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemAdded { location, item }
                if location == &at && item.id.as_str() == path("a.txt"))
        });
        std::fs::rename(path("a.txt"), path("b.txt")).expect("renamed");
        say(SHCNE_RENAMEITEM, &path("a.txt"), Some(&path("b.txt")));
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemRenamed { from, item, .. }
                if from.as_str() == path("a.txt") && item.id.as_str() == path("b.txt"))
        });
        std::fs::create_dir(path("Folder")).expect("made");
        say(SHCNE_MKDIR, &path("Folder"), None);
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemAdded { item, .. }
                if item.id.as_str() == path("Folder") && item.kind == ItemKind::Folder)
        });
        std::fs::write(path("b.txt"), b"one and more").expect("written");
        say(SHCNE_UPDATEITEM, &path("b.txt"), None);
        until(&news, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::ItemChanged { item, .. }
                if item.id.as_str() == path("b.txt") && item.facts.size == Some(12))
        });
        // What happens below is not told on.
        std::fs::write(path("Folder\\deep.txt"), b"deep").expect("written");
        say(SHCNE_CREATE, &path("Folder\\deep.txt"), None);
        std::fs::remove_file(path("Folder\\deep.txt")).expect("removed");
        // "Look again": what came and went without a word is found.
        std::fs::write(path("quiet.txt"), b"quiet").expect("written");
        std::fs::remove_file(path("b.txt")).expect("removed");
        say(SHCNE_UPDATEDIR, &root, None);
        until(
            &news,
            &wakes,
            &mut all,
            |event| matches!(event, BackendEvent::ItemAdded { item, .. } if item.id.as_str() == path("quiet.txt")),
        );
        until(
            &news,
            &wakes,
            &mut all,
            |event| matches!(event, BackendEvent::ItemRemoved { item, .. } if item.as_str() == path("b.txt")),
        );
        std::fs::remove_dir(path("Folder")).expect("removed");
        say(SHCNE_RMDIR, &path("Folder"), None);
        std::fs::remove_file(path("quiet.txt")).expect("removed");
        say(SHCNE_DELETE, &path("quiet.txt"), None);
        until(
            &news,
            &wakes,
            &mut all,
            |event| matches!(event, BackendEvent::ItemRemoved { item, .. } if item.as_str() == path("Folder")),
        );
        until(
            &news,
            &wakes,
            &mut all,
            |event| matches!(event, BackendEvent::ItemRemoved { item, .. } if item.as_str() == path("quiet.txt")),
        );
        // Each coming and going once, and nothing of what lies deeper.
        let count = |wanted: &dyn Fn(&BackendEvent) -> bool| {
            all.iter().filter(|event| wanted(event)).count()
        };
        assert_eq!(
            count(&|event| matches!(event, BackendEvent::ItemAdded { .. })),
            3,
            "{all:#?}"
        );
        assert_eq!(
            count(&|event| matches!(event, BackendEvent::ItemRemoved { .. })),
            3,
            "{all:#?}"
        );
        assert_eq!(
            count(&|event| matches!(event, BackendEvent::ItemRenamed { .. })),
            1,
            "{all:#?}"
        );
        assert!(
            !all.iter()
                .any(|event| format!("{event:?}").contains("deep.txt")),
            "{all:#?}"
        );
        drop(window);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_hidden_window_opens_listens_and_closes() {
        let news = Arc::new(News::default());
        let window = Window::open(Arc::clone(&news), Types::default()).expect("a window");
        let seen: Shared = Arc::new(Mutex::new(Seen::new(Location::Desktop)));
        // (Listening changes nothing of the desktop; the shell only
        // notes whom to tell.)
        window.listen_to_desktop(&seen).expect("listening");
        window.stop_listening_to_desktop();
        drop(window);
        // The thread has ended and let go of what it was handed.
        assert_eq!(Arc::strong_count(&seen), 1);
        assert!(!news.displays_changed());
        let folders = desktop_folders();
        assert!(!folders.is_empty());
        assert!(
            folders
                .iter()
                .all(|folder| super::super::facts::is_path(folder)),
            "{folders:?}"
        );
    }
}
