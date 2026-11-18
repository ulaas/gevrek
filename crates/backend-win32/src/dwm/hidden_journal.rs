//! Crash-safe journal of every window Gevrek has hidden or cloaked.
//!
//! Every hide goes through `WindowManager::hide_window` / `cloak_window`, and
//! both record the window here; `show_window` / uncloak forget it. The set is
//! process-global so that the panic hook, the console control handler, and the
//! session-end window can restore windows without access to the runtime.
//!
//! When persistence is enabled (only the WM binary does this), the set is
//! mirrored to `%LOCALAPPDATA%\gevrek\hidden.json` on every change. On the next
//! start, [`recover_orphans`] unhides whatever a crashed or killed instance
//! left behind, so a window can never stay invisible across a restart.
//! Entries carry the owning PID and window class, so a reused HWND that now
//! belongs to some other (legitimately hidden) window is left alone.

use super::HwndExt;
use super::WindowHandle;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, ShowWindowAsync, SW_SHOWNA,
};

const DWMWA_CLOAK: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(13);

/// Identity of a hidden window, used to validate an entry after a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub pid: u32,
    pub class: String,
    /// Hidden with `DWMWA_CLOAK` (needs an uncloak) rather than `SW_HIDE`.
    pub cloaked: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JournalFile {
    /// HWND value → identity. A map keyed by the raw handle so the file is
    /// stable and human-readable.
    windows: BTreeMap<isize, JournalEntry>,
}

#[derive(Debug, Default)]
struct Journal {
    entries: BTreeMap<isize, JournalEntry>,
    path: Option<PathBuf>,
}

impl Journal {
    /// Returns true when the set changed.
    fn insert(&mut self, handle: isize, entry: JournalEntry) -> bool {
        self.entries.insert(handle, entry.clone()).as_ref() != Some(&entry)
    }

    fn remove(&mut self, handle: isize) -> Option<JournalEntry> {
        self.entries.remove(&handle)
    }

    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Err(e) = write_file(path, &self.entries) {
            log::warn!("Failed to persist hidden-window journal: {e}");
        }
    }
}

/// What is in memory is not yet in the file.
static UNWRITTEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// When the file was last written.
static WRITTEN_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Within this long of the last writing, a change waits for the end of
/// the window manager's pass (`flush`). A workspace that is shown hides
/// and shows many windows at once; each used to write the whole file, a
/// dozen writings for one key, each looked at by whatever watches files
/// on the machine. The first of a run is written at once as before (a
/// single window hidden is in the file when it is hidden); the rest of
/// the run is written together, a moment later.
const WRITTEN_TOGETHER_WITHIN: Duration = Duration::from_millis(100);

/// The file is to say what is in memory: now, or with the changes that
/// follow at once.
fn written(journal: &Journal) {
    let mut at = WRITTEN_AT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if at.is_none_or(|at| at.elapsed() >= WRITTEN_TOGETHER_WITHIN) {
        *at = Some(Instant::now());
        UNWRITTEN.store(false, std::sync::atomic::Ordering::Release);
        journal.persist();
    } else {
        UNWRITTEN.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Write what waited to be written (`written`). Called at the end of
/// each pass of the window manager, and before it ends.
pub fn flush() {
    if UNWRITTEN.swap(false, std::sync::atomic::Ordering::AcqRel) {
        *WRITTEN_AT.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
        lock().persist();
    }
}

static JOURNAL: Mutex<Journal> = Mutex::new(Journal {
    entries: BTreeMap::new(),
    path: None,
});

fn lock() -> MutexGuard<'static, Journal> {
    JOURNAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lock without risking a deadlock when called from a panic hook or a
/// console-control thread: the lock is only ever held for a short file write,
/// so retry briefly and give up instead of blocking forever.
fn lock_for_emergency() -> Option<MutexGuard<'static, Journal>> {
    for _ in 0..50 {
        match JOURNAL.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    None
}

fn write_file(path: &Path, entries: &BTreeMap<isize, JournalEntry>) -> std::io::Result<()> {
    if entries.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = JournalFile {
        windows: entries.clone(),
    };
    let json = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    // Write-then-rename so a crash mid-write never leaves a truncated journal.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

fn read_file(path: &Path) -> BTreeMap<isize, JournalEntry> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    match serde_json::from_slice::<JournalFile>(&bytes) {
        Ok(file) => file.windows,
        Err(e) => {
            log::warn!(
                "Ignoring unreadable hidden-window journal {}: {e}",
                path.display()
            );
            BTreeMap::new()
        }
    }
}

/// Default journal location: `%LOCALAPPDATA%\gevrek\hidden.json`. Another
/// instance than the main one keeps its own, in its state folder: one
/// journal for two gevreks would have each "recover" (show) the windows
/// the other has hidden.
pub fn default_path() -> Option<PathBuf> {
    if gevrek_model::paths::instance() == gevrek_model::paths::MAIN_INSTANCE {
        dirs::data_local_dir().map(|dir| dir.join("gevrek").join("hidden.json"))
    } else {
        Some(gevrek_model::paths::state_dir().join("hidden.json"))
    }
}

fn identity(hwnd: HWND) -> (u32, String) {
    let mut pid = 0u32;
    let mut class = [0u16; 256];
    // SAFETY: both calls only write into the local buffers passed in.
    let len = unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        GetClassNameW(hwnd, &mut class)
    };
    let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
    (pid, class)
}

/// Make a window visible again: uncloak it if we cloaked it, and show it
/// without activation if it is hidden. A window that is already visible is
/// left alone, so a minimized window is not restored by accident.
fn unhide(hwnd: HWND, cloaked: bool) {
    // SAFETY: plain Win32 calls on a window handle; they fail harmlessly on a
    // handle that is no longer valid.
    unsafe {
        if cloaked {
            let value: i32 = 0;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CLOAK,
                &value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            );
        }
        // Posted, never sent: an emergency restore runs on crash, exit, or
        // session end, and a synchronous show would block forever on a window
        // whose app is hung. The app applies it as soon as it responds.
        // SW_SHOWNA shows the window in its current state.
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_SHOWNA);
        }
    }
}

/// Record that Gevrek hid (`cloaked == false`) or cloaked a window.
pub fn record_hidden(handle: WindowHandle, cloaked: bool) {
    stamp_own_change(handle);
    let hwnd = handle.to_hwnd();
    // SAFETY: IsWindow accepts any handle value.
    if !unsafe { IsWindow(hwnd) }.as_bool() {
        return;
    }
    let (pid, class) = identity(hwnd);
    let mut journal = lock();
    // Keep the cloak flag sticky: a window that was cloaked and then also
    // SW_HIDE'd still needs an uncloak on restore.
    let cloaked = cloaked || journal.entries.get(&handle.0).is_some_and(|e| e.cloaked);
    if journal.insert(
        handle.0,
        JournalEntry {
            pid,
            class,
            cloaked,
        },
    ) {
        written(&journal);
    }
}

/// Forget a window (it was shown again, or destroyed). Returns the entry so
/// the caller can tell whether the window still needs an uncloak.
pub fn record_shown(handle: WindowHandle) -> Option<JournalEntry> {
    stamp_own_change(handle);
    let mut journal = lock();
    let removed = journal.remove(handle.0);
    if removed.is_some() {
        written(&journal);
    }
    removed
}

/// Whether Gevrek currently has this window hidden or cloaked.
pub fn is_hidden(handle: WindowHandle) -> bool {
    lock().entries.contains_key(&handle.0)
}

/// How long a hide/show event can lag behind the WM's own request. Hide and
/// show are posted (`ShowWindowAsync`), so the event of a hide can arrive
/// after a following show has already cleared the journal entry.
const OWN_CHANGE_GRACE: Duration = Duration::from_secs(1);

/// When the WM last hid or showed each window.
static RECENT_OWN_CHANGES: Mutex<BTreeMap<isize, Instant>> = Mutex::new(BTreeMap::new());

fn stamp_own_change(handle: WindowHandle) {
    let now = Instant::now();
    let mut recent = RECENT_OWN_CHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    recent.retain(|_, at| now.duration_since(*at) < OWN_CHANGE_GRACE);
    recent.insert(handle.0, now);
}

/// True when a hide event for `handle` is (or may be) the WM's own doing:
/// the window is journaled hidden, or the WM changed its visibility within
/// the last second. Hides that are not are the app's own (minimize to tray).
pub fn visibility_changed_by_wm(handle: WindowHandle) -> bool {
    if is_hidden(handle) {
        return true;
    }
    RECENT_OWN_CHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&handle.0)
        .is_some_and(|at| at.elapsed() < OWN_CHANGE_GRACE)
}

/// Every window Gevrek currently has hidden or cloaked.
pub fn hidden_handles() -> Vec<WindowHandle> {
    lock().entries.keys().map(|&h| WindowHandle(h)).collect()
}

/// Unhide every journaled window and clear the journal. Safe to call from any
/// thread, including a panic hook or a console control handler. Returns the
/// number of windows that were still alive and got unhidden.
pub fn emergency_restore() -> usize {
    let Some(mut journal) = lock_for_emergency() else {
        log::error!("Hidden-window journal is busy; could not restore hidden windows");
        return 0;
    };
    let entries = std::mem::take(&mut journal.entries);
    let mut restored = 0;
    for (handle, entry) in &entries {
        let hwnd = WindowHandle(*handle).to_hwnd();
        // SAFETY: IsWindow accepts any handle value.
        if unsafe { IsWindow(hwnd) }.as_bool() {
            unhide(hwnd, entry.cloaked);
            restored += 1;
        }
    }
    journal.persist();
    restored
}

/// Decide which entries from a previous run still refer to the same window.
fn live_orphans(
    entries: &BTreeMap<isize, JournalEntry>,
    probe: impl Fn(isize) -> Option<(u32, String)>,
) -> Vec<(isize, bool)> {
    entries
        .iter()
        .filter(|(handle, entry)| {
            probe(**handle).is_some_and(|(pid, class)| pid == entry.pid && class == entry.class)
        })
        .map(|(handle, entry)| (*handle, entry.cloaked))
        .collect()
}

/// Unhide windows a previous instance left hidden (crash, kill, power loss),
/// then enable persistence at `path` for this run. Call once at startup,
/// before the runtime enumerates windows, so recovered windows get adopted.
/// Returns the number of windows recovered.
pub fn recover_orphans(path: PathBuf) -> usize {
    let previous = read_file(&path);
    let orphans = live_orphans(&previous, |handle| {
        let hwnd = WindowHandle(handle).to_hwnd();
        // SAFETY: IsWindow accepts any handle value.
        unsafe { IsWindow(hwnd) }.as_bool().then(|| identity(hwnd))
    });
    for (handle, cloaked) in &orphans {
        unhide(WindowHandle(*handle).to_hwnd(), *cloaked);
    }
    if orphans.len() < previous.len() {
        log::info!(
            "Dropped {} stale hidden-window journal entries (window gone or handle reused)",
            previous.len() - orphans.len()
        );
    }

    let mut journal = lock();
    journal.path = Some(path);
    // Anything recorded before persistence was enabled is still pending.
    journal.persist();
    orphans.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The journal is a file anyone may have written, and it is read
    /// before anything else at the start: cut short or with nonsense
    /// where a value belongs, it is taken for what can be read of it, or
    /// for none.
    #[test]
    fn no_journal_from_outside_ends_the_program() {
        use crate::spoil::{harder, lived, places, spoil, Chance};
        let dir =
            std::env::temp_dir().join(format!("gevrek-journal-outside-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hidden.json");
        let mut entries = BTreeMap::new();
        entries.insert(42, entry(7, "Chrome_WidgetWin_1", true));
        entries.insert(-5, entry(8, "Notepad", false));
        entries.insert(isize::MAX, entry(u32::MAX, "", false));
        write_file(&path, &entries).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let whole: serde_json::Value = serde_json::from_str(&text).unwrap();

        // Read as at the start, and asked which of its windows are there
        // still (every one is said to be: none is touched).
        let taken = |text: &str, path: &Path| {
            std::fs::write(path, text).unwrap();
            let read = read_file(path);
            let _ = live_orphans(&read, |_| Some((7, "Chrome_WidgetWin_1".to_string())));
        };

        let (times, seed) = harder();
        let mut chance = Chance(0x1f83_d9ab_fb41_bd6b ^ seed.rotate_left(7) | 1);
        let mut ended: Vec<String> = Vec::new();
        for end in 0..text.len() {
            let (part, path) = (text[..end].to_string(), path.clone());
            if let Err(panic) = lived(move || taken(&part, &path)) {
                ended.push(format!("cut at {end}: {panic}"));
            }
        }
        let mut all = Vec::new();
        places(&whole, &mut Vec::new(), &mut all);
        for _ in 0..300 * times {
            let mut document = whole.clone();
            let done = spoil(&mut document, &all, &mut chance);
            let (spoilt, path) = (document.to_string(), path.clone());
            if let Err(panic) = lived(move || taken(&spoilt, &path)) {
                ended.push(format!("with {done}: {panic}"));
            }
        }
        let _ = std::fs::remove_dir_all(dir);
        assert!(ended.is_empty(), "{}", ended.join("
"));
    }

    #[test]
    fn restoring_a_window_of_an_unresponsive_thread_does_not_block() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        };
        // The window's thread never pumps messages while we restore it, like
        // a hung app. A sent show would wait until that thread exits.
        let (sender, receiver) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    windows::core::w!("STATIC"),
                    windows::core::w!("gevrek unresponsive window"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("create window");
            sender.send(hwnd.0 as isize).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(3));
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        });
        let hwnd = HWND(receiver.recv().unwrap() as _);

        let started = std::time::Instant::now();
        unhide(hwnd, false);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "restore blocked on an unresponsive window for {:?}",
            started.elapsed()
        );
        owner.join().unwrap();
    }

    #[test]
    fn a_late_hide_event_after_the_wm_showed_the_window_is_still_its_own() {
        // Hide then show in quick succession: the journal entry is gone by
        // the time the (posted) hide's event arrives.
        let handle = WindowHandle(0x7E57_0001);
        assert!(!visibility_changed_by_wm(handle));
        record_shown(handle);
        assert!(!is_hidden(handle));
        assert!(visibility_changed_by_wm(handle));
    }

    fn entry(pid: u32, class: &str, cloaked: bool) -> JournalEntry {
        JournalEntry {
            pid,
            class: class.to_string(),
            cloaked,
        }
    }

    #[test]
    fn insert_reports_change_only_when_entry_differs() {
        let mut journal = Journal::default();
        assert!(journal.insert(1, entry(10, "Notepad", false)));
        assert!(!journal.insert(1, entry(10, "Notepad", false)));
        assert!(journal.insert(1, entry(10, "Notepad", true)));
        assert_eq!(journal.remove(1), Some(entry(10, "Notepad", true)));
        assert_eq!(journal.remove(1), None);
    }

    #[test]
    fn file_round_trips_and_empty_journal_removes_file() {
        let dir = std::env::temp_dir().join(format!("gevrek-journal-test-{}", std::process::id()));
        let path = dir.join("hidden.json");
        let mut entries = BTreeMap::new();
        entries.insert(42, entry(7, "Chrome_WidgetWin_1", true));
        entries.insert(-5, entry(8, "Notepad", false));

        write_file(&path, &entries).unwrap();
        assert_eq!(read_file(&path), entries);

        write_file(&path, &BTreeMap::new()).unwrap();
        assert!(!path.exists());
        assert!(read_file(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("gevrek-journal-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hidden.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(read_file(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn orphans_require_matching_pid_and_class() {
        let mut entries = BTreeMap::new();
        entries.insert(1, entry(10, "Notepad", false)); // same window
        entries.insert(2, entry(11, "Notepad", true)); // handle reused by another pid
        entries.insert(3, entry(12, "Notepad", false)); // handle reused by another class
        entries.insert(4, entry(13, "Notepad", false)); // window gone

        let orphans = live_orphans(&entries, |handle| match handle {
            1 => Some((10, "Notepad".to_string())),
            2 => Some((99, "Notepad".to_string())),
            3 => Some((12, "TrayWnd".to_string())),
            _ => None,
        });
        assert_eq!(orphans, vec![(1, false)]);
    }
}
