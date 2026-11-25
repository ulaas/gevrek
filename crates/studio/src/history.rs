//! What changed: against what the window manager runs, and against the
//! versions of the file that were kept.
//!
//! Every save of a configuration keeps the file as it was (the model does
//! that, `gevrek_model::files`). This page lists those versions and what
//! differs between one of them and the configuration in the window,
//! setting by setting, and it lists the same for the edits that are not
//! applied yet, each with a way to take that one back.

use crate::document::Document;
use egui::{RichText, Ui};
use gevrek_model::files::Backup;
use gevrek_model::Config;
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;
use serde_json::Value;
use std::time::{Duration, Instant};

/// One place that differs.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub pointer: String,
    /// What was there (`None`: nothing).
    pub before: Option<Value>,
    /// What is there now (`None`: nothing).
    pub after: Option<Value>,
}

/// The places where `after` differs from `before`. Lists of the same
/// length are compared item by item; when the length differs, the items
/// they share by position are compared and the rest are additions or
/// removals. The graph is one place: its inside is for the graph's page.
pub fn changes(before: &Value, after: &Value) -> Vec<Change> {
    let mut found = Vec::new();
    compare(before, after, String::new(), &mut found);
    found
}

fn compare(before: &Value, after: &Value, pointer: String, found: &mut Vec<Change>) {
    if before == after {
        return;
    }
    if pointer == "/node_graph" {
        found.push(Change {
            pointer,
            before: None,
            after: None,
        });
        return;
    }
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in old {
                let place = format!("{pointer}/{}", crate::document::escape(key));
                match new.get(key) {
                    Some(other) => compare(value, other, place, found),
                    None => found.push(Change {
                        pointer: place,
                        before: Some(value.clone()),
                        after: None,
                    }),
                }
            }
            for (key, value) in new {
                if !old.contains_key(key) {
                    let place = format!("{pointer}/{}", crate::document::escape(key));
                    found.push(Change {
                        pointer: place,
                        before: None,
                        after: Some(value.clone()),
                    });
                }
            }
        }
        // A list with as many items as before is compared item by item.
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => {
            for (index, (one, other)) in old.iter().zip(new).enumerate() {
                compare(one, other, format!("{pointer}/{index}"), found);
            }
        }
        // One that is longer or shorter is one change, of the whole list:
        // which item came or went cannot be told by its place (take the
        // first of three away, and by their places the first two "changed"
        // and the third "went"; taking that one back put a copy of the
        // third at the end). The list is taken back as a whole.
        (Value::Array(_), Value::Array(_)) => found.push(Change {
            pointer,
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
        _ => found.push(Change {
            pointer,
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
    }
}

/// A place as a person reads it: `/desktop/shell/panel/height` is
/// "Desktop › Bar › Height"; an item of a list is named by what it holds.
pub fn describe(pointer: &str, value: &Value) -> String {
    let mut parts = Vec::new();
    let mut walked = String::new();
    for step in pointer.split('/').skip(1) {
        walked.push('/');
        walked.push_str(step);
        match step.parse::<usize>() {
            Ok(index) => {
                let name = value
                    .pointer(&walked)
                    .map(|item| crate::form::summary(item, index))
                    .unwrap_or_else(|| (index + 1).to_string());
                parts.push(name);
            }
            Err(_) => parts.push(gevrek_schema::title(step)),
        }
    }
    parts.join(" › ")
}

/// A value on one line.
pub fn brief(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "nothing".to_string();
    };
    let text = match value {
        Value::String(text) => format!("\"{text}\""),
        Value::Array(items) if items.len() > 4 => format!("{} items", items.len()),
        Value::Object(object) if object.len() > 3 => {
            crate::form::summary(value, 0).parse::<usize>().map_or_else(
                |_| crate::form::summary(value, 0),
                |_| format!("{} settings", object.len()),
            )
        }
        other => other.to_string(),
    };
    if text.chars().count() > 56 {
        format!("{}…", text.chars().take(55).collect::<String>())
    } else {
        text
    }
}

#[derive(Default)]
pub struct HistoryPage {
    versions: Vec<Backup>,
    listed: Option<Instant>,
    /// The version looked at, and what it holds (or why it is not read).
    opened: Option<(Backup, Result<Config, String>)>,
}

impl HistoryPage {
    /// `base`: what the window manager runs (or the file holds), to show
    /// the edits that are not applied against.
    pub fn show(&mut self, ui: &mut Ui, document: &mut Document, base: Option<&Value>) {
        let palette = theme::pal(ui.ctx());
        widgets::card(ui, "Not applied yet", |ui| {
            let Some(base) = base else {
                ui.label(RichText::new("Nothing to compare with yet.").color(palette.muted));
                return;
            };
            let found = changes(base, document.value());
            if found.is_empty() {
                ui.label(RichText::new("The window shows what runs.").color(palette.muted));
                return;
            }
            let mut take_back = None;
            for change in &found {
                ui.horizontal_wrapped(|ui| {
                    if widgets::small_icon_button(
                        ui,
                        icons::ARROW_COUNTER_CLOCKWISE,
                        "Take this one back",
                    )
                    .clicked()
                    {
                        take_back = Some(change.clone());
                    }
                    change_line(ui, change, document.value(), base);
                });
            }
            if let Some(change) = take_back {
                restore(document, &change, base);
            }
        });

        // The folder is read again every few seconds: a save elsewhere adds one.
        if self
            .listed
            .is_none_or(|listed| listed.elapsed() > Duration::from_secs(4))
        {
            self.versions = gevrek_model::files::list_backups();
            self.listed = Some(Instant::now());
        }
        let mut load = None;
        widgets::card(ui, "Earlier versions", |ui| {
            if self.versions.is_empty() {
                ui.label(
                    RichText::new("None yet. Each time a configuration is saved, the file as it was is kept here.")
                        .color(palette.muted),
                );
                return;
            }
            ui.label(
                RichText::new(format!(
                    "Kept in {}",
                    gevrek_model::files::backup_dir().display()
                ))
                .color(palette.faint)
                .size(12.0),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for version in self.versions.clone() {
                        let chosen = self
                            .opened
                            .as_ref()
                            .is_some_and(|(opened, _)| opened.path == version.path);
                        ui.horizontal(|ui| {
                            let label = format!(
                                "{}   {}",
                                ago(version.taken_at),
                                file_name(&version.origin)
                            );
                            if ui
                                .selectable_label(chosen, label)
                                .on_hover_text(version.origin.display().to_string())
                                .clicked()
                            {
                                self.opened = if chosen {
                                    None
                                } else {
                                    Some((
                                        version.clone(),
                                        Config::load(&version.path)
                                            .map_err(|error| error.to_string()),
                                    ))
                                };
                            }
                        });
                    }
                });
        });
        if let Some((version, read)) = &self.opened {
            widgets::card(
                ui,
                &format!("The version of {}", ago(version.taken_at)),
                |ui| match read {
                    Err(error) => {
                        ui.colored_label(palette.error, format!("It is not read: {error}"));
                    }
                    Ok(config) => {
                        let then = serde_json::to_value(config).unwrap_or(Value::Null);
                        let found = changes(&then, document.value());
                        ui.horizontal(|ui| {
                        if widgets::secondary_button(ui, format!("{}  Load this version", icons::CLOCK_COUNTER_CLOCKWISE)).on_hover_text("Into the window; nothing runs until you apply, and Undo brings yours back").clicked() {
                            load = Some(config.clone());
                        }
                        ui.label(RichText::new(format!("{} places differ from the window", found.len())).color(palette.muted));
                    });
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_salt("differences")
                            .max_height(300.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for change in found.iter().take(200) {
                                    ui.horizontal_wrapped(|ui| {
                                        change_line(ui, change, document.value(), &then)
                                    });
                                }
                                if found.len() > 200 {
                                    ui.label(
                                        RichText::new(format!("and {} more", found.len() - 200))
                                            .color(palette.muted),
                                    );
                                }
                            });
                    }
                },
            );
        }
        if let Some(config) = load {
            document.load(&config);
            self.opened = None;
        }
    }
}

/// One difference on a line: where, what it was, what it is.
fn change_line(ui: &mut Ui, change: &Change, now: &Value, then: &Value) {
    let palette = theme::pal(ui.ctx());
    let holder = if change.after.is_some() { now } else { then };
    ui.label(RichText::new(describe(&change.pointer, holder)).strong())
        .on_hover_text(&change.pointer);
    if change.pointer == "/node_graph" {
        ui.label(RichText::new("changed").color(palette.muted));
        return;
    }
    ui.label(
        RichText::new(brief(change.before.as_ref()))
            .monospace()
            .color(palette.muted),
    );
    ui.label(RichText::new(icons::ARROW_RIGHT).color(palette.faint));
    ui.label(
        RichText::new(brief(change.after.as_ref()))
            .monospace()
            .color(palette.accent_fg()),
    );
}

/// Put one place back to what `base` has there.
fn restore(document: &mut Document, change: &Change, base: &Value) {
    match base.pointer(&change.pointer) {
        Some(old) => {
            // An item taken out of a list goes back in at its place.
            let Some((parent, key)) = change.pointer.rsplit_once('/') else {
                return;
            };
            match (
                document.get(&change.pointer).is_some(),
                key.parse::<usize>(),
            ) {
                (false, Ok(index)) => {
                    let old = old.clone();
                    document.edit(parent, |list| {
                        if let Value::Array(items) = list {
                            items.insert(index.min(items.len()), old);
                        }
                    });
                }
                _ => {
                    document.set(&change.pointer, old.clone());
                }
            }
        }
        None => {
            // It was not there: take it out again.
            let Some((parent, key)) = change.pointer.rsplit_once('/') else {
                return;
            };
            // (As the key is, not as it is written in a place: `~1` is `/`.)
            let key = key.replace("~1", "/").replace("~0", "~");
            document.edit(parent, |holder| match holder {
                Value::Array(items) => {
                    if let Ok(index) = key.parse::<usize>() {
                        if index < items.len() {
                            items.remove(index);
                        }
                    }
                }
                Value::Object(object) => {
                    object.remove(&key);
                }
                _ => {}
            });
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || "a file".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// How long ago a moment (seconds since 1970) was.
fn ago(taken_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(taken_at);
    span(now.saturating_sub(taken_at))
}

fn span(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=86_399 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn differences_are_found_place_by_place() {
        let before = json!({"desktop": {"shell": {"panel": {"height": 36, "margin": 8}}, "outputs": [{"name": "DP-1"}], "old": 1}});
        let after = json!({"desktop": {"shell": {"panel": {"height": 44, "margin": 8}}, "outputs": [{"name": "DP-1"}, {"name": "DP-2"}], "new": true}});
        let found = changes(&before, &after);
        let places: Vec<&str> = found.iter().map(|change| change.pointer.as_str()).collect();
        assert_eq!(
            places,
            [
                "/desktop/shell/panel/height",
                // (A list that is longer or shorter: one change, the list's.)
                "/desktop/outputs",
                "/desktop/old",
                "/desktop/new"
            ]
        );
        assert_eq!(found[0].before, Some(json!(36)));
        assert_eq!(found[0].after, Some(json!(44)));
        assert_eq!(found[1].before, Some(json!([{"name": "DP-1"}])));
        assert_eq!(found[2].after, None);
        assert!(changes(&after, &after).is_empty());
    }

    #[test]
    fn the_graph_is_one_place() {
        let before = json!({"node_graph": {"nodes": [1, 2], "next": 3}});
        let after = json!({"node_graph": {"nodes": [1, 2, 3], "next": 4}});
        let found = changes(&before, &after);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pointer, "/node_graph");
    }

    #[test]
    fn a_place_reads_as_words() {
        let value = json!({"desktop": {"bindings": [{"name": "Terminal", "key": "T"}], "shell": {"panel": {"corner_radius": 12}}}});
        assert_eq!(
            describe("/desktop/shell/panel/corner_radius", &value),
            "Desktop › Shell › Panel › Corner radius"
        );
        assert_eq!(
            describe("/desktop/bindings/0/key", &value),
            "Desktop › Bindings › Terminal › Key"
        );
        assert_eq!(brief(Some(&json!("foot"))), "\"foot\"");
        assert_eq!(brief(None), "nothing");
        assert_eq!(brief(Some(&json!([1, 2, 3, 4, 5, 6]))), "6 items");
    }

    #[test]
    fn one_change_is_taken_back_and_the_others_stay() {
        let base_config = Config::default();
        let base = serde_json::to_value(&base_config).expect("JSON");
        let mut document = Document::new(&base_config);
        document.set("/desktop/shell/panels/0/height", json!(44));
        document.set("/desktop/shell/panels/0/margin", json!(3));
        document.edit("/desktop/outputs", |outputs| {
            if let Value::Array(outputs) = outputs {
                outputs.push(json!({"name": "DP-1"}));
            }
        });
        let found = changes(&base, document.value());
        assert_eq!(found.len(), 3);
        let height = found
            .iter()
            .find(|change| change.pointer == "/desktop/shell/panels/0/height")
            .expect("height")
            .clone();
        restore(&mut document, &height, &base);
        let added = found
            .iter()
            .find(|change| change.pointer == "/desktop/outputs")
            .expect("output")
            .clone();
        restore(&mut document, &added, &base);
        let left = changes(&base, document.value());
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].pointer, "/desktop/shell/panels/0/margin");
        assert!(document.config().is_some());
    }

    #[test]
    fn an_item_taken_out_of_a_list_comes_back_at_its_place() {
        let base_config = Config::default();
        let mut document = Document::new(&base_config);
        document.set(
            "/desktop/outputs",
            json!([{"name": "A"}, {"name": "B"}, {"name": "C"}]),
        );
        let base = document.value().clone();
        // The first of the three is taken away.
        document.edit("/desktop/outputs", |outputs| {
            if let Value::Array(outputs) = outputs {
                outputs.remove(0);
            }
        });
        let found = changes(&base, document.value());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pointer, "/desktop/outputs");
        restore(&mut document, &found[0], &base);
        // As it was: not B, C and a second C.
        assert_eq!(document.value(), &base);
    }

    #[test]
    fn a_span_reads_as_time() {
        assert_eq!(span(5), "just now");
        assert_eq!(span(180), "3 min ago");
        assert_eq!(span(7300), "2 h ago");
        assert_eq!(span(200_000), "2 days ago");
    }
}
