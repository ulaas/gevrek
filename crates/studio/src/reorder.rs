//! A list of the configuration put into another order by dragging: each
//! entry a line that is taken up by its handle and put down between two
//! others.
//!
//! The window rules use it: they are tried from the first to the last,
//! and with rules that hold for the same window the order is what
//! decides. The same list is in the form below it, where an entry is
//! changed; this is the way to move one.

use crate::document::Document;
use egui::{RichText, Ui};
use gevrek_ui::theme::{self, icons};
use serde_json::Value;

/// A list after the entry at `from` was put down before the one at `to`
/// (the list's length: at its end). Nothing changes where either is no
/// place of the list, or the entry is put down where it is.
pub fn moved<T>(list: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= list.len() || to > list.len() || to == from || to == from + 1 {
        return false;
    }
    let entry = list.remove(from);
    // Taken out before the place it goes to: that place is one nearer.
    let to = if from < to { to - 1 } else { to };
    list.insert(to, entry);
    true
}

/// What a line says of an entry: its name, what else is to be said of
/// it, and whether it is switched on.
pub struct Line {
    pub name: String,
    pub detail: String,
    pub enabled: bool,
}

/// Draw the list at `pointer` as lines to be dragged, and carry out what
/// was dragged. `describe` says what a line shows of an entry.
pub fn show(
    ui: &mut Ui,
    document: &mut Document,
    pointer: &str,
    describe: impl Fn(&Value) -> Line,
) {
    let palette = theme::pal(ui.ctx());
    let entries: Vec<Value> = document
        .value()
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        ui.label(RichText::new("Nothing to put in order yet.").color(palette.muted));
        return;
    }
    if entries.len() == 1 {
        ui.label(RichText::new("One entry: nothing to put in order.").color(palette.muted));
    } else {
        ui.label(
            RichText::new(
                "Drag a line by its handle to where it is to stand: the first is tried first.",
            )
            .color(palette.muted),
        );
    }
    ui.add_space(4.0);
    let mut done: Option<(usize, usize)> = None;
    for (index, entry) in entries.iter().enumerate() {
        let line = describe(entry);
        let id = egui::Id::unique(("a line to put in order", pointer, index));
        let response = ui
            .dnd_drag_source(id, index, |ui| {
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_min_width((ui.available_width() - 24.0).max(160.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icons::DOTS_SIX_VERTICAL).color(palette.faint));
                            ui.label(
                                RichText::new(format!("{}.", index + 1))
                                    .monospace()
                                    .color(palette.faint),
                            );
                            let ink = if line.enabled {
                                palette.text
                            } else {
                                palette.muted
                            };
                            ui.label(RichText::new(&line.name).color(ink));
                            if !line.enabled {
                                ui.label(RichText::new("off").size(11.5).color(palette.faint));
                            }
                            ui.label(RichText::new(&line.detail).size(12.0).color(palette.muted));
                        });
                    });
            })
            .response;
        // Put down on a line: before it, or behind it when it is let go
        // over its lower half.
        if let Some(from) = response.dnd_release_payload::<usize>() {
            let behind = ui
                .ctx()
                .pointer_interact_pos()
                .is_some_and(|at| at.y > response.rect.center().y);
            done = Some((*from, index + usize::from(behind)));
        }
        // (Where the line that is dragged would go: a mark between two.)
        if response.dnd_hover_payload::<usize>().is_some() {
            let behind = ui
                .ctx()
                .pointer_interact_pos()
                .is_some_and(|at| at.y > response.rect.center().y);
            let y = if behind {
                response.rect.bottom() + 1.0
            } else {
                response.rect.top() - 1.0
            };
            ui.painter().hline(
                response.rect.x_range(),
                y,
                egui::Stroke::new(2.0, palette.accent),
            );
        }
        ui.add_space(2.0);
    }
    if let Some((from, to)) = done {
        document.edit(pointer, |value| {
            if let Some(list) = value.as_array_mut() {
                moved(list, from, to);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_is_put_down_between_two_others() {
        let order = |from: usize, to: usize| {
            let mut list = vec!['a', 'b', 'c', 'd'];
            let changed = moved(&mut list, from, to);
            (list.into_iter().collect::<String>(), changed)
        };
        // Down the list, up the list, to its ends.
        assert_eq!(order(0, 3), ("bcad".to_string(), true));
        assert_eq!(order(3, 1), ("adbc".to_string(), true));
        assert_eq!(order(1, 4), ("acdb".to_string(), true));
        assert_eq!(order(2, 0), ("cabd".to_string(), true));
        // Where it is (before itself, or behind itself): nothing.
        assert_eq!(order(1, 1), ("abcd".to_string(), false));
        assert_eq!(order(1, 2), ("abcd".to_string(), false));
        // No such place: nothing.
        assert_eq!(order(7, 0), ("abcd".to_string(), false));
        assert_eq!(order(0, 9), ("abcd".to_string(), false));
    }

    #[test]
    fn the_rules_of_a_document_are_put_in_another_order() {
        let mut document = Document::new(&gevrek_model::Config::default());
        let rule = |name: &str| serde_json::json!({"name": name});
        document.edit("/desktop/window_rules", |rules| {
            *rules = serde_json::json!([rule("first"), rule("second"), rule("third")]);
        });
        document.edit("/desktop/window_rules", |value| {
            moved(value.as_array_mut().expect("a list"), 2, 0);
        });
        let names: Vec<&str> = document
            .value()
            .pointer("/desktop/window_rules")
            .and_then(Value::as_array)
            .expect("the rules")
            .iter()
            .filter_map(|rule| rule["name"].as_str())
            .collect();
        assert_eq!(names, ["third", "first", "second"]);
    }
}
