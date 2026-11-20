//! Programs as items: the ones that are installed, the ones the person
//! pinned or starts most, and the ones that run now.
//!
//! None of these is a file a zone could show by its folder, so each gets
//! an id of its own kind, which stays the same over restarts:
//!
//! - `app:<id>` for an installed program, by the id the launcher knows it
//!   by (its desktop entry's name on Linux, its place in the Start menu
//!   or its package on Windows).
//! - `running:<key>` for a program that has windows, by what its windows
//!   say they belong to (the program's file on Windows, the application
//!   id on Linux).
//!
//! # Where pins are kept
//!
//! Nothing in gevrek kept pins before: the launcher only counts how often
//! each program is started (`launcher-usage.json` in the state folder)
//! and which were started last (`launcher-recent.json`). Both are read
//! here as the launcher writes them. Pins are the zones' own:
//! `zones/pinned-launchers.json`, a list of program ids in the person's
//! order. A zone of launchers shows the pinned ones first, then the most
//! started ones that are not pinned.
//!
//! # Programs that run
//!
//! The window manager knows the windows; the shell has its state as JSON.
//! [`running`] reads that state as `displays_of_state` does for the
//! displays: one item for each program with windows, with the ids of
//! those windows (the one that has the keyboard first). Activating such
//! an item is a command for the window manager ([`focus_command`]),
//! which the shell sends over the line it already has.

use crate::apps::App;
use gevrek_zones::ids::ItemId;
use gevrek_zones::item::{Item, ItemFacts, ItemKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the id of an installed program begins with.
pub const APP: &str = "app:";
/// What the id of a program that runs begins with.
pub const RUNNING: &str = "running:";

/// How many of the most started programs follow the pinned ones.
pub const MOST_STARTED: usize = 12;

/// The item id of an installed program.
pub fn app_id(app: &str) -> ItemId {
    ItemId::new(format!("{APP}{app}"))
}

/// The program an item id names, where it names an installed one.
pub fn app_of_id(item: &ItemId) -> Option<&str> {
    item.as_str().strip_prefix(APP)
}

/// An installed program as an item: a launcher by its name, with what it
/// starts as the place it leads to.
pub fn launcher_item(app: &App) -> Item {
    Item {
        id: app_id(&app.id),
        kind: ItemKind::Launcher,
        name: app.name.clone(),
        path: None,
        facts: ItemFacts { shortcut_target: app.command.first().cloned(), ..ItemFacts::default() },
    }
}

/// Every installed program as an item, by name: what a person chooses
/// from when they pin one.
pub fn installed_items(apps: &[App]) -> Vec<Item> {
    let mut items: Vec<Item> = apps.iter().map(launcher_item).collect();
    items.sort_by_key(|item| item.name.to_lowercase());
    items
}

/// The files under the state folder that the launchers are made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// How often each program was started (the launcher's file).
    pub counts: PathBuf,
    /// Which were started last (the launcher's file).
    pub recent: PathBuf,
    /// The pinned programs (the zones' own file).
    pub pins: PathBuf,
}

impl Kept {
    pub fn under(state_dir: &Path) -> Self {
        Self {
            counts: state_dir.join("launcher-usage.json"),
            recent: state_dir.join("launcher-recent.json"),
            pins: state_dir.join("zones").join("pinned-launchers.json"),
        }
    }

    /// The folders to watch for a change of any of the three.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = [&self.counts, &self.pins]
            .into_iter()
            .filter_map(|file| file.parent().map(Path::to_path_buf))
            .collect();
        folders.dedup();
        folders
    }

    /// Whether a change of this file is a change of the launchers.
    pub fn concerns(&self, path: &Path) -> bool {
        let name = path.file_name();
        [&self.counts, &self.recent, &self.pins].into_iter().any(|file| file.file_name() == name)
    }

    /// The pinned programs, in the person's order. Nothing where the
    /// file is not there or cannot be read.
    pub fn pins(&self) -> Vec<String> {
        text(&self.pins).and_then(|text| understood(&self.pins, serde_json::from_str(&text))).unwrap_or_default()
    }

    /// Write the pinned programs down, whole or not at all.
    pub fn set_pins(&self, pins: &[String]) -> std::io::Result<()> {
        if let Some(folder) = self.pins.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let text = serde_json::to_string_pretty(pins).map_err(std::io::Error::other)?;
        let beside = self.pins.with_extension("json.new");
        std::fs::write(&beside, text)?;
        std::fs::rename(&beside, &self.pins)
    }

    /// How often each program was started.
    pub fn counts(&self) -> HashMap<String, u32> {
        text(&self.counts).and_then(|text| understood(&self.counts, serde_json::from_str(&text))).unwrap_or_default()
    }

    /// The programs started last, the newest first.
    pub fn recent(&self) -> Vec<String> {
        text(&self.recent).and_then(|text| understood(&self.recent, serde_json::from_str(&text))).unwrap_or_default()
    }
}

/// What a file of ours holds; nothing where it is not there, and (said
/// in the log) where it cannot be read.
fn text(file: &Path) -> Option<String> {
    match std::fs::read_to_string(file) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            log::debug!("zones: {} cannot be read: {error}", file.display());
            None
        }
    }
}

/// What was read of a file as JSON; nothing (said in the log) where it
/// is not understood.
fn understood<T>(file: &Path, parsed: serde_json::Result<T>) -> Option<T> {
    parsed.inspect_err(|error| log::debug!("zones: {} is not understood, and is taken as empty: {error}", file.display())).ok()
}

/// The name of a program's file without its folders and its ending, as
/// either system writes paths (the state may come from the other one in
/// a test).
fn stem_of(file: &str) -> Option<String> {
    let name = file.rsplit(['\\', '/']).next()?;
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    (!stem.is_empty()).then(|| stem.to_string())
}

/// The pins with one program added at the end or taken out. Pinned
/// twice, a program is still there once.
pub fn pinned(mut pins: Vec<String>, app: &str, pin: bool) -> Vec<String> {
    pins.retain(|known| known != app);
    if pin {
        pins.push(app.to_string());
    }
    pins
}

/// The launchers a zone shows: the pinned programs in the person's
/// order, then the `most` most started that are not pinned (the more
/// often the earlier; of two started equally often, the one started
/// later; then by name). A program that is not installed any more is
/// left out, and comes back with the program.
pub fn launchers(apps: &[App], pins: &[String], counts: &HashMap<String, u32>, recent: &[String], most: usize) -> Vec<Item> {
    let by_id: HashMap<&str, &App> = apps.iter().map(|app| (app.id.as_str(), app)).collect();
    let mut items: Vec<Item> = Vec::new();
    for pin in pins {
        if let Some(app) = by_id.get(pin.as_str()) {
            if !items.iter().any(|item| item.id == app_id(&app.id)) {
                items.push(launcher_item(app));
            }
        }
    }
    let lately = |id: &str| recent.iter().position(|known| known == id).unwrap_or(usize::MAX);
    let mut started: Vec<&App> = apps
        .iter()
        .filter(|app| counts.get(&app.id).copied().unwrap_or(0) > 0 && !pins.contains(&app.id))
        .collect();
    started.sort_by(|one, other| {
        let count = |app: &App| counts.get(&app.id).copied().unwrap_or(0);
        count(other)
            .cmp(&count(one))
            .then(lately(&one.id).cmp(&lately(&other.id)))
            .then(one.name.to_lowercase().cmp(&other.name.to_lowercase()))
    });
    items.extend(started.into_iter().take(most).map(launcher_item));
    items
}

/// A program that runs, as an item, with what the item does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub item: Item,
    /// Its windows, by the window manager's ids: the one that has the
    /// keyboard first, then as the state lists them.
    pub windows: Vec<u64>,
    /// What its picture is looked for by: the picture its window names
    /// itself, else the program's file (Windows) or what the program
    /// calls itself (Linux). Empty: none.
    pub icon: String,
}

/// The programs that have windows, as the window manager's state (the
/// JSON the shell has from it) lists them: one for each program, in the
/// order their first windows come in. Windows that are the window
/// manager's own (borders) are no programs.
///
/// A program is known by its file where the state names one (Windows),
/// else by what it calls itself (`app_id`, then `class`); a window that
/// says none of these is a program by itself, known by its window. The
/// name is the installed program's where one fits (`apps`), else the
/// file's or the window's.
pub fn running(state: &serde_json::Value, apps: &[App]) -> Vec<Running> {
    let mut found: Vec<Running> = Vec::new();
    let windows = state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|monitor| monitor["workspaces"].as_array().into_iter().flatten())
        .flat_map(|workspace| workspace["detailed_windows"].as_array().into_iter().flatten());
    for window in windows {
        if window["is_border_window"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(id) = window["window_id"].as_u64() else {
            continue;
        };
        let said = |key: &str| window[key].as_str().unwrap_or_default();
        let (exe, app, class, title) = (said("exe"), said("app_id"), said("class"), said("title"));
        let key = if !exe.is_empty() {
            exe.to_lowercase()
        } else if !app.is_empty() {
            app.to_string()
        } else if !class.is_empty() {
            class.to_string()
        } else {
            format!("window-{id}")
        };
        let item_id = ItemId::new(format!("{RUNNING}{key}"));
        let focused = window["is_focused"].as_bool().unwrap_or(false);
        if let Some(known) = found.iter_mut().find(|known| known.item.id == item_id) {
            if focused {
                known.windows.insert(0, id);
            } else {
                known.windows.push(id);
            }
            continue;
        }
        let file_stem = stem_of(exe);
        let installed = [app, class, file_stem.as_deref().unwrap_or_default()]
            .into_iter()
            .find_map(|name| crate::icons::app_of(apps, name));
        let name = installed
            .map(|app| app.name.clone())
            .or_else(|| file_stem.clone())
            .or_else(|| [app, class, title].into_iter().find(|text| !text.is_empty()).map(str::to_string))
            .unwrap_or_else(|| format!("Window {id}"));
        let own = said("icon");
        let icon = if !own.is_empty() {
            own
        } else if cfg!(windows) {
            exe
        } else if !app.is_empty() {
            app
        } else {
            class
        };
        found.push(Running {
            item: Item {
                id: item_id,
                kind: ItemKind::Launcher,
                name,
                path: None,
                facts: ItemFacts {
                    shortcut_target: (!exe.is_empty()).then(|| exe.to_string()),
                    ..ItemFacts::default()
                },
            },
            windows: vec![id],
            icon: icon.to_string(),
        });
    }
    found
}

/// The command that gives a window the keyboard, as the window manager
/// takes it over the shell's line.
pub fn focus_command(window: u64) -> serde_json::Value {
    serde_json::json!({"command": "focus_window", "selector": {"kind": "handle", "value": window}})
}

/// What is known of programs, shared with the thread that makes pictures
/// (which finds a program's picture by its item's id).
#[derive(Debug, Default)]
pub struct Known {
    /// The installed programs, as they were last read.
    pub apps: Vec<App>,
    /// The programs that run, as the window manager last said.
    pub running: Vec<Running>,
}

impl Known {
    /// What the picture of an item that is a program is looked for by:
    /// an icon's name, a file, or what the shell of Windows takes for
    /// one. None for an item that is no program.
    pub fn icon_of(&self, id: &str) -> Option<String> {
        if let Some(app) = id.strip_prefix(APP) {
            return self.apps.iter().find(|known| known.id == app).map(|app| app.icon.clone());
        }
        if id.starts_with(RUNNING) {
            return self.running.iter().find(|known| known.item.id.as_str() == id).map(|known| known.icon.clone());
        }
        None
    }

    /// The installed program an item names.
    pub fn app(&self, item: &ItemId) -> Option<&App> {
        let id = app_of_id(item)?;
        self.apps.iter().find(|app| app.id == id)
    }

    /// The window to give the keyboard to when an item of a program
    /// that runs is activated.
    pub fn window_of(&self, item: &ItemId) -> Option<u64> {
        self.running.iter().find(|known| &known.item.id == item)?.windows.first().copied()
    }
}

/// The picture of a program by what [`Known::icon_of`] said. It reads
/// files, and on Windows asks the shell: not for the thread that draws.
pub fn picture(icon: &str, pixels: u32) -> Option<crate::icons::Picture> {
    if icon.is_empty() {
        return None;
    }
    #[cfg(not(windows))]
    if icon.starts_with('/') {
        return crate::icons::load(Path::new(icon), pixels);
    }
    crate::icons::picture(icon, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app(id: &str, name: &str) -> App {
        App {
            id: id.to_string(),
            name: name.to_string(),
            generic_name: String::new(),
            comment: String::new(),
            keywords: Vec::new(),
            icon: format!("icon-of-{id}"),
            command: vec![format!("/usr/bin/{id}")],
            terminal: false,
            dbus_activatable: false,
            directory: None,
            entry: PathBuf::new(),
            actions: Vec::new(),
        }
    }

    fn ids(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.id.as_str()).collect()
    }

    #[test]
    fn a_program_is_a_launcher_with_an_id_that_stays() {
        let item = launcher_item(&app("org.editor", "Editor"));
        assert_eq!(item.id.as_str(), "app:org.editor");
        assert_eq!(item.kind, ItemKind::Launcher);
        assert_eq!(item.name, "Editor");
        assert_eq!(item.path, None);
        assert_eq!(item.facts.shortcut_target.as_deref(), Some("/usr/bin/org.editor"));
        assert_eq!(app_of_id(&item.id), Some("org.editor"));
        assert_eq!(app_of_id(&ItemId::new("/home/ada/a.txt")), None);
        let all = installed_items(&[app("b", "zebra"), app("a", "Apple")]);
        assert_eq!(ids(&all), ["app:a", "app:b"]);
    }

    #[test]
    fn pinned_programs_come_first_and_the_most_started_follow() {
        let apps = [app("mail", "Mail"), app("web", "Web"), app("edit", "Edit"), app("draw", "Draw"), app("calc", "Calc")];
        let counts: HashMap<String, u32> =
            [("web", 9), ("edit", 3), ("draw", 3), ("mail", 40), ("gone", 99)].map(|(id, count)| (id.to_string(), count)).into();
        let recent = vec!["draw".to_string(), "edit".to_string()];
        let pins = vec!["calc".to_string(), "gone".to_string(), "mail".to_string(), "calc".to_string()];
        // Pins in their order, once each, without what is not installed;
        // then by how often, the one started later first of two equals;
        // what was never started is not there.
        assert_eq!(
            ids(&launchers(&apps, &pins, &counts, &recent, 12)),
            ["app:calc", "app:mail", "app:web", "app:draw", "app:edit"]
        );
        assert_eq!(ids(&launchers(&apps, &pins, &counts, &recent, 1)), ["app:calc", "app:mail", "app:web"]);
        assert_eq!(ids(&launchers(&apps, &[], &HashMap::new(), &[], 12)), Vec::<&str>::new());

        assert_eq!(pinned(vec!["a".to_string()], "b", true), ["a", "b"]);
        assert_eq!(pinned(vec!["a".to_string(), "b".to_string()], "a", true), ["b", "a"]);
        assert_eq!(pinned(vec!["a".to_string(), "b".to_string()], "a", false), ["b"]);
    }

    #[test]
    fn pins_are_kept_in_the_zones_own_file_and_the_launchers_counts_are_read() {
        let root = std::env::temp_dir().join(format!("gevrek-zones-kept-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let kept = Kept::under(&root);
        assert_eq!(kept.pins(), Vec::<String>::new());
        assert!(kept.counts().is_empty());
        kept.set_pins(&["web".to_string(), "mail".to_string()]).expect("written");
        assert_eq!(kept.pins(), ["web", "mail"]);
        // As the launcher writes its two files.
        std::fs::write(&kept.counts, r#"{"web":3,"mail":1}"#).expect("written");
        std::fs::write(&kept.recent, r#"["mail","web"]"#).expect("written");
        assert_eq!(kept.counts().get("web"), Some(&3));
        assert_eq!(kept.recent(), ["mail", "web"]);
        // A file that is no list is no pins, and is written over.
        std::fs::write(&kept.pins, "{").expect("written");
        assert_eq!(kept.pins(), Vec::<String>::new());
        assert!(kept.concerns(&root.join("launcher-usage.json")));
        assert!(kept.concerns(&root.join("zones").join("pinned-launchers.json")));
        assert!(!kept.concerns(&root.join("zones").join("zones.json")));
        assert_eq!(kept.folders(), [root.clone(), root.join("zones")]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_window_managers_state_says_which_programs_run() {
        let state = json!({"monitors": [
            {"workspaces": [
                {"detailed_windows": [
                    {"window_id": 7, "title": "notes.txt", "class": "Notepad", "exe": "C:\\Windows\\notepad.exe"},
                    {"window_id": 8, "title": "", "is_border_window": true, "exe": "C:\\gevrek.exe"},
                    {"window_id": 9, "title": "Inbox", "app_id": "org.mail", "class": "mail"},
                ]},
                {"detailed_windows": [
                    {"window_id": 11, "title": "other.txt", "class": "Notepad", "exe": "c:\\windows\\NOTEPAD.EXE", "is_focused": true},
                    {"title": "no id"},
                ]},
            ]},
            {"workspaces": [{"detailed_windows": [
                {"window_id": 12, "title": "A picture", "icon": "/usr/share/icons/viewer.png", "class": "viewer"},
                {"window_id": 13},
            ]}]},
        ]});
        let found = running(&state, &[app("org.mail", "Mail")]);
        let seen: Vec<(&str, &str, &[u64])> =
            found.iter().map(|known| (known.item.id.as_str(), known.item.name.as_str(), &known.windows[..])).collect();
        assert_eq!(
            seen,
            [
                // Both windows of one program, the one with the keyboard
                // first; named by its file.
                ("running:c:\\windows\\notepad.exe", "notepad", &[11u64, 7][..]),
                // Named as the installed program is.
                ("running:org.mail", "Mail", &[9][..]),
                ("running:viewer", "viewer", &[12][..]),
                ("running:window-13", "Window 13", &[13][..]),
            ]
        );
        assert!(found.iter().all(|known| known.item.kind == ItemKind::Launcher && known.item.path.is_none()));
        assert_eq!(found[0].item.facts.shortcut_target.as_deref(), Some("C:\\Windows\\notepad.exe"));
        // The window's own picture before the program's.
        assert_eq!(found[2].icon, "/usr/share/icons/viewer.png");
        assert_eq!(found[1].icon, if cfg!(windows) { "" } else { "org.mail" });
        assert!(running(&json!({}), &[]).is_empty());

        let known = Known { apps: vec![app("org.mail", "Mail")], running: found };
        assert_eq!(known.window_of(&ItemId::new("running:c:\\windows\\notepad.exe")), Some(11));
        assert_eq!(known.window_of(&ItemId::new("running:nothing")), None);
        assert_eq!(known.icon_of("app:org.mail").as_deref(), Some("icon-of-org.mail"));
        assert_eq!(known.icon_of("running:viewer").as_deref(), Some("/usr/share/icons/viewer.png"));
        assert_eq!(known.icon_of("C:\\a.txt"), None);
        assert_eq!(known.app(&ItemId::new("app:org.mail")).map(|app| app.name.as_str()), Some("Mail"));
        assert_eq!(
            focus_command(11),
            json!({"command": "focus_window", "selector": {"kind": "handle", "value": 11}})
        );
    }
}
