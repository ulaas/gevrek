//! The state on the disk, in a folder the caller names (`state/zones`).
//!
//! `zones.json` is written whole, so that it is either the old file or the
//! new one, and what it held before is kept in `backups/`, the newest
//! twenty. Reading never loses a layout: a file that cannot be read, or
//! that a newer gevrek wrote, is not written over but put aside under
//! another name, and the newest backup that can be read is used. What
//! happened is said, for the person to be told.

use crate::state::{ZonesState, STATE_VERSION};
use gevrek_model::files::write_atomic;
use serde_json::Value;
use std::path::{Path, PathBuf};

const FILE: &str = "zones.json";
const BACKUPS: &str = "backups";

/// How many backups are kept where nothing else is said.
pub const KEEP: usize = 20;

/// Why a file is not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// It is not there, though backups of it are.
    Missing,
    /// It is not a state file: cut off, changed by hand into something
    /// else, or not JSON at all.
    Corrupt(String),
    /// A newer gevrek wrote it, with this layout version.
    Newer(u64),
}

/// A step takes a state document from the version before to its own.
type Step = fn(&mut Value);

/// Every step, in order, with the version it brings a document to. A new
/// `STATE_VERSION` adds a line here. There is none yet: version 1 is the
/// first.
const STEPS: &[(u32, Step)] = &[];

/// Bring a document up to the current layout.
pub fn migrate(document: &mut Value, from: u64) {
    migrate_with(document, from, STEPS, STATE_VERSION);
}

fn migrate_with(document: &mut Value, from: u64, steps: &[(u32, Step)], current: u32) {
    for (to, step) in steps {
        if from < u64::from(*to) {
            step(document);
        }
    }
    if let Some(fields) = document.as_object_mut() {
        fields.insert("version".to_string(), Value::from(current));
    }
}

/// The state as the text of its file.
pub fn encode(state: &ZonesState) -> String {
    let mut text = serde_json::to_string_pretty(state).unwrap_or_else(|_| "{}".to_string());
    text.push('\n');
    text
}

/// The state a file's text holds, and the version it was brought up from
/// if it was an older one.
pub fn decode(text: &str) -> Result<(ZonesState, Option<u64>), Unreadable> {
    let mut document: Value =
        serde_json::from_str(text).map_err(|error| Unreadable::Corrupt(error.to_string()))?;
    let version = document
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| Unreadable::Corrupt("no version".to_string()))?;
    if version > u64::from(STATE_VERSION) {
        return Err(Unreadable::Newer(version));
    }
    let older = (version < u64::from(STATE_VERSION)).then_some(version);
    if older.is_some() {
        migrate(&mut document, version);
    }
    let state: ZonesState = serde_json::from_value(document)
        .map_err(|error| Unreadable::Corrupt(error.to_string()))?;
    if let Some(broken) = state.placements.broken() {
        return Err(Unreadable::Corrupt(broken));
    }
    Ok((state, older))
}

/// What reading found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadReport {
    /// Nothing was ever saved: an empty state.
    Fresh,
    /// The file was read as it is.
    Read,
    /// The file was of an older layout and was brought up. (It is written
    /// in the new one with the next save; its old text is then a backup.)
    Migrated { from: u64 },
    /// The file could not be used.
    Recovered {
        why: Unreadable,
        /// Where the file is now, if there was one. It is never deleted.
        set_aside: Option<PathBuf>,
        /// The backup the state was taken from; without one the state is
        /// empty.
        backup: Option<PathBuf>,
        /// Backups newer than that one which could not be read either.
        /// They stay where they are.
        unreadable_backups: usize,
    },
}

/// What reading gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub state: ZonesState,
    pub report: LoadReport,
}

/// The folder the state is kept in.
#[derive(Debug, Clone)]
pub struct Store {
    folder: PathBuf,
    keep: usize,
}

impl Store {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self { folder: folder.into(), keep: KEEP }
    }

    /// Keep this many backups (one at least).
    pub fn keeping(mut self, backups: usize) -> Self {
        self.keep = backups.max(1);
        self
    }

    /// The state file.
    pub fn file(&self) -> PathBuf {
        self.folder.join(FILE)
    }

    fn backup_folder(&self) -> PathBuf {
        self.folder.join(BACKUPS)
    }

    /// The backups, the newest first.
    pub fn backups(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.backup_folder()) else {
            return Vec::new();
        };
        let mut numbered: Vec<(u64, PathBuf)> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?.to_string();
                let number = name.strip_prefix("zones-")?.strip_suffix(".json")?.parse().ok()?;
                Some((number, path))
            })
            .collect();
        numbered.sort_by_key(|(number, _)| std::cmp::Reverse(*number));
        numbered.into_iter().map(|(_, path)| path).collect()
    }

    /// Keep `text` (what the state file held until now) and drop the
    /// oldest backups beyond those that are kept. Backups are numbered,
    /// not dated: the order is right whatever the clock says.
    fn back_up(&self, text: &str) -> std::io::Result<PathBuf> {
        let existing = self.backups();
        let number = existing
            .first()
            .and_then(|newest| number_of(newest))
            .map_or(1, |newest| newest + 1);
        let path = self.backup_folder().join(format!("zones-{number:08}.json"));
        write_atomic(&path, text.as_bytes())?;
        for old in existing.iter().skip(self.keep.saturating_sub(1)) {
            let _ = std::fs::remove_file(old);
        }
        Ok(path)
    }

    /// Move a file that cannot be used out of the way, under a name that
    /// says why and that nothing else has.
    fn set_aside(&self, why: &Unreadable) -> std::io::Result<PathBuf> {
        let word = match why {
            Unreadable::Newer(_) => "newer",
            _ => "corrupt",
        };
        let mut number = 1;
        let aside = loop {
            let candidate = self.folder.join(format!("{FILE}.{word}-{number}"));
            if !candidate.exists() {
                break candidate;
            }
            number += 1;
        };
        std::fs::rename(self.file(), &aside)?;
        Ok(aside)
    }

    /// Write the state. What the file held before becomes a backup; a
    /// file that could not have been read is put aside instead, so that it
    /// pushes no good backup out. Nothing is written when nothing changed.
    pub fn save(&self, state: &ZonesState) -> std::io::Result<()> {
        let text = encode(state);
        match std::fs::read_to_string(self.file()) {
            Ok(before) if before == text => return Ok(()),
            Ok(before) => match decode(&before) {
                Ok(_) => {
                    self.back_up(&before)?;
                }
                Err(why) => {
                    self.set_aside(&why)?;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // Not text at all.
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                self.set_aside(&Unreadable::Corrupt(error.to_string()))?;
            }
            Err(error) => return Err(error),
        }
        write_atomic(&self.file(), text.as_bytes())
    }

    /// Read the state. An error is only what the disk refuses (the file
    /// is there and may not be read or moved); everything else gives a
    /// state and says how it was come by.
    pub fn load(&self) -> std::io::Result<Loaded> {
        let why = match std::fs::read_to_string(self.file()) {
            Ok(text) => match decode(&text) {
                Ok((state, None)) => return Ok(Loaded { state, report: LoadReport::Read }),
                Ok((state, Some(from))) => {
                    return Ok(Loaded { state, report: LoadReport::Migrated { from } })
                }
                Err(why) => why,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if self.backups().is_empty() {
                    return Ok(Loaded { state: ZonesState::default(), report: LoadReport::Fresh });
                }
                Unreadable::Missing
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                Unreadable::Corrupt(error.to_string())
            }
            Err(error) => return Err(error),
        };
        let set_aside = match why {
            Unreadable::Missing => None,
            _ => Some(self.set_aside(&why)?),
        };
        let mut unreadable_backups = 0;
        let mut found = None;
        for backup in self.backups() {
            let state = std::fs::read_to_string(&backup)
                .ok()
                .and_then(|text| decode(&text).ok());
            match state {
                Some((state, _)) => {
                    found = Some((state, backup));
                    break;
                }
                None => unreadable_backups += 1,
            }
        }
        let (state, backup) = match found {
            Some((state, backup)) => (state, Some(backup)),
            None => (ZonesState::default(), None),
        };
        Ok(Loaded {
            state,
            report: LoadReport::Recovered { why, set_aside, backup, unreadable_backups },
        })
    }
}

fn number_of(backup: &Path) -> Option<u64> {
    backup
        .file_name()?
        .to_str()?
        .strip_prefix("zones-")?
        .strip_suffix(".json")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ItemId, ZoneId};
    use crate::state::Snapshot;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-zones-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    /// A state that differs with every number.
    fn state(number: u64) -> ZonesState {
        let mut state = ZonesState { zones_made: number, ..ZonesState::default() };
        state.placements.assign(&ItemId::new(format!("item-{number}")), &ZoneId::new("z"), None, None);
        state.snapshots.push(Snapshot { name: format!("s{number}"), ..Snapshot::default() });
        state
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn what_is_saved_is_read_back() {
        let folder = scratch("round");
        let store = Store::new(&folder);
        assert_eq!(
            store.load().unwrap(),
            Loaded { state: ZonesState::default(), report: LoadReport::Fresh }
        );
        // (Reading made no folder and no file.)
        assert!(!folder.exists());
        store.save(&state(1)).unwrap();
        assert_eq!(store.load().unwrap(), Loaded { state: state(1), report: LoadReport::Read });
        assert_eq!(names(&folder), ["zones.json"]);
        // Keys of a newer gevrek's are kept through a round.
        let mut text: Value = serde_json::from_str(&encode(&state(1))).unwrap();
        text["later"] = serde_json::json!({"a": 1});
        std::fs::write(store.file(), text.to_string()).unwrap();
        let loaded = store.load().unwrap().state;
        assert_eq!(serde_json::to_value(&loaded).unwrap()["later"]["a"], 1);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn keys_of_a_newer_version_are_kept_at_every_depth() {
        let text = r#"{
            "version": 1,
            "top": 1,
            "placements": {"zones": {"z": {"items": ["a"], "pinned": true}}, "loose": {}, "order": "x"},
            "profiles": [{"key": {"displays": []}, "geometry": {}, "last_used": 3, "wallpaper": "w"}],
            "snapshots": [{"name": "s", "note": "n", "placements": {"shelf": 2}}],
            "undo": {"applied_at": 5, "entries": [], "moved": [], "by": "rules"}
        }"#;
        let (state, _) = decode(text).unwrap();
        let written: Value = serde_json::from_str(&encode(&state)).unwrap();
        for (pointer, value) in [
            ("/top", serde_json::json!(1)),
            ("/placements/order", serde_json::json!("x")),
            ("/placements/zones/z/pinned", serde_json::json!(true)),
            ("/profiles/0/wallpaper", serde_json::json!("w")),
            ("/snapshots/0/note", serde_json::json!("n")),
            ("/snapshots/0/placements/shelf", serde_json::json!(2)),
            ("/undo/by", serde_json::json!("rules")),
        ] {
            assert_eq!(written.pointer(pointer), Some(&value), "{pointer}");
        }
        assert_eq!(decode(&encode(&state)).unwrap().0, state);
    }

    #[test]
    fn every_save_keeps_what_was_there_and_only_the_newest_backups() {
        let folder = scratch("rotation");
        let store = Store::new(&folder).keeping(3);
        for number in 1..=6 {
            store.save(&state(number)).unwrap();
        }
        // Saving the same again writes nothing.
        store.save(&state(6)).unwrap();
        let backups = store.backups();
        assert_eq!(backups.len(), 3);
        let held: Vec<u64> = backups
            .iter()
            .map(|path| decode(&std::fs::read_to_string(path).unwrap()).unwrap().0.zones_made)
            .collect();
        assert_eq!(held, [5, 4, 3], "the newest first, each what the file held before");
        assert_eq!(store.load().unwrap().state, state(6));
        assert_eq!(names(&folder), ["backups", "zones.json"]);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_corrupt_file_is_put_aside_and_the_newest_good_backup_is_used() {
        let folder = scratch("corrupt");
        let store = Store::new(&folder);
        for number in 1..=3 {
            store.save(&state(number)).unwrap();
        }
        // The file is cut off in the middle, and the newest backup is spoilt too.
        let whole = std::fs::read_to_string(store.file()).unwrap();
        std::fs::write(store.file(), &whole[..whole.len() / 2]).unwrap();
        let newest = store.backups().remove(0);
        std::fs::write(&newest, "not json").unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.state, state(1), "the newest backup that can be read");
        let LoadReport::Recovered { why, set_aside, backup, unreadable_backups } = loaded.report else {
            panic!("{:?}", loaded.report);
        };
        assert!(matches!(why, Unreadable::Corrupt(_)));
        assert_eq!(unreadable_backups, 1);
        assert_eq!(backup, Some(store.backups().remove(1)));
        // The spoilt file is still there, as it was, under another name.
        let aside = set_aside.unwrap();
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), whole[..whole.len() / 2]);
        assert!(!store.file().exists());
        assert!(newest.exists());

        // Saving goes on from there, and a second spoilt file gets a name of its own.
        store.save(&loaded.state).unwrap();
        std::fs::write(store.file(), "[1, 2").unwrap();
        let again = store.load().unwrap();
        let LoadReport::Recovered { set_aside: second, .. } = again.report else {
            panic!("{:?}", again.report);
        };
        assert_ne!(second.unwrap(), aside);
        assert!(aside.exists());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn what_is_not_a_state_file_is_corrupt() {
        for text in ["", "null", "[]", "{}", "{\"version\": \"one\"}", "{\"version\": 1, \"profiles\": 7}"] {
            assert!(matches!(decode(text), Err(Unreadable::Corrupt(_))), "{text}");
        }
        // An item in two zones is no layout either.
        let twice = r#"{"version": 1, "placements": {"zones": {"a": {"items": ["x"]}, "b": {"items": ["x"]}}}}"#;
        assert!(matches!(decode(twice), Err(Unreadable::Corrupt(_))));
    }

    #[test]
    fn a_file_of_a_newer_gevrek_is_left_as_it_is() {
        let folder = scratch("newer");
        let store = Store::new(&folder);
        store.save(&state(1)).unwrap();
        store.save(&state(2)).unwrap();
        let newer = r#"{"version": 99, "shape": "unknown to us"}"#;
        std::fs::write(store.file(), newer).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.state, state(1));
        let LoadReport::Recovered { why, set_aside, .. } = loaded.report else {
            panic!("{:?}", loaded.report);
        };
        assert_eq!(why, Unreadable::Newer(99));
        let aside = set_aside.unwrap();
        assert!(aside.to_string_lossy().ends_with("zones.json.newer-1"));
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), newer);

        // Without any backup: an empty state, and still nothing lost.
        let alone = scratch("newer-alone");
        let store = Store::new(&alone);
        std::fs::create_dir_all(&alone).unwrap();
        std::fs::write(store.file(), newer).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.state, ZonesState::default());
        assert!(matches!(loaded.report, LoadReport::Recovered { backup: None, .. }));
        assert_eq!(names(&alone), ["zones.json.newer-1"]);
        // A save by this version writes its own file beside it.
        store.save(&state(3)).unwrap();
        assert_eq!(names(&alone), ["zones.json", "zones.json.newer-1"]);
        std::fs::remove_dir_all(folder).unwrap();
        std::fs::remove_dir_all(alone).unwrap();
    }

    #[test]
    fn a_save_over_a_file_that_cannot_be_read_puts_it_aside() {
        let folder = scratch("save-over");
        let store = Store::new(&folder);
        store.save(&state(1)).unwrap();
        std::fs::write(store.file(), "{\"version\": 99}").unwrap();
        store.save(&state(2)).unwrap();
        assert_eq!(names(&folder), ["zones.json", "zones.json.newer-1"]);
        assert!(store.backups().is_empty(), "what cannot be read is no backup");
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_file_that_went_missing_is_made_good_from_a_backup() {
        let folder = scratch("missing");
        let store = Store::new(&folder);
        store.save(&state(1)).unwrap();
        store.save(&state(2)).unwrap();
        std::fs::remove_file(store.file()).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.state, state(1));
        assert!(matches!(
            loaded.report,
            LoadReport::Recovered { why: Unreadable::Missing, set_aside: None, backup: Some(_), .. }
        ));
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_older_file_is_brought_up_step_by_step() {
        // No older layout exists yet, so the steps are made up here: 2
        // renames a field, 3 adds one.
        let steps: &[(u32, Step)] = &[
            (2, |document| {
                let old = document["zone_count"].take();
                document["zones_made"] = old;
                document.as_object_mut().unwrap().remove("zone_count");
            }),
            (3, |document| document["choices"] = Value::from(7)),
        ];
        let mut from_one = serde_json::json!({"version": 1, "zone_count": 4});
        migrate_with(&mut from_one, 1, steps, 3);
        assert_eq!(from_one, serde_json::json!({"version": 3, "zones_made": 4, "choices": 7}));
        let mut from_two = serde_json::json!({"version": 2, "zones_made": 4});
        migrate_with(&mut from_two, 2, steps, 3);
        assert_eq!(from_two, serde_json::json!({"version": 3, "zones_made": 4, "choices": 7}));
        // And through the door a file comes in by: a version before the
        // first reads with what it has and is said to be brought up.
        let (state, from) = decode(r#"{"version": 0, "zones_made": 9}"#).unwrap();
        assert_eq!((state.zones_made, state.version, from), (9, STATE_VERSION, Some(0)));
    }
}
