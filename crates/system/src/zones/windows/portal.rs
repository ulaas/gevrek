//! A folder a portal shows: opened, read and watched off the caller's
//! thread, and told as away when it cannot be reached.
//!
//! Watching a folder means going to it three times: to see that it is
//! there, to have the file system tell of it, and to read what is in it.
//! Each of those can take a second for a folder of thousands of files and
//! half a minute for a machine that does not answer. So none of them is
//! done by the caller: [`Portal::open`] returns at once, and what is in
//! the folder is told when it is known (`BackendEvent::Listed`).
//!
//! # A folder that is not there, goes, or comes back
//!
//! - A folder that does not answer within the patience it is given
//!   ([`crate::zones::reach::PATIENCE`]) is told as away
//!   (`BackendEvent::ListingFailed`; nothing is known to be in it from
//!   then on). The thread that went to it goes on waiting, for as long as
//!   Windows does; when it answers after all, the folder is told whole.
//! - While a folder is watched, the folder above it is watched for the
//!   one thing: the watched folder's own name. So its being deleted,
//!   renamed away or replaced is noticed (the file system does not tell a
//!   folder of its own going in every case), and so is its coming back.
//! - While a folder is away, the nearest folder above it that is there is
//!   watched for the next name on the way down. A whole path that is made
//!   again is followed one step at a time.
//! - What no folder above can tell (a share on a machine that is switched
//!   on again, a drive that is plugged in) is tried again when somebody
//!   asks: [`Portal::retry`].
//!
//! Nothing here looks by the clock. Every thread sleeps in the system
//! until it is told, or ends.

use super::facts::same_path;
use super::seen::Seen;
use super::shell::{self, Types};
use super::watch::{self, Folder, News, Shared};
use crate::com::Com;
use crate::zones::reach::{Reach, PATIENCE};
use gevrek_zones::backend::{BackendError, BackendEvent, Location};
use notify::event::{EventKind, ModifyKind};
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::Duration;

/// How a folder is gone to. Other than the system's own in tests, which
/// play a machine that does not answer.
pub struct How {
    /// How long a folder may take before it is told as away.
    pub patience: Duration,
    /// Whether the folder at this path is there. This is the first going
    /// to it, and the one that takes long when a machine is gone.
    pub reach: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// When the person last opened each file, for the facts of what is
    /// read.
    pub opened: super::recent::Opened,
}

impl Default for How {
    fn default() -> Self {
        Self {
            patience: PATIENCE,
            reach: Box::new(|path| Path::new(path).is_dir()),
            opened: Default::default(),
        }
    }
}

struct State {
    /// The file system's telling of the folder, while it is there.
    folder: Option<Folder>,
    /// The telling of a folder above it: of the watched folder's own
    /// coming and going.
    sentinel: Option<notify::RecommendedWatcher>,
    reach: Reach,
    /// A thread is going to the folder now.
    opening: bool,
    /// Something happened meanwhile: go once more when it is back.
    again: bool,
}

struct Inner {
    /// The folder, as Windows writes it.
    path: String,
    seen: Shared,
    news: Arc<News>,
    how: Arc<How>,
    ended: AtomicBool,
    state: Mutex<State>,
}

/// A folder that is watched for a portal. The watching ends with this.
pub struct Portal {
    /// What is known to be in it. Empty until it was read.
    pub seen: Shared,
    inner: Arc<Inner>,
}

impl Drop for Portal {
    fn drop(&mut self) {
        self.inner.ended.store(true, Ordering::SeqCst);
        let mut state = self.inner.state();
        state.folder = None;
        state.sentinel = None;
    }
}

impl Portal {
    /// Begin to watch the folder `path`. Returns at once; what is in the
    /// folder is told to `news` when it was read.
    pub fn open(path: &str, news: Arc<News>, how: Arc<How>) -> Self {
        let location = Location::Directory { path: path.to_string() };
        let seen: Shared = Arc::new(Mutex::new(Seen::new(location)));
        let inner = Arc::new(Inner {
            path: shell::plain(path),
            seen: Arc::clone(&seen),
            news,
            how,
            ended: AtomicBool::new(false),
            state: Mutex::new(State {
                folder: None,
                sentinel: None,
                reach: Reach::Opening,
                opening: false,
                again: false,
            }),
        });
        attempt(&inner);
        Self { seen, inner }
    }

    /// How the folder stands now.
    pub fn reach(&self) -> Reach {
        self.inner.state().reach.clone()
    }

    /// The folder, as Windows writes it.
    pub fn path(&self) -> String {
        self.inner.path.clone()
    }

    /// Go to the folder and read it whole again, whether it is there or
    /// away: told as a listing, or as away. While it is being gone to
    /// anyway, that going is the answer.
    pub fn read_again(&self) {
        if self.inner.state().opening {
            return;
        }
        // (Away for the same reason as before is told again: somebody
        // asked.)
        {
            let mut state = self.inner.state();
            if state.reach.is_away() {
                state.reach = Reach::Opening;
            }
        }
        attempt(&self.inner);
    }

    /// Go to a folder that is away once more (somebody asked, or
    /// something happened that may have brought it back). Nothing is done
    /// for a folder that is there.
    pub fn retry(&self) {
        if self.inner.state().reach.is_away() {
            attempt(&self.inner);
        }
    }
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// The folder is away: nothing is in it, as far as anyone can see.
    fn away(&self, why: &str) {
        let location = {
            let mut seen = self.seen();
            seen.listed(&[]);
            seen.location().clone()
        };
        let mut state = self.state();
        state.folder = None;
        let reach = Reach::Away { why: why.to_string() };
        if state.reach == reach || self.ended() {
            return;
        }
        log::debug!("zones: {} is away: {why}", self.path);
        state.reach = reach.clone();
        self.news.tell(crate::zones::reach::told(&location, &reach));
    }

    /// Go to the folder: see that it is there, have the file system tell
    /// of it, read it and tell what is in it. The answer is the telling,
    /// to be kept.
    fn read(self: &Arc<Self>) -> Result<Folder, BackendError> {
        let gone = || BackendError::NotFound { what: self.path.clone() };
        if self.ended() || !(self.how.reach)(&self.path) {
            return Err(gone());
        }
        let weak = Arc::downgrade(self);
        let types = || Types::knowing(self.how.opened.clone());
        let folder = watch::watch_into(&self.path, Arc::clone(&self.seen), Arc::clone(&self.news), types(), move || {
            again(&weak);
        })?;
        // The telling begins before the reading, so nothing that happens
        // between the two is missed; and what changes while the reading
        // goes on is read once more at its end, since the listing may
        // have it either way.
        let location = {
            let mut seen = self.seen();
            seen.begin_listing();
            seen.location().clone()
        };
        let _com = Com::new();
        let mut types = types();
        let listed = shell::list(&location, &mut types);
        let mut seen = self.seen();
        let touched = seen.end_listing();
        let mut items = listed?;
        for id in touched {
            items.retain(|item| !same_path(item.id.as_str(), id.as_str()));
            items.extend(shell::item_if_listed(&id, &mut types));
        }
        if self.ended() {
            return Err(gone());
        }
        seen.listed(&items);
        log::debug!("zones: {} is there, with {} items", self.path, items.len());
        // (It is there from the moment it is told so, for whoever asks
        // when woken by the telling.)
        self.state().reach = Reach::There;
        // (Told while what is known is held: see `watch_into`.)
        self.news.tell(vec![BackendEvent::Listed { location, items }]);
        Ok(folder)
    }

    /// What became of a going to the folder.
    fn settle(self: &Arc<Self>, outcome: Result<Folder, BackendError>) {
        if self.ended() {
            return;
        }
        match outcome {
            Ok(folder) => {
                let mut state = self.state();
                state.folder = Some(folder);
                state.reach = Reach::There;
            }
            Err(error) => self.away(&error.to_string()),
        }
        let sentinel = self.sentinel();
        // What happened to the folder before the folder above it was
        // listened to went unheard: looked at once more now, it is
        // either as it was taken to be, or gone to again.
        let there = (self.how.reach)(&self.path);
        let mut state = self.state();
        state.sentinel = sentinel;
        if there == state.reach.is_away() {
            state.again = true;
        }
    }

    /// Watch the nearest folder above this one that is there, for the
    /// next name on the way down to it.
    fn sentinel(self: &Arc<Self>) -> Option<notify::RecommendedWatcher> {
        let target = PathBuf::from(&self.path);
        let mut next = target.as_path();
        let anchor = loop {
            let above = next.parent()?;
            if (self.how.reach)(&above.to_string_lossy()) {
                break above.to_path_buf();
            }
            next = above;
        };
        let name = next.to_string_lossy().into_owned();
        let weak = Arc::downgrade(self);
        let handler = move |event: notify::Result<notify::Event>| {
            let ours = match event {
                // (The folder above went itself: look for the next one.)
                Err(_) => true,
                Ok(event) => {
                    // A folder is told as changed whenever something in
                    // it is; only its own coming, going and renaming
                    // count here.
                    matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)))
                        && event.paths.iter().any(|path| same_path(&path.to_string_lossy(), &name))
                }
            };
            if ours {
                again(&weak);
            }
        };
        let watching = notify::recommended_watcher(handler).and_then(|mut watcher| {
            watcher.watch(&anchor, RecursiveMode::NonRecursive)?;
            Ok(watcher)
        });
        watching
            .inspect_err(|error| log::debug!("zones: {} is not listened to for a folder in it to come back: {error}", anchor.display()))
            .ok()
    }
}

/// Go to the folder again, if the portal is still there.
fn again(inner: &Weak<Inner>) {
    if let Some(inner) = inner.upgrade() {
        attempt(&inner);
    }
}

/// Go to the folder, on a thread of its own. One going at a time: asked
/// for while one is under way, another follows it.
fn attempt(inner: &Arc<Inner>) {
    {
        let mut state = inner.state();
        if inner.ended() {
            return;
        }
        if state.opening {
            state.again = true;
            return;
        }
        state.opening = true;
    }
    let going = Arc::clone(inner);
    let started = std::thread::Builder::new().name("gevrek-zones-portal".to_string()).spawn(move || loop {
        let (answer, answered) = mpsc::channel();
        let reader = Arc::clone(&going);
        // The going itself is on a second thread, so that this one can
        // say "it does not answer" while that one still waits.
        let read = std::thread::Builder::new()
            .name("gevrek-zones-portal-read".to_string())
            .spawn(move || {
                let _ = answer.send(reader.read());
            });
        let no_answer = || BackendError::Refused { what: format!("{}: no answer", going.path) };
        let outcome = match read {
            Err(_) => Err(no_answer()),
            Ok(_) => match answered.recv_timeout(going.how.patience) {
                Ok(outcome) => outcome,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    going.away(&format!("{} does not answer", going.path));
                    answered.recv().unwrap_or_else(|_| Err(no_answer()))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(no_answer()),
            },
        };
        going.settle(outcome);
        let mut state = going.state();
        if state.again && !going.ended() {
            state.again = false;
            continue;
        }
        state.opening = false;
        break;
    });
    if started.is_err() {
        inner.state().opening = false;
        inner.away("no thread to read it on");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_zones::ids::ItemId;
    use std::time::Instant;

    fn folder(name: &str) -> String {
        let folder = std::env::temp_dir().join(format!("gevrek-zones-portal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::canonicalize(&folder)
            .expect("the folder")
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .to_string()
    }

    fn news() -> (Arc<News>, mpsc::Receiver<()>) {
        let news = Arc::new(News::default());
        let (woken, wakes) = mpsc::channel();
        news.on_news(Box::new(move || {
            let _ = woken.send(());
        }));
        (news, wakes)
    }

    /// Wait until `wanted` holds of what was told so far (kept in `all`).
    fn until(news: &News, wakes: &mpsc::Receiver<()>, all: &mut Vec<BackendEvent>, wanted: impl Fn(&[BackendEvent]) -> bool) {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            all.extend(news.take());
            if wanted(all) {
                return;
            }
            let left = end.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "not told in time; told so far: {all:#?}");
            let _ = wakes.recv_timeout(left);
        }
    }

    /// The name of the file an item is. (Not the name it is shown with:
    /// the shell leaves the ending away where Explorer is set to hide the
    /// endings of known types, as it is on a new Windows.)
    fn file_of(item: &gevrek_zones::item::Item) -> &str {
        gevrek_zones::paths::file_name(item.id.as_str())
    }

    /// The files in the listing that was told last; nothing in it where
    /// the last word was that the folder is away.
    fn last_listing(all: &[BackendEvent]) -> Option<Vec<String>> {
        all.iter().rev().find_map(|event| match event {
            BackendEvent::Listed { items, .. } => Some(items.iter().map(|item| file_of(item).to_string()).collect()),
            BackendEvent::ListingFailed { .. } => Some(Vec::new()),
            _ => None,
        })
    }

    #[test]
    fn a_folder_that_does_not_answer_is_told_as_away_and_shown_when_it_does() {
        let root = folder("slow");
        std::fs::write(format!("{root}\\a.txt"), b"a").expect("written");
        let (news, wakes) = news();
        // A machine that takes its time: the going to it waits until the
        // test lets it through.
        let (let_through, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let how = Arc::new(How {
            patience: Duration::from_millis(100),
            reach: Box::new(move |path| {
                let _ = gate.lock().expect("the gate").recv();
                Path::new(path).is_dir()
            }),
            opened: Default::default(),
        });
        let asked = Instant::now();
        let portal = Portal::open(&root, Arc::clone(&news), how);
        // The caller is answered at once, whatever the folder does.
        assert!(asked.elapsed() < Duration::from_millis(50), "{:?}", asked.elapsed());
        assert_eq!(portal.reach(), Reach::Opening);
        let mut all = Vec::new();
        until(&news, &wakes, &mut all, |_| portal.reach().is_away());
        assert_eq!(last_listing(&all), Some(Vec::new()));
        assert!(matches!(portal.reach(), Reach::Away { why } if why.contains("does not answer")));
        // It answers after all (once for the folder, and again for each
        // folder above that is asked about).
        drop(let_through);
        until(&news, &wakes, &mut all, |all| last_listing(all) == Some(vec!["a.txt".to_string()]));
        until(&news, &wakes, &mut all, |_| portal.reach() == Reach::There);
        drop(portal);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_goes_and_comes_back_is_followed() {
        let root = folder("comes-back");
        let shown = format!("{root}\\deep\\shown");
        let (news, wakes) = news();
        // Not there yet: away, and nothing in it.
        let portal = Portal::open(&shown, Arc::clone(&news), Arc::new(How::default()));
        let mut all = Vec::new();
        until(&news, &wakes, &mut all, |_| portal.reach().is_away());
        assert_eq!(last_listing(&all), Some(Vec::new()));

        // Made one step at a time: followed down, and told whole when it
        // is there.
        std::fs::create_dir(format!("{root}\\deep")).expect("made");
        std::fs::create_dir(&shown).expect("made");
        until(&news, &wakes, &mut all, |_| portal.reach() == Reach::There);
        std::fs::write(format!("{shown}\\a.txt"), b"a").expect("written");
        until(&news, &wakes, &mut all, |all| {
            all.iter().any(|event| matches!(event, BackendEvent::ItemAdded { item, .. } if file_of(item) == "a.txt"))
                || last_listing(all) == Some(vec!["a.txt".to_string()])
        });

        // Renamed away: nothing is in it any more.
        all.clear();
        std::fs::rename(&shown, format!("{root}\\deep\\elsewhere")).expect("renamed");
        until(&news, &wakes, &mut all, |all| portal.reach().is_away() && last_listing(all) == Some(Vec::new()));

        // Another folder of that name, with other things in it.
        all.clear();
        std::fs::create_dir(&shown).expect("made");
        std::fs::write(format!("{shown}\\b.txt"), b"b").expect("written");
        until(&news, &wakes, &mut all, |all| {
            portal.reach() == Reach::There
                && (last_listing(all) == Some(vec!["b.txt".to_string()])
                    || all.iter().any(|event| matches!(event, BackendEvent::ItemAdded { item, .. } if file_of(item) == "b.txt")))
        });
        let known: Vec<ItemId> = portal.seen.lock().expect("seen").items().into_iter().map(|item| item.id).collect();
        assert_eq!(known, [ItemId::new(format!("{shown}\\b.txt"))]);

        // Deleted with everything in it.
        all.clear();
        std::fs::remove_dir_all(&shown).expect("removed");
        until(&news, &wakes, &mut all, |_| portal.reach().is_away());
        assert!(portal.seen.lock().expect("seen").items().is_empty());

        // Asked to try again while it is away: still away, and no harm.
        portal.retry();
        std::fs::create_dir(&shown).expect("made");
        until(&news, &wakes, &mut all, |_| portal.reach() == Reach::There);
        drop(portal);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A folder of five thousand files: the caller is not held, and the
    /// listing comes in one piece. How long it took is printed (and
    /// written into the notes); only a bound far above it is asserted,
    /// since a busy machine is slow for reasons that are not this code's.
    #[test]
    fn five_thousand_files_are_listed_without_holding_the_caller() {
        let root = folder("many");
        for number in 0..5000 {
            std::fs::write(format!("{root}\\file {number:04}.txt"), b"x").expect("written");
        }
        let (news, wakes) = news();
        let asked = Instant::now();
        let portal = Portal::open(&root, Arc::clone(&news), Arc::new(How { patience: Duration::from_secs(60), ..How::default() }));
        let held = asked.elapsed();
        let mut all = Vec::new();
        until(&news, &wakes, &mut all, |all| last_listing(all).is_some_and(|names| names.len() == 5000));
        let took = asked.elapsed();
        println!("5000 files: the caller was held {held:?}; listed after {took:?}");
        assert!(held < Duration::from_millis(100), "{held:?}");
        assert!(took < Duration::from_secs(60), "{took:?}");
        assert_eq!(portal.seen.lock().expect("seen").items().len(), 5000);
        drop(portal);
        let _ = std::fs::remove_dir_all(&root);
    }
}
