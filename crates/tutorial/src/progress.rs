//! What the tutorial remembers between sessions.
//!
//! Kept with the instance's state (`tutorial.json`): that the person was
//! welcomed, which lessons they finished (and which revision of each),
//! where a lesson was left, and the version of gevrek they last saw.
//! Nothing of the person's own is in it.

use crate::lessons::{lessons, Lesson};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    /// The welcome was shown, and answered (also with "later").
    pub welcomed: bool,
    /// The lessons that were finished, each with the revision it had.
    pub finished: BTreeMap<String, u32>,
    /// A lesson that was left before its end, and the step it was at.
    pub left_at: Option<(String, usize)>,
    /// A lesson is being taken now (so that one that ended with the
    /// program, and not by its own end, is tidied up after).
    pub running: bool,
    /// The version of gevrek that was last seen.
    pub version: String,
}

/// Where the tutorial's memory is kept.
pub fn file() -> PathBuf {
    gevrek_model::paths::state_dir().join("tutorial.json")
}

/// A version as numbers, to tell which is the later (`0.7.0`).
fn numbers(version: &str) -> Vec<u32> {
    version
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

/// Whether `one` is a later version than `other`.
pub fn later(one: &str, other: &str) -> bool {
    numbers(one) > numbers(other)
}

impl Progress {
    /// What is kept in this file; nothing where there is none, or where
    /// it cannot be read.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(error) = std::fs::write(path, text) {
                    log::warn!("the tutorial's memory is not written: {error}");
                }
            }
            Err(error) => log::warn!("the tutorial's memory is not written: {error}"),
        }
    }

    /// Whether this is the first time gevrek runs for this person: there
    /// is no configuration of their own yet, and nobody was welcomed.
    pub fn first_launch(&self, has_configuration: bool) -> bool {
        !has_configuration && !self.welcomed
    }

    /// Whether a lesson was finished as it is now.
    pub fn has_finished(&self, lesson: &Lesson) -> bool {
        self.finished
            .get(lesson.id)
            .is_some_and(|revision| *revision >= lesson.revision)
    }

    /// A lesson was finished.
    pub fn finish(&mut self, lesson: &Lesson) {
        self.finished.insert(lesson.id.to_string(), lesson.revision);
        if self.left_at.as_ref().is_some_and(|(id, _)| id == lesson.id) {
            self.left_at = None;
        }
        self.running = false;
    }

    /// A lesson was left at this step.
    pub fn leave(&mut self, lesson: &Lesson, step: usize) {
        self.left_at = (step > 0).then(|| (lesson.id.to_string(), step));
        self.running = false;
    }

    /// The lesson to offer next: the one that was left, else the first
    /// that is not finished.
    pub fn next(&self) -> Option<(&'static Lesson, usize)> {
        if let Some((id, step)) = &self.left_at {
            if let Some(lesson) = crate::lessons::lesson(id) {
                return Some((lesson, *step));
            }
        }
        lessons()
            .iter()
            .find(|lesson| !self.has_finished(lesson))
            .map(|lesson| (lesson, 0))
    }

    /// How many lessons are finished, of how many.
    pub fn count(&self) -> (usize, usize) {
        (
            lessons()
                .iter()
                .filter(|lesson| self.has_finished(lesson))
                .count(),
            lessons().len(),
        )
    }

    /// What is new for someone who last saw another version: the lessons
    /// that came since. Nothing for someone who was never here (they are
    /// welcomed instead) and nothing where the version is the same.
    pub fn news(&self, running: &str) -> Vec<&'static Lesson> {
        if self.version.is_empty() || !later(running, &self.version) {
            return Vec::new();
        }
        lessons()
            .iter()
            .filter(|lesson| later(lesson.since, &self.version) && !later(lesson.since, running))
            .collect()
    }

    /// This version was seen.
    pub fn seen(&mut self, running: &str) {
        self.version = running.to_string();
    }
}

/// (What spoils a file from outside: the engine's, taken in as it is.)
#[cfg(test)]
#[path = "../../engine/src/spoil.rs"]
mod spoil;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lessons::lesson;

    #[test]
    fn what_is_finished_and_where_it_was_left_is_kept() {
        let folder = std::env::temp_dir().join(format!("gevrek-tutorial-{}", std::process::id()));
        let path = folder.join("tutorial.json");
        let mut progress = Progress::load(&path);
        assert_eq!(progress, Progress::default());
        // Nobody was here, and there is no configuration: the first time.
        assert!(progress.first_launch(false));
        assert!(!progress.first_launch(true));
        // The first lesson is the one to offer.
        assert_eq!(progress.next().map(|(lesson, step)| (lesson.id, step)), Some(("tiling", 0)));
        let focus = lesson("focus").expect("the lesson");
        progress.welcomed = true;
        progress.finish(lesson("tiling").expect("the lesson"));
        progress.leave(focus, 2);
        progress.save(&path);
        let read = Progress::load(&path);
        assert_eq!(read, progress);
        assert!(!read.first_launch(false));
        assert_eq!(read.count().0, 1);
        // The one that was left is offered where it was left.
        assert_eq!(read.next().map(|(lesson, step)| (lesson.id, step)), Some(("focus", 2)));
        // Finished: the next one, from its beginning.
        progress.finish(focus);
        assert_eq!(progress.next().map(|(lesson, step)| (lesson.id, step)), Some(("move", 0)));
        // A lesson that was changed since is not finished any more.
        progress.finished.insert("tiling".to_string(), 0);
        assert_eq!(progress.next().map(|(lesson, _)| lesson.id), Some("tiling"));
        // What cannot be read is nothing.
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Progress::load(&path), Progress::default());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn what_is_new_is_what_came_since_the_version_last_seen() {
        assert!(later("0.7.0", "0.6.0") && later("0.10.0", "0.9.9") && later("1.0", "0.99.3"));
        assert!(!later("0.6.0", "0.6.0") && !later("0.6.0", "0.7.0"));
        let mut progress = Progress::default();
        // Never here: welcomed, not told what is new.
        assert!(progress.news("0.7.0").is_empty());
        progress.seen("0.6.0");
        let new = progress.news("0.7.0");
        assert!(!new.is_empty() && new.iter().all(|lesson| lesson.since == "0.7.0"));
        // The same version, or an older one running: nothing.
        progress.seen("0.7.0");
        assert!(progress.news("0.7.0").is_empty());
        assert!(progress.news("0.6.0").is_empty());
    }

    /// The tutorial's memory is a file anyone may have written: with
    /// nonsense where a value belongs, or cut short, it is read as what
    /// can be read of it or as nothing, and a lesson is begun from it
    /// all the same (a step that no lesson has is the lesson's last).
    #[test]
    fn a_memory_from_outside_is_lived_with() {
        use super::spoil::{harder, lived, places, spoil, Chance};
        let mut whole = Progress {
            welcomed: true,
            left_at: Some(("focus".to_string(), 2)),
            running: true,
            version: "0.6.0".to_string(),
            ..Progress::default()
        };
        for lesson in lessons() {
            whole.finished.insert(lesson.id.to_string(), lesson.revision);
        }
        let whole = serde_json::to_value(&whole).expect("as json");
        let text = whole.to_string();
        let folder =
            std::env::temp_dir().join(format!("gevrek-tutorial-outside-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let path = folder.join("tutorial.json");

        let taken = |text: &str, path: &Path| {
            std::fs::write(path, text).expect("written");
            let mut progress = Progress::load(path);
            let _ = progress.count();
            let _ = progress.news("0.7.0");
            let _ = progress.first_launch(false);
            if let Some((lesson, step)) = progress.next() {
                let run = crate::run::Run::begin(lesson, step, std::time::Instant::now());
                let _ = run.step();
                progress.leave(lesson, step);
                progress.finish(lesson);
            }
            progress.seen("0.7.0");
            progress.save(path);
            assert_eq!(Progress::load(path), progress, "what is kept is read back");
        };

        let (times, seed) = harder();
        let mut chance = Chance(0xa54f_f53a_5f1d_36f1 ^ seed.rotate_left(3) | 1);
        let mut ended: Vec<String> = Vec::new();
        for end in 0..text.len() {
            let (part, path) = (text[..end].to_string(), path.clone());
            if let Err(panic) = lived(move || taken(&part, &path)) {
                ended.push(format!("cut at {end}: {panic}"));
            }
        }
        let mut all = Vec::new();
        places(&whole, &mut Vec::new(), &mut all);
        for _ in 0..400 * times {
            let mut document = whole.clone();
            let done = spoil(&mut document, &all, &mut chance);
            let (spoilt, path) = (document.to_string(), path.clone());
            if let Err(panic) = lived(move || taken(&spoilt, &path)) {
                ended.push(format!("with {done}: {panic}"));
            }
        }
        // A step as far as numbers go, in a lesson that is there.
        let far = format!(r#"{{"left_at": ["focus", {}]}}"#, usize::MAX);
        let far_path = path.clone();
        if let Err(panic) = lived(move || taken(&far, &far_path)) {
            ended.push(format!("the furthest step: {panic}"));
        }
        let _ = std::fs::remove_dir_all(folder);
        ended.truncate(20);
        assert!(ended.is_empty(), "{}", ended.join("\n"));
    }
}
