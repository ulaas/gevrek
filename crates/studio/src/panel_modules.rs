//! A panel's modules, placed by dragging: three rows for the panel's
//! left, its middle and its right, each module a piece that is taken up
//! and put down in another place or another row; below them the modules
//! that are not on the panel, to be dragged onto it. A piece dragged to
//! those is taken off.
//!
//! The same lists are in the form below it (`left`, `centre`, `right` of
//! `desktop.shell.panel`); this is the quicker way to the same thing.

use crate::document::Document;
use egui::{RichText, Ui};
use gevrek_ui::theme;
use serde_json::Value;

const PANEL: &str = "/desktop/shell/panel";
/// The rows, by their names in the configuration, and what they are called.
const SIDES: [(&str, &str); 3] = [("left", "Left"), ("centre", "Middle"), ("right", "Right")];

/// Where a module is taken from: a row and its place there, or the
/// modules that are not on the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum From {
    Row(usize, usize),
    Unused(String),
}

/// Where it is put down: a row, before the piece at this place (the
/// row's length: at its end), or off the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    Row(usize, usize),
    Off,
}

/// The three rows after a module was moved. One that is put down in its
/// own row keeps the row's length; a spacer may stand on a panel more
/// than once, any other module is taken out of where else it stood.
pub fn moved(rows: &[Vec<String>; 3], from: &From, to: To) -> [Vec<String>; 3] {
    let mut rows = rows.clone();
    let (module, came_from) = match from {
        From::Row(row, index) => {
            if *row >= 3 || *index >= rows[*row].len() {
                return rows;
            }
            (rows[*row].remove(*index), Some((*row, *index)))
        }
        From::Unused(module) => (module.clone(), None),
    };
    let To::Row(row, mut index) = to else {
        return rows;
    };
    if row >= 3 {
        return rows;
    }
    // Taken out of the same row before the place it goes to: that place
    // is one nearer now.
    if let Some((from_row, from_index)) = came_from {
        if from_row == row && from_index < index {
            index -= 1;
        }
    }
    if module != "spacer" {
        for other in rows.iter_mut() {
            other.retain(|there| *there != module);
        }
    }
    let index = index.min(rows[row].len());
    rows[row].insert(index, module);
    rows
}

/// The modules that can stand on a panel and do not stand on this one
/// (a spacer always can: there may be several).
pub fn unused(rows: &[Vec<String>; 3]) -> Vec<String> {
    gevrek_model::settings::PanelModule::all()
        .iter()
        .filter_map(|module| serde_json::to_value(module).ok())
        .filter_map(|value| value.as_str().map(str::to_string))
        .filter(|name| name == "spacer" || !rows.iter().flatten().any(|there| there == name))
        .collect()
}

fn rows_of(document: &Document) -> [Vec<String>; 3] {
    SIDES.map(|(side, _)| {
        document
            .get(&format!("{PANEL}/{side}"))
            .and_then(Value::as_array)
            .map(|modules| {
                modules
                    .iter()
                    .filter_map(|module| module.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// A piece: a module by its name.
fn piece(ui: &mut Ui, name: &str, palette: &theme::Palette) {
    egui::Frame::new()
        .fill(palette.control)
        .stroke(egui::Stroke::new(1.0, palette.border_strong()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            // On one line: a piece is as wide as its word.
            ui.add(egui::Label::new(RichText::new(gevrek_schema::title(name)).size(13.0)).extend());
        });
}

/// Begin a new line of pieces when what is left of this one is too
/// narrow for the next (its word would be broken letter by letter).
fn room_for(ui: &mut Ui, name: &str) {
    let word = ui.painter().layout_no_wrap(
        gevrek_schema::title(name),
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
    );
    // (The piece's own margins and edge beside the word.)
    if ui.available_size_before_wrap().x < word.size().x + 22.0 {
        ui.end_row();
    }
}

/// Draw the rows and carry out what was dragged.
pub fn show(ui: &mut Ui, document: &mut Document) {
    let palette = theme::pal(ui.ctx());
    let rows = rows_of(document);
    let mut done: Option<(From, To)> = None;
    ui.label(
        RichText::new("Drag a module to where it is to stand; drag it down to take it off.")
            .color(palette.muted),
    );
    ui.add_space(4.0);
    for (row, (_, title)) in SIDES.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized(
                [58.0, 28.0],
                egui::Label::new(RichText::new(*title).color(palette.muted)),
            );
            let frame = egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border))
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(8, 6));
            let (_, dropped) = ui.dnd_drop_zone::<From, ()>(frame, |ui| {
                ui.set_min_width((ui.available_width() - 20.0).max(120.0));
                ui.set_min_height(28.0);
                ui.horizontal_wrapped(|ui| {
                    for (index, module) in rows[row].iter().enumerate() {
                        room_for(ui, module);
                        let id = egui::Id::unique(("panel module", row, index, module));
                        let response = ui
                            .dnd_drag_source(id, From::Row(row, index), |ui| {
                                piece(ui, module, &palette)
                            })
                            .response;
                        // Put down on a piece: before it, or behind it
                        // when it is let go over its right half.
                        if let Some(from) = response.dnd_release_payload::<From>() {
                            let behind = ui
                                .ctx()
                                .pointer_interact_pos()
                                .is_some_and(|at| at.x > response.rect.center().x);
                            done =
                                Some(((*from).clone(), To::Row(row, index + usize::from(behind))));
                        }
                    }
                    if rows[row].is_empty() {
                        ui.label(RichText::new("nothing here").color(palette.faint));
                    }
                });
            });
            // Put down on the row's free room: at its end.
            if let (Some(from), None) = (dropped, &done) {
                done = Some(((*from).clone(), To::Row(row, rows[row].len())));
            }
        });
        ui.add_space(4.0);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [58.0, 28.0],
            egui::Label::new(RichText::new("Not on it").color(palette.muted)),
        );
        let frame = egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, palette.border))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(8, 6));
        let (_, dropped) = ui.dnd_drop_zone::<From, ()>(frame, |ui| {
            ui.set_min_width((ui.available_width() - 20.0).max(120.0));
            ui.horizontal_wrapped(|ui| {
                for module in unused(&rows) {
                    room_for(ui, &module);
                    let id = egui::Id::unique(("unused panel module", &module));
                    ui.dnd_drag_source(id, From::Unused(module.clone()), |ui| {
                        piece(ui, &module, &palette)
                    });
                }
            });
        });
        if let (Some(from), None) = (dropped, &done) {
            done = Some(((*from).clone(), To::Off));
        }
    });
    if let Some((from, to)) = done {
        let after = moved(&rows, &from, to);
        for ((side, _), (before, now)) in SIDES.iter().zip(rows.iter().zip(after.iter())) {
            if before != now {
                document.set(
                    &format!("{PANEL}/{side}"),
                    Value::Array(now.iter().cloned().map(Value::String).collect()),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(left: &[&str], centre: &[&str], right: &[&str]) -> [Vec<String>; 3] {
        [left, centre, right].map(|side| side.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn a_module_is_moved_within_its_row_and_to_another() {
        let start = rows(&["launcher", "workspaces", "title"], &[], &["clock"]);
        // To the end of its own row.
        assert_eq!(
            moved(&start, &From::Row(0, 0), To::Row(0, 3)),
            rows(&["workspaces", "title", "launcher"], &[], &["clock"])
        );
        // Before the piece in front of it.
        assert_eq!(
            moved(&start, &From::Row(0, 2), To::Row(0, 0)),
            rows(&["title", "launcher", "workspaces"], &[], &["clock"])
        );
        // Put down where it stands: nothing changes.
        assert_eq!(moved(&start, &From::Row(0, 1), To::Row(0, 1)), start);
        assert_eq!(moved(&start, &From::Row(0, 1), To::Row(0, 2)), start);
        // Into the middle, and before the clock.
        assert_eq!(
            moved(&start, &From::Row(0, 2), To::Row(1, 0)),
            rows(&["launcher", "workspaces"], &["title"], &["clock"])
        );
        assert_eq!(
            moved(&start, &From::Row(0, 0), To::Row(2, 0)),
            rows(&["workspaces", "title"], &[], &["launcher", "clock"])
        );
        // Off the panel.
        assert_eq!(
            moved(&start, &From::Row(2, 0), To::Off),
            rows(&["launcher", "workspaces", "title"], &[], &[])
        );
        // A place that is not there changes nothing.
        assert_eq!(moved(&start, &From::Row(1, 0), To::Row(0, 0)), start);
        assert_eq!(moved(&start, &From::Row(7, 0), To::Row(0, 0)), start);
    }

    #[test]
    fn a_module_stands_once_and_a_spacer_as_often_as_wanted() {
        let start = rows(&["workspaces", "spacer"], &[], &["clock"]);
        // One that is not on the panel comes onto it.
        assert_eq!(
            moved(&start, &From::Unused("sound".to_string()), To::Row(2, 0)),
            rows(&["workspaces", "spacer"], &[], &["sound", "clock"])
        );
        // Another spacer beside the first.
        assert_eq!(
            moved(&start, &From::Unused("spacer".to_string()), To::Row(2, 1)),
            rows(&["workspaces", "spacer"], &[], &["clock", "spacer"])
        );
        // One that stands already is not there twice.
        assert_eq!(
            moved(&start, &From::Unused("clock".to_string()), To::Row(0, 0)),
            rows(&["clock", "workspaces", "spacer"], &[], &[])
        );
        let free = unused(&start);
        assert!(
            free.contains(&"spacer".to_string()),
            "a spacer can always be added"
        );
        assert!(free.contains(&"sound".to_string()));
        assert!(!free.contains(&"clock".to_string()) && !free.contains(&"workspaces".to_string()));
    }
}
