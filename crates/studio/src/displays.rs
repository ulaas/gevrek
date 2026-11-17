//! The displays, drawn where they are: drag one to put it elsewhere.
//!
//! The picture is of the outputs the window manager has now, at the places
//! the configuration in the window gives them (a display without an entry
//! is where the window manager put it). Dragging a display writes the
//! place of every display into `desktop.outputs`, so what is drawn is what
//! is applied; a display snaps to the edges of the others, because
//! displays that do not touch leave a strip the pointer cannot cross.

use crate::document::Document;
use egui::{Color32, Ui};
use gevrek_ui::theme;
use serde_json::Value;

const LIST: &str = "/desktop/outputs";
/// How near (in the displays' own pixels) an edge pulls another to it.
const SNAP: i32 = 48;

/// A display: its connector, where it is and how large (logical pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Default)]
pub struct Arrangement {
    /// The display being dragged: its place in the list, and where it was
    /// taken hold of (in the displays' own pixels, from its corner).
    dragged: Option<(usize, egui::Vec2)>,
    /// Where the displays are while one is dragged.
    moving: Vec<Display>,
    /// The picture's scale and where its 0, 0 is on the stage, as they
    /// were when the drag began: a picture that fitted itself anew while
    /// a display is moved would move under the hand.
    held_view: Option<(f32, egui::Vec2)>,
}

/// The displays the window manager reports, from its state.
pub fn displays_of(state: &Value) -> Vec<Display> {
    state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|monitor| {
            let connector = monitor["connector"]
                .as_str()
                // (On Windows the window manager tells a device's path
                // there, a line of marks nobody reads: the display's name
                // is what a person, and `desktop.outputs`, knows it by.)
                .filter(|name| !name.is_empty() && !name.starts_with(r"\\"));
            Some(Display {
                name: connector.or_else(|| monitor["name"].as_str())?.to_string(),
                x: monitor["position"]["x"].as_f64()?.round() as i32,
                y: monitor["position"]["y"].as_f64()?.round() as i32,
                width: monitor["size"]["width"].as_i64()? as i32,
                height: monitor["size"]["height"].as_i64()? as i32,
            })
        })
        .collect()
}

/// The displays at the places the configuration gives them, where it
/// gives one.
pub fn placed(mut displays: Vec<Display>, outputs: Option<&Value>) -> Vec<Display> {
    for display in &mut displays {
        let entry = outputs
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&display.name))
            });
        if let Some(position) = entry.and_then(|entry| entry["position"].as_array()) {
            if let [x, y] = position.as_slice() {
                display.x = x.as_i64().unwrap_or(0) as i32;
                display.y = y.as_i64().unwrap_or(0) as i32;
            }
        }
    }
    displays
}

/// Where the display at `index` ends up when it is let go at `(x, y)`:
/// pulled to the nearest edges of the others, and never on top of one.
pub fn settle(displays: &[Display], index: usize, x: i32, y: i32) -> (i32, i32) {
    let moved = &displays[index];
    let others: Vec<&Display> = displays
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .map(|(_, d)| d)
        .collect();
    if others.is_empty() {
        return (0, 0);
    }
    let mut best_x = (SNAP + 1, x);
    let mut best_y = (SNAP + 1, y);
    for other in &others {
        // Left and right edges to the other's edges; the same for top and bottom.
        for candidate in [
            other.x - moved.width,
            other.x + other.width,
            other.x,
            other.x + other.width - moved.width,
        ] {
            let distance = (candidate - x).abs();
            if distance < best_x.0 {
                best_x = (distance, candidate);
            }
        }
        for candidate in [
            other.y - moved.height,
            other.y + other.height,
            other.y,
            other.y + other.height - moved.height,
        ] {
            let distance = (candidate - y).abs();
            if distance < best_y.0 {
                best_y = (distance, candidate);
            }
        }
    }
    let (mut x, mut y) = (best_x.1, best_y.1);
    // On top of another: pushed out the shorter way.
    for other in &others {
        let overlap_x = (x + moved.width).min(other.x + other.width) - x.max(other.x);
        let overlap_y = (y + moved.height).min(other.y + other.height) - y.max(other.y);
        if overlap_x > 0 && overlap_y > 0 {
            if overlap_x <= overlap_y {
                x = if x + moved.width / 2 < other.x + other.width / 2 {
                    other.x - moved.width
                } else {
                    other.x + other.width
                };
            } else {
                y = if y + moved.height / 2 < other.y + other.height / 2 {
                    other.y - moved.height
                } else {
                    other.y + other.height
                };
            }
        }
    }
    (x, y)
}

/// Write the place of every display into `desktop.outputs`, the top left
/// of them all at 0, 0. Entries that are there keep their other settings;
/// a display without an entry gets one.
pub fn write_places(document: &mut Document, displays: &[Display]) {
    let left = displays.iter().map(|display| display.x).min().unwrap_or(0);
    let top = displays.iter().map(|display| display.y).min().unwrap_or(0);
    let displays = displays.to_vec();
    document.edit(LIST, |list| {
        let Value::Array(entries) = list else {
            return;
        };
        for display in &displays {
            let place = serde_json::json!([display.x - left, display.y - top]);
            let entry = entries.iter_mut().find(|entry| {
                entry["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&display.name))
            });
            match entry {
                Some(entry) => entry["position"] = place,
                None => entries.push(serde_json::json!({"name": display.name, "position": place})),
            }
        }
    });
}

/// One mode a display offers, as the window manager reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub width: i64,
    pub height: i64,
    /// In hertz.
    pub refresh: f64,
    pub preferred: bool,
    pub current: bool,
}

impl Mode {
    pub fn words(&self) -> String {
        format!(
            "{} × {}  at {} Hz",
            self.width,
            self.height,
            (self.refresh * 100.0).round() / 100.0
        )
    }
}

/// The modes of the display `name`, the largest first. Empty where the
/// window manager does not set modes (it then reports none).
pub fn modes_of(state: &Value, name: &str) -> Vec<Mode> {
    state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|monitor| {
            monitor["connector"].as_str() == Some(name) || monitor["name"].as_str() == Some(name)
        })
        .and_then(|monitor| monitor["modes"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|mode| {
            Some(Mode {
                width: mode["width"].as_i64()?,
                height: mode["height"].as_i64()?,
                refresh: mode["refresh_millihertz"].as_f64()? / 1000.0,
                preferred: mode["preferred"].as_bool().unwrap_or(false),
                current: mode["current"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// Change the entry of the display `name` in `desktop.outputs`, making
/// one when it has none.
pub fn edit_entry(document: &mut Document, name: &str, change: impl FnOnce(&mut Value)) {
    let name = name.to_string();
    document.edit(LIST, |list| {
        let Value::Array(entries) = list else {
            return;
        };
        let at = entries.iter().position(|entry| {
            entry["name"]
                .as_str()
                .is_some_and(|known| known.eq_ignore_ascii_case(&name))
        });
        let at = at.unwrap_or_else(|| {
            entries.push(serde_json::json!({ "name": name }));
            entries.len() - 1
        });
        change(&mut entries[at]);
    });
}

/// The displays that are written down and not among `attached` (not
/// attached now, or all of them where no window manager says which are):
/// each with all of its settings, and a way to take it out. This is the
/// one place a display is set that the window manager does not have now.
fn written_down(ui: &mut Ui, document: &mut Document, attached: &[String], running: bool) {
    let palette = theme::pal(ui.ctx());
    let entries: Vec<(usize, String)> = document
        .get(LIST)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, entry)| (index, entry["name"].as_str().unwrap_or("").to_string()))
        .filter(|(_, name)| !attached.iter().any(|known| known.eq_ignore_ascii_case(name)))
        .collect();
    if !running {
        ui.label(
            egui::RichText::new(
                "The window manager is not running: its displays, and the modes they offer, are not known here.",
            )
            .color(palette.muted),
        );
    }
    if entries.is_empty() {
        return;
    }
    ui.add_space(8.0);
    gevrek_ui::widgets::caption(
        ui,
        if running {
            "Written down, not attached now"
        } else {
            "Written down"
        },
    );
    let mut remove = None;
    for (index, name) in entries {
        let title = if name.trim().is_empty() {
            "A display without a name".to_string()
        } else {
            name
        };
        egui::CollapsingHeader::new(title)
            .id_salt(("a display written down", index))
            .show(ui, |ui| {
                crate::form::Form::new(document).fields(ui, &format!("{LIST}/{index}"), &[]);
                if gevrek_ui::widgets::text_button(ui, gevrek_ui::icons::TRASH, "Take it out")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
    }
    if let Some(index) = remove {
        document.edit(LIST, |list| {
            if let Value::Array(entries) = list {
                if index < entries.len() {
                    entries.remove(index);
                }
            }
        });
    }
}

/// The settings of each display the window manager has: its mode from
/// the ones it offers, its scale, whether it is used, how it is turned;
/// and below them the displays that are written down and not attached.
/// Every setting of a display is here and nowhere else on the page.
pub fn settings(ui: &mut Ui, document: &mut Document, state: Option<&Value>) {
    let palette = theme::pal(ui.ctx());
    let Some(state) = state else {
        written_down(ui, document, &[], false);
        return;
    };
    let attached: Vec<String> = displays_of(state)
        .into_iter()
        .map(|display| display.name)
        .collect();
    for display in displays_of(state) {
        let entry = document
            .get(LIST)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&display.name))
            })
            .cloned()
            .unwrap_or(Value::Null);
        ui.push_id(&display.name, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&display.name).strong());
                ui.add_space(8.0);
                // The mode: one of those the display offers.
                let modes = modes_of(state, &display.name);
                if modes.is_empty() {
                    ui.label(egui::RichText::new("its modes are not reported here").color(palette.faint));
                } else {
                    let set = &entry["mode"];
                    let chosen = modes.iter().find(|mode| {
                        set["width"].as_i64() == Some(mode.width)
                            && set["height"].as_i64() == Some(mode.height)
                            && (set["refresh"].as_f64().unwrap_or(0.0) == 0.0 || (set["refresh"].as_f64().unwrap_or(0.0) - mode.refresh).abs() < 0.6)
                    });
                    let shown = match (set.is_null(), chosen) {
                        (true, _) => "The display's own choice".to_string(),
                        (false, Some(mode)) => mode.words(),
                        (false, None) => format!("{} × {} (not offered)", set["width"], set["height"]),
                    };
                    let mut pick: Option<Option<Mode>> = None;
                    egui::ComboBox::from_id_salt("mode").selected_text(shown).width(230.0).show_ui(ui, |ui| {
                        if ui.selectable_label(set.is_null(), "The display's own choice").clicked() {
                            pick = Some(None);
                        }
                        for mode in &modes {
                            let mut words = mode.words();
                            if mode.preferred {
                                words.push_str("  (preferred)");
                            }
                            if ui.selectable_label(chosen == Some(mode), words).clicked() {
                                pick = Some(Some(*mode));
                            }
                        }
                    });
                    if let Some(pick) = pick {
                        edit_entry(document, &display.name, |entry| {
                            entry["mode"] = match pick {
                                Some(mode) => serde_json::json!({"width": mode.width, "height": mode.height, "refresh": mode.refresh}),
                                None => Value::Null,
                            };
                        });
                    }
                }
                // The scale.
                let scale = entry["scale"].as_f64();
                let options = [(1.0, "1×"), (1.25, "1.25×"), (1.5, "1.5×"), (2.0, "2×")];
                let current = scale.unwrap_or(1.0);
                let selected = options.iter().map(|(value, _)| *value).find(|value| (value - current).abs() < 0.01);
                if let Some(new) = gevrek_ui::widgets::segmented(ui, ui.scope_id().with("scale"), selected.as_ref(), &options, scale.is_none()) {
                    edit_entry(document, &display.name, |entry| entry["scale"] = Value::from(new));
                }
                // Whether it is used at all.
                let mut used = entry["enabled"].as_bool().unwrap_or(true);
                if gevrek_ui::widgets::switch(ui, &mut used).on_hover_text("Use this display").changed() {
                    edit_entry(document, &display.name, |entry| entry["enabled"] = Value::Bool(used));
                }
            });
            // What is set more rarely, under it: how it is turned,
            // whether its refresh rate follows the program that fills
            // it, and whether it is the first of the displays.
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                let turned = entry["transform"].as_str().unwrap_or("normal").to_string();
                let ways: Vec<(String, String, String)> = [
                    ("normal", "Upright"),
                    ("right", "Turned to the right"),
                    ("upside_down", "Upside down"),
                    ("left", "Turned to the left"),
                ]
                .into_iter()
                .map(|(value, name)| (value.to_string(), name.to_string(), String::new()))
                .collect();
                if let Some(new) = gevrek_ui::widgets::dropdown(ui, "turned", &turned, &ways, 170.0) {
                    edit_entry(document, &display.name, |entry| {
                        entry["transform"] = Value::String(new)
                    });
                }
                let mut follows = entry["variable_refresh"].as_bool().unwrap_or(false);
                if gevrek_ui::widgets::checkbox(ui, &mut follows, "Variable refresh")
                    .on_hover_text(
                        "The refresh rate follows the program that fills the display, where the display can.",
                    )
                    .changed()
                {
                    edit_entry(document, &display.name, |entry| {
                        entry["variable_refresh"] = Value::Bool(follows)
                    });
                }
                let mut primary = entry["primary"].as_bool().unwrap_or(false);
                if gevrek_ui::widgets::checkbox(ui, &mut primary, "Primary")
                    .on_hover_text("New windows and the first workspace go to it.")
                    .changed()
                {
                    // (One display is that: the others are not.)
                    if primary {
                        document.edit(LIST, |list| {
                            for other in list.as_array_mut().into_iter().flatten() {
                                if other.get("primary").is_some() {
                                    other["primary"] = Value::Bool(false);
                                }
                            }
                        });
                    }
                    edit_entry(document, &display.name, |entry| {
                        entry["primary"] = Value::Bool(primary)
                    });
                }
            });
        });
    }
    written_down(ui, document, &attached, true);
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("A new mode, or a display switched off, is on trial when applied: keep it within 15 seconds or it goes back.")
            .color(palette.muted)
            .size(12.5),
    );
}

impl Arrangement {
    pub fn show(&mut self, ui: &mut Ui, document: &mut Document, state: Option<&Value>) {
        let palette = theme::pal(ui.ctx());
        let reported = state.map(displays_of).unwrap_or_default();
        if reported.is_empty() {
            ui.label(
                egui::RichText::new(
                    "The displays are drawn here when a window manager is connected.",
                )
                .color(palette.muted),
            );
            return;
        }
        let displays = if self.dragged.is_some() && self.moving.len() == reported.len() {
            self.moving.clone()
        } else {
            placed(reported, document.get(LIST))
        };
        // Fit all of them on the stage, with room to drag one around.
        let left = displays.iter().map(|display| display.x).min().unwrap_or(0) as f32;
        let top = displays.iter().map(|display| display.y).min().unwrap_or(0) as f32;
        let right = displays
            .iter()
            .map(|display| display.x + display.width)
            .max()
            .unwrap_or(1) as f32;
        let bottom = displays
            .iter()
            .map(|display| display.y + display.height)
            .max()
            .unwrap_or(1) as f32;
        let width = 640.0_f32.min(ui.available_width());
        let height = 280.0;
        let scale = ((width - 120.0) / (right - left).max(1.0))
            .min((height - 80.0) / (bottom - top).max(1.0));
        let (stage, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let painter = ui.painter_at(stage);
        let desktop = if palette.dark {
            Color32::from_rgb(30, 31, 40)
        } else {
            Color32::from_rgb(214, 218, 228)
        };
        painter.rect_filled(stage, 8.0, desktop);
        let fitted =
            stage.center() - egui::vec2((left + right) / 2.0, (top + bottom) / 2.0) * scale;
        let (scale, origin) = match self.held_view.filter(|_| self.dragged.is_some()) {
            Some((scale, from_corner)) => (scale, stage.min + from_corner),
            None => (scale, fitted),
        };
        let on_stage = |display: &Display| {
            egui::Rect::from_min_size(
                origin + egui::vec2(display.x as f32, display.y as f32) * scale,
                egui::vec2(display.width as f32, display.height as f32) * scale,
            )
        };
        let mut let_go = false;
        for (index, display) in displays.iter().enumerate() {
            let rect = on_stage(display);
            let response = ui.interact(
                rect,
                ui.scope_id().with(("display", index)),
                egui::Sense::click_and_drag(),
            );
            let held = self.dragged.is_some_and(|(dragged, _)| dragged == index);
            let fill = if held {
                theme::mix(desktop, palette.accent, 0.35)
            } else {
                theme::mix(desktop, palette.text, 0.12)
            };
            painter.rect_filled(rect.shrink(1.0), 4.0, fill);
            painter.rect_stroke(
                rect.shrink(1.0),
                4.0,
                egui::Stroke::new(
                    1.5,
                    if held || response.hovered() {
                        palette.accent_fg()
                    } else {
                        palette.border_strong()
                    },
                ),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center() - egui::vec2(0.0, 8.0),
                egui::Align2::CENTER_CENTER,
                &display.name,
                egui::FontId::proportional(14.0),
                palette.text,
            );
            painter.text(
                rect.center() + egui::vec2(0.0, 10.0),
                egui::Align2::CENTER_CENTER,
                format!(
                    "{}×{}  at {}, {}",
                    display.width, display.height, display.x, display.y
                ),
                egui::FontId::proportional(11.5),
                palette.muted,
            );
            if response.drag_started() && displays.len() > 1 {
                if let Some(pointer) = response.interact_pointer_pos() {
                    self.dragged = Some((index, (pointer - rect.min) / scale));
                    self.moving = displays.clone();
                    self.held_view = Some((scale, origin - stage.min));
                }
            }
            if response.hovered() && displays.len() > 1 {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if held {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                if let (Some(pointer), Some((_, grip))) =
                    (response.interact_pointer_pos(), self.dragged)
                {
                    let at = (pointer - origin) / scale - grip;
                    if let Some(moving) = self.moving.get_mut(index) {
                        moving.x = at.x.round() as i32;
                        moving.y = at.y.round() as i32;
                    }
                }
                if response.drag_stopped() {
                    let_go = true;
                }
            }
        }
        if let_go {
            self.held_view = None;
            if let Some((index, _)) = self.dragged.take() {
                let mut settled = std::mem::take(&mut self.moving);
                if index < settled.len() {
                    let (x, y) = settle(&settled, index, settled[index].x, settled[index].y);
                    settled[index].x = x;
                    settled[index].y = y;
                    write_places(document, &settled);
                }
            }
        }
        ui.add_space(4.0);
        let hint = if displays.len() > 1 {
            "Drag a display to where it stands. Apply to make it so."
        } else {
            "One display is connected."
        };
        ui.label(egui::RichText::new(hint).color(palette.muted));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::Config;
    use serde_json::json;

    fn two() -> Vec<Display> {
        vec![
            Display {
                name: "DP-1".to_string(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
            },
            Display {
                name: "DP-2".to_string(),
                x: 2560,
                y: 0,
                width: 1920,
                height: 1080,
            },
        ]
    }

    #[test]
    fn the_window_managers_displays_are_read_from_its_state() {
        let state = json!({"monitors": [
            {"name": "gevrek-1", "connector": "DP-1", "position": {"x": 0.0, "y": 0.0}, "size": {"width": 2560, "height": 1440}},
            {"name": "HEADLESS-2", "connector": "", "position": {"x": 2560.0, "y": 0.0}, "size": {"width": 1920, "height": 1080}},
        ]});
        let displays = displays_of(&state);
        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].name, "DP-1");
        assert_eq!(displays[1].name, "HEADLESS-2");
        assert_eq!((displays[1].x, displays[1].width), (2560, 1920));
        // The configuration's place wins over where the display is now.
        let outputs = json!([{"name": "dp-1", "position": [1920, 0]}]);
        let placed = placed(displays, Some(&outputs));
        assert_eq!((placed[0].x, placed[0].y), (1920, 0));
        assert_eq!(placed[1].x, 2560);
    }

    #[test]
    fn a_displays_modes_are_read_and_one_is_written_into_its_entry() {
        let state = json!({"monitors": [{"name": "x", "connector": "DP-1", "position": {"x": 0.0, "y": 0.0},
            "size": {"width": 2560, "height": 1440}, "modes": [
                {"width": 2560, "height": 1440, "refresh_millihertz": 144000, "preferred": true, "current": true},
                {"width": 1920, "height": 1080, "refresh_millihertz": 60000, "preferred": false, "current": false}]}]});
        let modes = modes_of(&state, "DP-1");
        assert_eq!(modes.len(), 2);
        assert!(modes[0].preferred && modes[0].current);
        assert_eq!(modes[1].words(), "1920 × 1080  at 60 Hz");
        assert!(modes_of(&state, "DP-9").is_empty());

        let mut document = Document::new(&Config::default());
        edit_entry(&mut document, "DP-1", |entry| {
            entry["mode"] = json!({"width": 1920, "height": 1080, "refresh": 60.0})
        });
        edit_entry(&mut document, "dp-1", |entry| entry["scale"] = json!(1.5));
        let outputs = &document.config().expect("valid").desktop.outputs;
        assert_eq!(outputs.len(), 1, "the same display has one entry");
        let mode = outputs[0].mode.expect("a mode");
        assert_eq!((mode.width, mode.height), (1920, 1080));
        assert_eq!(outputs[0].scale, Some(1.5));
    }

    #[test]
    fn a_display_let_go_near_an_edge_snaps_to_it() {
        let displays = two();
        // The second, dropped a little off the left edge of the first.
        assert_eq!(settle(&displays, 1, -1900, 20), (-1920, 0));
        // Below it, lined up with its left side.
        assert_eq!(settle(&displays, 1, 30, 1460), (0, 1440));
        // Far from every edge in y: it stays where it was dropped.
        assert_eq!(settle(&displays, 1, 2570, 600).0, 2560);
        assert_eq!(settle(&displays, 1, 2570, 600).1, 600);
    }

    #[test]
    fn a_display_is_never_left_on_top_of_another() {
        let displays = two();
        let (x, y) = settle(&displays, 1, 400, 500);
        let overlap_x = (x + 1920).min(2560) - x.max(0);
        let overlap_y = (y + 1080).min(1440) - y.max(0);
        assert!(overlap_x <= 0 || overlap_y <= 0, "let go at {x}, {y}");
    }

    #[test]
    fn places_are_written_from_the_top_left_and_keep_other_settings() {
        let mut config = Config::default();
        config.desktop.outputs =
            serde_json::from_value(json!([{"name": "DP-2", "scale": 1.5}])).expect("outputs");
        let mut document = Document::new(&config);
        let mut displays = two();
        // The second to the left of the first.
        displays[1].x = -1920;
        write_places(&mut document, &displays);
        let outputs = &document.config().expect("valid").desktop.outputs;
        assert_eq!(outputs.len(), 2);
        let second = outputs
            .iter()
            .find(|output| output.name == "DP-2")
            .expect("DP-2");
        assert_eq!(second.position, Some((0, 0)));
        assert_eq!(second.scale, Some(1.5));
        let first = outputs
            .iter()
            .find(|output| output.name == "DP-1")
            .expect("DP-1");
        assert_eq!(first.position, Some((1920, 0)));
        // One step to take back.
        assert!(document.undo());
        assert_eq!(document.config().expect("valid").desktop.outputs.len(), 1);
    }
}
