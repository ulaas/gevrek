//! What smart zones look through that is no plain folder, as far as both
//! systems do it alike: which sources are asked for, the launchers, and
//! the programs that run. Where the files opened lately and the
//! downloads come from is each system's own; they are handed in as
//! lists ([`crate::zones::sources::Source`]).
//!
//! | Source | What is told | Kept current by |
//! |---|---|---|
//! | `Recent` | the files opened last, the newest first | the system's record of them, watched |
//! | `Downloads` | the files of the downloads folder, the newest first | that folder, watched |
//! | `Pinned` | the pinned programs, then the most started | gevrek's own files of pins and counts, and the folders programs are installed into, watched |
//! | `RunningApps` | the programs that have windows | the window manager's state, whenever the shell hands it over |
//!
//! Each is told whole (`BackendEvent::SourceListed`) when it is first
//! asked for and whenever it is other than it was; none is read on the
//! caller's thread, and none by the clock.

use super::programs::{self, Kept, Known};
use super::sources::{Source, Tell};
use crate::apps::App;
use gevrek_zones::backend::{BackendError, BackendEvent, WatchId};
use gevrek_zones::config::SmartSource;
use gevrek_zones::ids::ItemId;
use gevrek_zones::item::Item;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How many files of the downloads folder are told at most (the newest).
pub const MOST_DOWNLOADS: usize = 1000;

/// Reads the installed programs. It may take a moment; it is called on a
/// list's own thread.
pub type Installed = Arc<dyn Fn() -> Vec<App> + Send + Sync>;

/// Sends a command to the window manager (the shell's line to it).
pub type Commander = Box<dyn FnMut(serde_json::Value)>;

/// The files of a folder, the newest first (by the later of made and
/// written, which for a download is when it arrived), no more than
/// [`MOST_DOWNLOADS`].
pub fn newest_first(mut items: Vec<Item>) -> Vec<Item> {
    let arrived = |item: &Item| item.facts.created.max(item.facts.modified);
    items.sort_by(|one, other| arrived(other).cmp(&arrived(one)).then_with(|| one.id.cmp(&other.id)));
    items.truncate(MOST_DOWNLOADS);
    items
}

/// The event a source is told with, when somebody asked for the source.
pub fn told_as(source: SmartSource, wanted: &Arc<AtomicBool>) -> impl Fn(Vec<Item>) -> Vec<BackendEvent> + Send + 'static {
    let wanted = Arc::clone(wanted);
    move |items| {
        if wanted.load(Ordering::SeqCst) {
            vec![BackendEvent::SourceListed { source: source.clone(), items }]
        } else {
            Vec::new()
        }
    }
}

pub struct Smart {
    tell: Tell,
    kept: Kept,
    /// The installed programs and the ones that run, shared with the
    /// thread that makes pictures.
    pub known: Arc<Mutex<Known>>,
    installed: Installed,
    /// The folders programs are installed into (their shortcuts or
    /// entries): a change there is a change of the launchers.
    program_folders: Vec<PathBuf>,
    /// The files opened lately: the system's own list, made by it.
    pub recent: Option<Source>,
    pub recent_wanted: Arc<AtomicBool>,
    /// The downloads: likewise.
    pub downloads: Option<Source>,
    pub downloads_wanted: Arc<AtomicBool>,
    launchers: Option<Source>,
    launchers_wanted: Arc<AtomicBool>,
    running_wanted: Arc<AtomicBool>,
    watched: BTreeMap<WatchId, SmartSource>,
    /// The window manager's state, as it was last handed over.
    state: Arc<Mutex<Option<serde_json::Value>>>,
    commander: Option<Commander>,
}

fn locked<T>(shared: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Work out the programs that run from the state and what is installed;
/// the answer is the list when it is other than it was.
fn rerun(known: &Mutex<Known>, state: &Mutex<Option<serde_json::Value>>) -> Option<Vec<Item>> {
    let state = locked(state);
    let mut known = locked(known);
    let running = state.as_ref().map(|state| programs::running(state, &known.apps)).unwrap_or_default();
    if running == known.running {
        return None;
    }
    known.running = running;
    Some(known.running.iter().map(|running| running.item.clone()).collect())
}

impl Smart {
    /// `state_dir` is gevrek's folder for what it keeps; `installed`
    /// reads the programs; `program_folders` are watched for programs
    /// that come and go; `tell` hands events to the backend.
    pub fn new(state_dir: &std::path::Path, installed: Installed, program_folders: Vec<PathBuf>, tell: Tell) -> Self {
        Self {
            tell,
            kept: Kept::under(state_dir),
            known: Arc::new(Mutex::new(Known::default())),
            installed,
            program_folders,
            recent: None,
            recent_wanted: Arc::new(AtomicBool::new(false)),
            downloads: None,
            downloads_wanted: Arc::new(AtomicBool::new(false)),
            launchers: None,
            launchers_wanted: Arc::new(AtomicBool::new(false)),
            running_wanted: Arc::new(AtomicBool::new(false)),
            watched: BTreeMap::new(),
            state: Arc::new(Mutex::new(None)),
            commander: None,
        }
    }

    /// Read the programs another way (a test's own list).
    pub fn set_installed(&mut self, installed: Installed) {
        self.installed = installed;
        self.launchers = None;
    }

    /// How a command reaches the window manager from now on.
    pub fn set_commander(&mut self, commander: Commander) {
        self.commander = Some(commander);
    }

    /// The list of launchers, made when it is first needed. It also
    /// keeps the installed programs known, by which the programs that
    /// run are named.
    fn launchers(&mut self) -> &Source {
        if self.launchers.is_none() {
            let (kept, known, installed) = (self.kept.clone(), Arc::clone(&self.known), Arc::clone(&self.installed));
            let (state, running_wanted, tell) = (Arc::clone(&self.state), Arc::clone(&self.running_wanted), Arc::clone(&self.tell));
            let read = move || {
                let apps = installed();
                let items = programs::launchers(&apps, &kept.pins(), &kept.counts(), &kept.recent(), programs::MOST_STARTED);
                locked(&known).apps = apps;
                // With other programs installed, a program that runs may
                // have another name.
                if let Some(items) = rerun(&known, &state) {
                    if running_wanted.load(Ordering::SeqCst) {
                        tell(vec![BackendEvent::SourceListed { source: SmartSource::RunningApps, items }]);
                    }
                }
                items
            };
            let mut watched = self.kept.folders();
            watched.extend(self.program_folders.iter().cloned());
            let (kept, folders) = (self.kept.clone(), self.program_folders.clone());
            let concerns = move |path: &std::path::Path| {
                kept.concerns(path) || folders.iter().any(|folder| path.starts_with(folder))
            };
            self.launchers = Some(Source::new(
                told_as(SmartSource::Pinned, &self.launchers_wanted),
                read,
                &watched,
                concerns,
                Arc::clone(&self.tell),
            ));
        }
        self.launchers.as_ref().expect("made above")
    }

    fn wanted(&self, source: &SmartSource) -> Option<&Arc<AtomicBool>> {
        match source {
            SmartSource::Recent => Some(&self.recent_wanted),
            SmartSource::Downloads => Some(&self.downloads_wanted),
            SmartSource::Pinned => Some(&self.launchers_wanted),
            SmartSource::RunningApps => Some(&self.running_wanted),
            SmartSource::Desktop | SmartSource::Folder { .. } => None,
        }
    }

    /// Tell of a source from now on, and tell what it has now. (The
    /// system's own lists, `recent` and `downloads`, are made by the
    /// system's half before this is called.)
    pub fn watch(&mut self, watch: WatchId, source: &SmartSource) -> Result<(), BackendError> {
        let wanted = self.wanted(source).ok_or(BackendError::Unsupported)?;
        wanted.store(true, Ordering::SeqCst);
        self.watched.insert(watch, source.clone());
        self.tell_again(source);
        Ok(())
    }

    /// Read a source again and tell what it has, changed or not.
    pub fn tell_again(&mut self, source: &SmartSource) {
        match source {
            SmartSource::Recent => self.recent.iter().for_each(Source::tell_again),
            SmartSource::Downloads => self.downloads.iter().for_each(Source::tell_again),
            SmartSource::Pinned => self.launchers().tell_again(),
            SmartSource::RunningApps => {
                // (The installed programs are read for the names.)
                self.launchers();
                let items = self.running();
                (self.tell)(vec![BackendEvent::SourceListed { source: SmartSource::RunningApps, items }]);
            }
            SmartSource::Desktop | SmartSource::Folder { .. } => {}
        }
    }

    /// End a watch. The answer is its source when nobody watches that
    /// any more (and whether the watch was one of these at all).
    pub fn unwatch(&mut self, watch: WatchId) -> Option<Option<SmartSource>> {
        let source = self.watched.remove(&watch)?;
        if self.watched.values().any(|other| other == &source) {
            return Some(None);
        }
        if let Some(wanted) = self.wanted(&source) {
            wanted.store(false, Ordering::SeqCst);
        }
        let programs_wanted = self.launchers_wanted.load(Ordering::SeqCst) || self.running_wanted.load(Ordering::SeqCst);
        if !programs_wanted {
            // (Its thread and its watches end with it.)
            self.launchers = None;
        }
        Some(Some(source))
    }

    /// What a source has, as far as it is known now: nothing before it
    /// was first read. It is also read again (on its own thread), and
    /// told if that finds it other than this answer.
    pub fn list(&mut self, source: &SmartSource) -> Result<Vec<Item>, BackendError> {
        let known = |source: &Option<Source>| {
            source.iter().for_each(Source::refresh);
            source.as_ref().map(Source::items).unwrap_or_default()
        };
        match source {
            SmartSource::Recent => Ok(known(&self.recent)),
            SmartSource::Downloads => Ok(known(&self.downloads)),
            SmartSource::Pinned => {
                let launchers = self.launchers();
                launchers.refresh();
                Ok(launchers.items())
            }
            SmartSource::RunningApps => Ok(self.running()),
            SmartSource::Desktop | SmartSource::Folder { .. } => Err(BackendError::Unsupported),
        }
    }

    /// The programs that run, as last worked out.
    pub fn running(&self) -> Vec<Item> {
        locked(&self.known).running.iter().map(|running| running.item.clone()).collect()
    }

    /// The window manager's state is this now (the JSON the shell has
    /// from it). The programs that run are worked out of it, and told
    /// where they are others than before and somebody asked for them.
    pub fn set_wm_state(&mut self, state: &serde_json::Value) {
        *locked(&self.state) = Some(state.clone());
        if let Some(items) = rerun(&self.known, &self.state) {
            if self.running_wanted.load(Ordering::SeqCst) {
                (self.tell)(vec![BackendEvent::SourceListed { source: SmartSource::RunningApps, items }]);
            }
        }
    }

    /// The pinned programs, by their ids, in the person's order.
    pub fn pins(&self) -> Vec<String> {
        self.kept.pins()
    }

    /// Pin a program or take its pin away. Written down at once; the
    /// launchers are told anew.
    pub fn pin(&mut self, app: &str, pin: bool) -> Result<(), BackendError> {
        let pins = programs::pinned(self.kept.pins(), app, pin);
        self.set_pins(&pins)
    }

    /// The pinned programs are these, in this order.
    pub fn set_pins(&mut self, pins: &[String]) -> Result<(), BackendError> {
        self.kept
            .set_pins(pins)
            .map_err(|error| BackendError::Refused { what: format!("{}: {error}", self.kept.pins.display()) })?;
        if let Some(launchers) = &self.launchers {
            launchers.refresh();
        }
        Ok(())
    }

    /// Every installed program as an item, by name (to pin one from).
    /// It reads the programs: not for the thread that draws.
    pub fn installed_items(&self) -> Vec<Item> {
        let apps = (self.installed)();
        let items = programs::installed_items(&apps);
        locked(&self.known).apps = apps;
        items
    }

    /// Whether an item is a program (and so no file of any folder).
    pub fn is_program(item: &ItemId) -> bool {
        item.as_str().starts_with(programs::APP) || item.as_str().starts_with(programs::RUNNING)
    }

    /// The item of a program as it is known now.
    pub fn item(&self, item: &ItemId) -> Option<Item> {
        let known = locked(&self.known);
        if let Some(app) = known.app(item) {
            return Some(programs::launcher_item(app));
        }
        known.running.iter().find(|running| &running.item.id == item).map(|running| running.item.clone())
    }

    /// The installed program an item names.
    pub fn app(&self, item: &ItemId) -> Option<App> {
        locked(&self.known).app(item).cloned()
    }

    /// Give the keyboard to a program that runs: its window that had it
    /// last. The command goes to the window manager by the shell's line.
    pub fn focus(&mut self, item: &ItemId) -> Result<(), BackendError> {
        let window = locked(&self.known)
            .window_of(item)
            .ok_or_else(|| BackendError::NotFound { what: item.to_string() })?;
        let commander = self.commander.as_mut().ok_or(BackendError::Unsupported)?;
        commander(programs::focus_command(window));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Duration;

    fn app(id: &str, name: &str) -> App {
        App {
            id: id.to_string(),
            name: name.to_string(),
            generic_name: String::new(),
            comment: String::new(),
            keywords: Vec::new(),
            icon: String::new(),
            command: vec![id.to_string()],
            terminal: false,
            dbus_activatable: false,
            directory: None,
            entry: PathBuf::new(),
            actions: Vec::new(),
        }
    }

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("gevrek-zones-smart-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::canonicalize(&folder).expect("a real folder")
    }

    /// Wait until `source` is told with exactly these names. (A list
    /// may be told twice as it was: once for being asked for, once for
    /// its first reading.)
    fn expect(tellings: &mpsc::Receiver<Vec<BackendEvent>>, source: &SmartSource, names: &[&str]) {
        let mut last = None;
        loop {
            let Ok(events) = tellings.recv_timeout(Duration::from_secs(20)) else {
                panic!("{source:?} was not told as {names:?}; last as {last:?}");
            };
            for event in events {
                if let BackendEvent::SourceListed { source: of, items } = event {
                    if &of == source {
                        let told: Vec<String> = items.into_iter().map(|item| item.name).collect();
                        if told == names {
                            return;
                        }
                        last = Some(told);
                    }
                }
            }
        }
    }

    /// Nothing is told of `source` for a moment.
    fn quiet(tellings: &mpsc::Receiver<Vec<BackendEvent>>, source: &SmartSource) {
        while let Ok(events) = tellings.recv_timeout(Duration::from_millis(200)) {
            for event in events {
                assert!(
                    !matches!(&event, BackendEvent::SourceListed { source: of, .. } if of == source),
                    "told of {source:?} unasked: {event:?}"
                );
            }
        }
    }

    #[test]
    fn launchers_and_running_programs_are_told_when_asked_for_and_when_they_change() {
        let state_dir = folder("programs");
        let (told, tellings) = mpsc::channel();
        let told = Mutex::new(told);
        let tell: Tell = Arc::new(move |events| {
            let _ = told.lock().expect("the channel").send(events);
        });
        let installed: Installed = Arc::new(|| vec![app("mail", "Mail"), app("web", "Web"), app("edit", "Edit")]);
        let mut smart = Smart::new(&state_dir, installed, Vec::new(), tell);
        std::fs::write(state_dir.join("launcher-usage.json"), r#"{"web":2}"#).expect("written");

        // Nothing is told of a source nobody asked for.
        smart.set_wm_state(&json!({"monitors": [{"workspaces": [{"detailed_windows": [
            {"window_id": 4, "title": "Inbox", "app_id": "mail"}]}]}]}));
        quiet(&tellings, &SmartSource::RunningApps);
        assert_eq!(smart.watch(WatchId(1), &SmartSource::Desktop), Err(BackendError::Unsupported));
        assert_eq!(smart.watch(WatchId(1), &SmartSource::Folder { directory: "/x".to_string() }), Err(BackendError::Unsupported));

        // The launchers: nothing pinned yet, so the one that was started.
        smart.watch(WatchId(1), &SmartSource::Pinned).expect("watched");
        expect(&tellings, &SmartSource::Pinned, &["Web"]);
        smart.pin("edit", true).expect("pinned");
        expect(&tellings, &SmartSource::Pinned, &["Edit", "Web"]);
        assert_eq!(smart.pins(), ["edit"]);
        assert_eq!(smart.list(&SmartSource::Pinned).expect("listed").len(), 2);
        // The launcher counts another start (it writes its file anew).
        std::fs::write(state_dir.join("launcher-usage.json"), r#"{"web":2,"mail":5}"#).expect("written");
        expect(&tellings, &SmartSource::Pinned, &["Edit", "Mail", "Web"]);
        smart.pin("edit", false).expect("unpinned");
        expect(&tellings, &SmartSource::Pinned, &["Mail", "Web"]);

        // The programs that run: told at once when asked for, by the
        // installed program's name; then with every other state.
        smart.watch(WatchId(2), &SmartSource::RunningApps).expect("watched");
        expect(&tellings, &SmartSource::RunningApps, &["Mail"]);
        smart.set_wm_state(&json!({"monitors": [{"workspaces": [{"detailed_windows": [
            {"window_id": 4, "title": "Inbox", "app_id": "mail"},
            {"window_id": 5, "title": "a.txt", "app_id": "edit", "is_focused": true},
            {"window_id": 6, "title": "b.txt", "app_id": "edit"}]}]}]}));
        expect(&tellings, &SmartSource::RunningApps, &["Mail", "Edit"]);
        // The same state again is no news.
        let again = locked(&smart.state).clone().expect("a state");
        smart.set_wm_state(&again);
        quiet(&tellings, &SmartSource::RunningApps);

        // A program is an item with facts, and activating one that runs
        // is a command for the window manager.
        let running = ItemId::new("running:edit");
        assert!(Smart::is_program(&running));
        assert!(!Smart::is_program(&ItemId::new("/home/ada/a.txt")));
        assert_eq!(smart.item(&running).map(|item| item.name), Some("Edit".to_string()));
        assert_eq!(smart.item(&ItemId::new("app:web")).map(|item| item.name), Some("Web".to_string()));
        assert_eq!(smart.focus(&running), Err(BackendError::Unsupported));
        let (sent, commands) = mpsc::channel();
        smart.set_commander(Box::new(move |command| {
            let _ = sent.send(command);
        }));
        smart.focus(&running).expect("sent");
        assert_eq!(commands.try_recv().expect("a command"), programs::focus_command(5));
        assert!(matches!(smart.focus(&ItemId::new("running:nothing")), Err(BackendError::NotFound { .. })));

        // Let go of: nothing more is told.
        assert_eq!(smart.unwatch(WatchId(2)), Some(Some(SmartSource::RunningApps)));
        assert_eq!(smart.unwatch(WatchId(1)), Some(Some(SmartSource::Pinned)));
        assert_eq!(smart.unwatch(WatchId(9)), None);
        smart.set_wm_state(&json!({}));
        quiet(&tellings, &SmartSource::RunningApps);
        quiet(&tellings, &SmartSource::Pinned);
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn downloads_are_the_newest_first() {
        let file = |name: &str, created: Option<u64>, modified: Option<u64>| {
            let mut item = Item::file(name);
            item.facts.created = created;
            item.facts.modified = modified;
            item
        };
        let listed = newest_first(vec![
            file("old", Some(10), Some(10)),
            // Saved from an archive: written long ago, arrived today.
            file("arrived", Some(500), Some(3)),
            file("edited", Some(20), Some(400)),
            file("nothing known", None, None),
        ]);
        assert_eq!(listed.iter().map(|item| item.name.as_str()).collect::<Vec<_>>(), ["arrived", "edited", "old", "nothing known"]);
        let many: Vec<Item> = (0..MOST_DOWNLOADS as u64 + 50).map(|number| file(&format!("f{number}"), Some(number), None)).collect();
        let kept = newest_first(many);
        assert_eq!(kept.len(), MOST_DOWNLOADS);
        assert_eq!(kept[0].name, format!("f{}", MOST_DOWNLOADS + 49));
    }
}
