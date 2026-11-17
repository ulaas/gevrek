//! The hot corners, corner by corner (`desktop.hot_corners`).
//!
//! The configuration has a list of entries, each naming a corner. A
//! display has four corners and a corner does one thing, so the card shows
//! the four, each with what it does (the form's own control for a command,
//! as a key binding has it) or with a way to give it something to do. The
//! first entry of a corner is that corner's: it is the one that runs.

use crate::document::Document;
use crate::form::Form;
use egui::{RichText, Ui};
use gevrek_model::settings::{HotCorner, ScreenCorner};
use gevrek_ui::{icons, theme, widgets};
use serde_json::Value;

const PLACE: &str = "/desktop/hot_corners";

const CORNERS: [(ScreenCorner, &str, &str); 4] = [
    (ScreenCorner::TopLeft, "top_left", "Top left"),
    (ScreenCorner::TopRight, "top_right", "Top right"),
    (ScreenCorner::BottomLeft, "bottom_left", "Bottom left"),
    (ScreenCorner::BottomRight, "bottom_right", "Bottom right"),
];

/// Where the entry of a corner is in the list, if it has one.
pub fn entry_of(document: &Document, corner: &str) -> Option<usize> {
    document
        .get(&format!("{PLACE}/corners"))
        .and_then(Value::as_array)?
        .iter()
        .position(|entry| entry["corner"].as_str() == Some(corner))
}

pub fn show(ui: &mut Ui, document: &mut Document) {
    let palette = theme::pal(ui.ctx());
    ui.label(
        RichText::new(
            "Put the pointer into a corner of a display and leave it there for a moment. Every display has these corners.",
        )
        .color(palette.muted),
    );
    Form::new(document).only(ui, PLACE, &["enabled", "delay_ms", "size"]);
    for (corner, key, name) in CORNERS {
        ui.add_space(8.0);
        widgets::caption(ui, name);
        match entry_of(document, key) {
            Some(index) => {
                Form::new(document).only(
                    ui,
                    &format!("{PLACE}/corners/{index}"),
                    &["enabled", "command"],
                );
                if widgets::text_button(ui, icons::TRASH, "Nothing in this corner").clicked() {
                    document.edit(&format!("{PLACE}/corners"), |corners| {
                        if let Some(corners) = corners.as_array_mut() {
                            // (Every entry of the corner: one that came
                            // after the first would run in its place.)
                            corners.retain(|entry| entry["corner"].as_str() != Some(key));
                        }
                    });
                }
            }
            None => {
                if widgets::text_button(ui, icons::PLUS, "Give it something to do").clicked() {
                    let entry = serde_json::to_value(HotCorner {
                        corner,
                        ..HotCorner::default()
                    })
                    .unwrap_or(Value::Null);
                    document.edit(&format!("{PLACE}/corners"), |corners| {
                        if let Some(corners) = corners.as_array_mut() {
                            corners.push(entry);
                        }
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corners_are_named_as_the_configuration_names_them() {
        for (corner, key, _) in CORNERS {
            assert_eq!(serde_json::to_value(corner).unwrap(), Value::from(key));
        }
    }
}
