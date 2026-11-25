//! The configuration being edited.
//!
//! The forms work on the configuration as JSON, at the places the schema
//! names, because that is what lets one form show every setting. After
//! each change the JSON is read back into the model's own type: that is
//! the check (the model decides what is valid, not the studio), and the
//! typed configuration is what is sent to the window manager.
//!
//! Undo keeps whole configurations: they are small, and nothing can be
//! half undone. Changes that belong together (dragging a slider, typing
//! in one field) are one step.

use gevrek_model::settings::{ConfigIssue, ConfigIssueSeverity};
use gevrek_model::Config;
use serde_json::Value;
use std::time::{Duration, Instant};

/// How many steps can be taken back.
const UNDO_STEPS: usize = 500;
/// Changes to the same place closer together than this are one step.
const SAME_STEP: Duration = Duration::from_millis(900);

pub struct Document {
    /// What the forms show and change.
    value: Value,
    /// `value` as the model reads it, or why it does not.
    typed: Result<Config, String>,
    /// What the model's own validation says about it.
    issues: Vec<ConfigIssue>,
    /// gevrek's defaults, to say which settings differ from them.
    defaults: Value,
    undo: Vec<Value>,
    redo: Vec<Value>,
    /// The place and time of the last change, to join it with the next.
    last: Option<(String, Instant)>,
    /// Counts every change: who watches the document compares it.
    generation: u64,
    /// Whether the model read it at the last check (`None`: not checked
    /// yet), for the log to say when that changes.
    was_typed: Option<bool>,
}

impl Document {
    pub fn new(config: &Config) -> Self {
        let mut document = Self {
            value: Value::Null,
            typed: Err(String::new()),
            issues: Vec::new(),
            defaults: serde_json::to_value(Config::default()).unwrap_or(Value::Null),
            was_typed: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last: None,
            generation: 0,
        };
        document.replace(config);
        document
    }

    /// Take another configuration as the document (the window manager's,
    /// a file's). What could be undone is forgotten: it was about the
    /// configuration before.
    pub fn replace(&mut self, config: &Config) {
        self.value = serde_json::to_value(config).unwrap_or(Value::Null);
        self.undo.clear();
        self.redo.clear();
        self.last = None;
        self.check();
    }

    /// Take another configuration as an edit of this one, which can be
    /// undone (an example loaded, a version restored).
    pub fn load(&mut self, config: &Config) {
        self.remember(None);
        self.value = serde_json::to_value(config).unwrap_or(Value::Null);
        self.check();
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn get(&self, pointer: &str) -> Option<&Value> {
        self.value.pointer(pointer)
    }

    /// The configuration as the model reads it, when it does.
    pub fn config(&self) -> Option<&Config> {
        self.typed.as_ref().ok()
    }

    /// Why the model does not read the document, when it does not.
    pub fn error(&self) -> Option<&str> {
        self.typed.as_ref().err().map(String::as_str)
    }

    pub fn issues(&self) -> &[ConfigIssue] {
        &self.issues
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Put `new` at `pointer`. Returns whether anything changed.
    pub fn set(&mut self, pointer: &str, new: Value) -> bool {
        if self.value.pointer(pointer) == Some(&new) {
            return false;
        }
        self.remember(Some(pointer));
        match self.value.pointer_mut(pointer) {
            Some(place) => *place = new,
            None => {
                // A key that is not there yet (a field the file left out).
                let Some((parent, key)) = pointer.rsplit_once('/') else {
                    return false;
                };
                match self.value.pointer_mut(parent) {
                    Some(Value::Object(object)) => {
                        object.insert(unescape(key), new);
                    }
                    _ => return false,
                }
            }
        }
        self.check();
        true
    }

    /// Change the value at `pointer` in place (a list: add, remove, move).
    pub fn edit(&mut self, pointer: &str, change: impl FnOnce(&mut Value)) -> bool {
        let Some(before) = self.value.pointer(pointer).cloned() else {
            return false;
        };
        let mut after = before.clone();
        change(&mut after);
        if after == before {
            return false;
        }
        // Never joined with the change before: a list item added twice is
        // two steps.
        self.remember(None);
        if let Some(place) = self.value.pointer_mut(pointer) {
            *place = after;
        }
        self.check();
        true
    }

    /// Whether the value at `pointer` is not gevrek's default.
    pub fn differs_from_default(&self, pointer: &str) -> bool {
        self.value.pointer(pointer) != self.defaults.pointer(pointer)
    }

    pub fn default_at(&self, pointer: &str) -> Option<&Value> {
        self.defaults.pointer(pointer)
    }

    /// Put gevrek's default back at `pointer`.
    pub fn reset(&mut self, pointer: &str) -> bool {
        match self.defaults.pointer(pointer).cloned() {
            Some(default) => self.set(pointer, default),
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.value, before));
        self.last = None;
        self.check();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.value, after));
        self.last = None;
        self.check();
        true
    }

    /// Keep the document as it is now as a step to come back to, unless
    /// this change continues the last one at the same place.
    fn remember(&mut self, pointer: Option<&str>) {
        let now = Instant::now();
        let continues = match (&self.last, pointer) {
            (Some((place, when)), Some(pointer)) => {
                place == pointer && now.duration_since(*when) < SAME_STEP
            }
            _ => false,
        };
        if !continues {
            self.undo.push(self.value.clone());
            if self.undo.len() > UNDO_STEPS {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = pointer.map(|pointer| (pointer.to_string(), now));
    }

    fn check(&mut self) {
        self.generation += 1;
        self.typed =
            serde_json::from_value::<Config>(self.value.clone()).map_err(|error| error.to_string());
        // Kept as the model writes it: a value a form put in with only
        // some of its fields has the rest at their defaults at once, and
        // the document is the same text as what the window manager gives
        // back once it is applied (or it would count as changed forever).
        if let Ok(config) = &self.typed {
            if let Ok(written) = serde_json::to_value(config) {
                self.value = written;
            }
        }
        let (was_typed, issues_before) = (self.was_typed, self.issues.len());
        self.issues = match &self.typed {
            Ok(config) => config.validate(),
            Err(_) => Vec::new(),
        };
        // Said when it is another than before, not at each edit (a slider
        // that is dragged edits at every frame).
        self.was_typed = Some(self.typed.is_ok());
        if was_typed != self.was_typed {
            match &self.typed {
                Ok(_) => log::debug!(
                    "the document is a configuration ({} issues)",
                    self.issues.len()
                ),
                Err(error) => log::debug!("the document is no configuration: {error}"),
            }
        } else if self.typed.is_ok() && issues_before != self.issues.len() {
            log::debug!(
                "the configuration's check finds {} issues",
                self.issues.len()
            );
        }
    }

    /// How many issues lie at or under `pointer` (`/desktop/shell/panel`); errors
    /// and warnings apart.
    pub fn issues_under(&self, pointer: &str) -> (usize, usize) {
        let prefix = pointer.trim_start_matches('/').replace('/', ".");
        let mut errors = 0;
        let mut warnings = 0;
        for issue in &self.issues {
            let path = issue.path.replace('[', ".").replace(']', "");
            if path == prefix || path.starts_with(&format!("{prefix}.")) {
                if issue.severity == ConfigIssueSeverity::Warning {
                    warnings += 1;
                } else {
                    errors += 1;
                }
            }
        }
        (errors, warnings)
    }
}

/// A key as it is written in a JSON pointer, back to the key.
fn unescape(key: &str) -> String {
    key.replace("~1", "/").replace("~0", "~")
}

/// A key, as it is written in a JSON pointer.
pub fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_checked_by_the_model_and_can_be_taken_back() {
        let mut document = Document::new(&Config::default());
        assert!(document.config().is_some());
        assert!(!document.can_undo());
        let before = document
            .get("/desktop/shell/panels/0/height")
            .cloned()
            .expect("the bar's height");
        assert!(!document.differs_from_default("/desktop/shell/panels/0/height"));

        assert!(document.set("/desktop/shell/panels/0/height", Value::from(48)));
        assert_eq!(
            document.config().expect("still valid").desktop.shell.panels[0].height,
            48
        );
        assert!(document.differs_from_default("/desktop/shell/panels/0/height"));
        // The same value again is no change, and no step.
        assert!(!document.set("/desktop/shell/panels/0/height", Value::from(48)));

        assert!(document.undo());
        assert_eq!(
            document.get("/desktop/shell/panels/0/height"),
            Some(&before)
        );
        assert!(document.redo());
        assert_eq!(
            document.config().expect("valid").desktop.shell.panels[0].height,
            48
        );
        assert!(document.reset("/desktop/shell/panels/0/height"));
        assert!(!document.differs_from_default("/desktop/shell/panels/0/height"));
    }

    #[test]
    fn what_the_model_does_not_read_is_said_and_not_sent() {
        let mut document = Document::new(&Config::default());
        assert!(document.set("/desktop/shell/panels/0/height", Value::from("tall")));
        assert!(document.config().is_none());
        assert!(document.error().is_some_and(|error| !error.is_empty()));
        assert!(document.undo());
        assert!(document.config().is_some());
    }

    #[test]
    fn dragging_a_slider_is_one_step() {
        let mut document = Document::new(&Config::default());
        for height in 30..40 {
            document.set("/desktop/shell/panels/0/height", Value::from(height));
        }
        // Another place is another step.
        document.set("/desktop/shell/panels/0/margin", Value::from(3));
        assert!(document.undo());
        assert_eq!(
            document.get("/desktop/shell/panels/0/height"),
            Some(&Value::from(39))
        );
        assert!(document.undo());
        assert!(!document.differs_from_default("/desktop/shell/panels/0/height"));
        assert!(!document.can_undo());
    }

    #[test]
    fn a_list_changes_in_place_and_each_change_is_a_step() {
        let mut document = Document::new(&Config::default());
        let add = |document: &mut Document| {
            document.edit("/desktop/outputs", |outputs| {
                if let Value::Array(outputs) = outputs {
                    outputs.push(serde_json::json!({"name": "DP-1"}));
                }
            })
        };
        assert!(add(&mut document));
        assert!(add(&mut document));
        assert_eq!(document.config().expect("valid").desktop.outputs.len(), 2);
        assert!(document.undo());
        assert_eq!(document.config().expect("valid").desktop.outputs.len(), 1);
        // Two outputs of one name: the model's validation says so.
        assert!(add(&mut document));
        let (errors, warnings) = document.issues_under("/desktop/outputs");
        assert!(errors + warnings > 0, "{:?}", document.issues());
    }

    #[test]
    fn a_value_put_in_by_halves_is_held_as_the_model_writes_it() {
        let mut document = Document::new(&Config::default());
        document.set("/desktop/outputs", serde_json::json!([{"name": "DP-1"}]));
        // The fields that were not given are there, at their defaults.
        assert_eq!(
            document.get("/desktop/outputs/0/enabled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            document.get("/desktop/outputs/0/transform"),
            Some(&Value::from("normal"))
        );
        // And the document is what the same configuration reads back as.
        let again = serde_json::to_value(document.config().expect("valid")).expect("JSON");
        assert_eq!(&again, document.value());
    }

    #[test]
    fn a_configuration_from_outside_starts_a_new_history() {
        let mut document = Document::new(&Config::default());
        document.set("/desktop/shell/panels/0/height", Value::from(48));
        let mut other = Config::default();
        other.desktop.shell.panels[0].height = 30;
        document.replace(&other);
        assert!(!document.can_undo());
        // Loaded as an edit, it can be taken back.
        document.load(&Config::default());
        assert!(document.can_undo());
        assert!(document.undo());
        assert_eq!(
            document.config().expect("valid").desktop.shell.panels[0].height,
            30
        );
    }
}
