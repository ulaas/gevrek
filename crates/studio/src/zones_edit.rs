//! The snap zones, edited by dragging: on the page of the zones.
//!
//! A display drawn small with the zones of a layout on it. A zone is
//! taken by its middle and moved, or by an edge or a corner and made
//! wider or narrower; what is dragged catches on the display's edges and
//! on the other zones' edges, so that zones that are meant to meet do.
//! Zones are added and taken out here, a layout begun from one of the
//! ones gevrek knows, and a display given a layout of its own. The zones'
//! places are set here and nowhere else: the form below has only how
//! they behave.

use crate::document::Document;
use egui::{RichText, Ui};
use gevrek_ui::{icons, theme};
use serde_json::{json, Value};

const ZONES: &str = "/desktop/zones";
/// The least share of a display a zone is wide or high.
const LEAST: f32 = 0.05;
/// How near (in shares of the display) an edge catches on another.
const CATCHES: f32 = 0.012;
/// How near an edge the pointer takes the edge and not the zone (points).
const EDGE: f32 = 7.0;

/// A zone's place in shares of the display: left, top, width, height.
pub type Place = [f32; 4];

/// What of a zone is held: all of it, or these of its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Held {
    /// No edge: the zone itself, to be moved.
    pub fn whole(self) -> bool {
        !(self.left || self.right || self.top || self.bottom)
    }

    fn cursor(self) -> egui::CursorIcon {
        match (self.left || self.right, self.top || self.bottom) {
            (true, true) if self.left == self.top => egui::CursorIcon::ResizeNwSe,
            (true, true) => egui::CursorIcon::ResizeNeSw,
            (true, false) => egui::CursorIcon::ResizeHorizontal,
            (false, true) => egui::CursorIcon::ResizeVertical,
            (false, false) => egui::CursorIcon::Grab,
        }
    }
}

/// What of a zone drawn at `rect` the pointer at `at` holds.
pub fn held(rect: egui::Rect, at: egui::Pos2) -> Held {
    // (A small zone keeps a middle to be moved by.)
    let edge_x = EDGE.min(rect.width() / 4.0);
    let edge_y = EDGE.min(rect.height() / 4.0);
    Held {
        left: at.x - rect.left() < edge_x,
        right: rect.right() - at.x < edge_x,
        top: at.y - rect.top() < edge_y,
        bottom: rect.bottom() - at.y < edge_y,
    }
}

/// A place along one axis, caught on the nearest of `edges` where one is
/// near, else on the nearest two hundredth.
fn caught(value: f32, edges: &[f32]) -> f32 {
    let nearest = edges
        .iter()
        .copied()
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()));
    match nearest {
        Some(edge) if (edge - value).abs() <= CATCHES => edge,
        _ => (value * 200.0).round() / 200.0,
    }
}

/// Where a zone that began at `start` is after what is held of it was
/// dragged by `by` (shares of the display), among `others`. It stays on
/// the display and never gets smaller than the least.
pub fn dragged(start: Place, held: Held, by: (f32, f32), others: &[Place]) -> Place {
    let mut across = vec![0.0, 1.0];
    let mut down = vec![0.0, 1.0];
    for other in others {
        across.extend([other[0], other[0] + other[2]]);
        down.extend([other[1], other[1] + other[3]]);
    }
    // One axis: where it begins and how long it is.
    let axis = |begin: f32, long: f32, by: f32, near: bool, far: bool, edges: &[f32]| {
        let end = begin + long;
        if !near && !far {
            if !held.whole() {
                return (begin, long);
            }
            // Moved: the side that catches takes the other with it.
            let moved = (begin + by).clamp(0.0, (1.0 - long).max(0.0));
            let by_begin = caught(moved, edges);
            let by_end = caught(moved + long, edges) - long;
            let exact = |value: f32| edges.iter().any(|edge| (edge - value).abs() < 1e-6);
            let begin = if exact(by_begin) || !exact(by_end + long) {
                by_begin
            } else {
                by_end
            };
            return (begin.clamp(0.0, (1.0 - long).max(0.0)), long);
        }
        if near {
            let begin = caught((begin + by).clamp(0.0, end - LEAST), edges).clamp(0.0, end - LEAST);
            (begin, end - begin)
        } else {
            let end = caught((end + by).clamp(begin + LEAST, 1.0), edges).clamp(begin + LEAST, 1.0);
            (begin, end - begin)
        }
    };
    let (x, width) = axis(start[0], start[2], by.0, held.left, held.right, &across);
    let (y, height) = axis(start[1], start[3], by.1, held.top, held.bottom, &down);
    let tidy = |value: f32| (value * 1000.0).round() / 1000.0;
    [tidy(x), tidy(y), tidy(width), tidy(height)]
}

fn place_of(zone: &Value) -> Place {
    let number = |key: &str| zone[key].as_f64().unwrap_or(0.0) as f32;
    [
        number("x_percent"),
        number("y_percent"),
        number("width_percent"),
        number("height_percent"),
    ]
}

fn put(zone: &mut Value, place: Place) {
    for (key, value) in ["x_percent", "y_percent", "width_percent", "height_percent"]
        .iter()
        .zip(place)
    {
        // (Through text: a share written as 0.35, not 0.3499999940395355.)
        zone[*key] = json!(format!("{value:.3}").parse::<f64>().unwrap_or(0.0));
    }
}

/// A zone to add to these: the first number no zone has, a quarter of
/// the display in its middle.
pub fn another(zones: &[Value]) -> Value {
    let id = (0u64..)
        .find(|id| !zones.iter().any(|zone| zone["id"].as_u64() == Some(*id)))
        .unwrap_or(0);
    let zone = gevrek_model::zones::Zone::new(id as usize, 0.25, 0.25, 0.5, 0.5);
    serde_json::to_value(zone).unwrap_or(Value::Null)
}

struct Drag {
    zone: usize,
    held: Held,
    start: Place,
    from: egui::Pos2,
}

/// What the card remembers between frames.
#[derive(Default)]
pub struct ZonesEdit {
    /// 0: the layout of every display; n: the n-th display's own.
    which: usize,
    chosen: Option<usize>,
    drag: Option<Drag>,
}

impl ZonesEdit {
    /// Where the layout that is edited stands in the configuration.
    fn layout(&self) -> String {
        match self.which {
            0 => format!("{ZONES}/default_layout"),
            own => format!("{ZONES}/monitor_layouts/{}/layout", own - 1),
        }
    }

    pub fn show(&mut self, ui: &mut Ui, document: &mut Document) {
        let palette = theme::pal(ui.ctx());
        let own: Vec<String> = document
            .get(&format!("{ZONES}/monitor_layouts"))
            .and_then(Value::as_array)
            .map(|layouts| {
                layouts
                    .iter()
                    .map(|layout| match &layout["monitor"] {
                        Value::String(name) => name.clone(),
                        other => other.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self.which > own.len() {
            self.which = 0;
        }
        let name_of = |which: usize| match which {
            0 => "Every display".to_string(),
            n => format!("The display {}", own[n - 1]),
        };
        let layout = self.layout();
        ui.horizontal(|ui| {
            if !own.is_empty() {
                egui::ComboBox::from_id_salt("the zones of which display")
                    .selected_text(name_of(self.which))
                    .show_ui(ui, |ui| {
                        for which in 0..=own.len() {
                            if ui
                                .selectable_label(self.which == which, name_of(which))
                                .clicked()
                            {
                                self.which = which;
                                self.chosen = None;
                                self.drag = None;
                            }
                        }
                    });
            }
            // Begun from a layout gevrek knows.
            let called = document
                .get(&format!("{layout}/name"))
                .and_then(Value::as_str)
                .unwrap_or("A layout")
                .to_string();
            let mut begun: Option<Value> = None;
            egui::ComboBox::from_id_salt("zones begun from")
                .selected_text(called)
                .show_ui(ui, |ui| {
                    for preset in gevrek_model::zones::presets::all() {
                        if ui
                            .selectable_label(false, &preset.name)
                            .on_hover_text(&preset.description)
                            .clicked()
                        {
                            begun = serde_json::to_value(&preset).ok();
                        }
                    }
                });
            if let Some(begun) = begun {
                document.set(&layout, begun);
                self.chosen = None;
                self.drag = None;
            }
        });
        let layout = self.layout();
        let zones_at = format!("{layout}/zones");
        let zones: Vec<Value> = document
            .get(&zones_at)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if self.chosen.is_some_and(|chosen| chosen >= zones.len()) {
            self.chosen = None;
        }

        // The display, drawn small.
        let width = ui.available_width().clamp(240.0, 560.0);
        let (pad, response) = ui.allocate_exact_size(
            egui::vec2(width, width * 9.0 / 16.0),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(pad);
        painter.rect_filled(pad, 6.0, theme::mix(palette.surface, palette.text, 0.06));
        let drawn = |place: Place| {
            egui::Rect::from_min_size(
                pad.min + egui::vec2(place[0] * pad.width(), place[1] * pad.height()),
                egui::vec2(place[2] * pad.width(), place[3] * pad.height()),
            )
        };
        // The zone under the pointer: of those that overlap, the one that
        // is chosen, else the one with the higher priority, else the later.
        let was_chosen = self.chosen;
        let under = |at: egui::Pos2| -> Option<usize> {
            let mut under: Vec<usize> = (0..zones.len())
                .filter(|index| drawn(place_of(&zones[*index])).contains(at))
                .collect();
            under.sort_by_key(|index| {
                (
                    was_chosen == Some(*index),
                    zones[*index]["priority"].as_i64().unwrap_or(0),
                    *index,
                )
            });
            under.last().copied()
        };
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        if response.drag_started() {
            if let Some(at) = pointer {
                self.chosen = under(at);
                self.drag = self.chosen.map(|zone| Drag {
                    zone,
                    held: held(drawn(place_of(&zones[zone])), at),
                    start: place_of(&zones[zone]),
                    from: at,
                });
            }
        } else if response.clicked() {
            self.chosen = pointer.and_then(under);
        }
        if !response.dragged() && !response.drag_started() {
            self.drag = None;
        }
        let mut shown = zones.clone();
        if let (Some(drag), Some(at)) = (&self.drag, pointer) {
            if let Some(zone) = shown.get_mut(drag.zone) {
                let others: Vec<Place> = zones
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != drag.zone)
                    .map(|(_, zone)| place_of(zone))
                    .collect();
                let by = (
                    (at.x - drag.from.x) / pad.width(),
                    (at.y - drag.from.y) / pad.height(),
                );
                put(zone, dragged(drag.start, drag.held, by, &others));
                // (One place in the document while it is dragged: one
                // step to take back.)
                document.set(&format!("{zones_at}/{}", drag.zone), zone.clone());
            }
            ui.ctx().set_cursor_icon(if drag.held.whole() {
                egui::CursorIcon::Grabbing
            } else {
                drag.held.cursor()
            });
        } else if let Some(at) = response.hover_pos() {
            if let Some(index) = under(at) {
                ui.ctx()
                    .set_cursor_icon(held(drawn(place_of(&zones[index])), at).cursor());
            }
        }
        for (index, zone) in shown.iter().enumerate() {
            let rect = drawn(place_of(zone)).shrink(2.0);
            let chosen = self.chosen == Some(index);
            painter.rect_filled(
                rect,
                5.0,
                palette
                    .accent
                    .gamma_multiply(if chosen { 0.34 } else { 0.16 }),
            );
            painter.rect_stroke(
                rect,
                5.0,
                egui::Stroke::new(
                    if chosen { 2.0 } else { 1.0 },
                    if chosen {
                        palette.accent
                    } else {
                        palette.border
                    },
                ),
                egui::StrokeKind::Inside,
            );
            let place = place_of(zone);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!(
                    "{}\n{:.0} × {:.0} %",
                    zone["name"].as_str().unwrap_or_default(),
                    place[2] * 100.0,
                    place[3] * 100.0
                ),
                egui::FontId::proportional(12.0),
                palette.text,
            );
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if gevrek_ui::widgets::secondary_button(ui, format!("{}  Add a zone", icons::PLUS)).clicked() {
                let zone = another(&zones);
                document.edit(&zones_at, |zones| {
                    if let Some(zones) = zones.as_array_mut() {
                        zones.push(zone);
                    }
                });
                self.chosen = Some(zones.len());
            }
            // (A layout has one zone at the least.)
            let chosen = self.chosen.filter(|_| zones.len() > 1);
            if ui
                .add_enabled(
                    chosen.is_some(),
                    egui::Button::new(format!("{}  Take it out", icons::TRASH)),
                )
                .clicked()
            {
                if let Some(chosen) = chosen {
                    document.edit(&zones_at, |zones| {
                        if let Some(zones) = zones.as_array_mut() {
                            zones.remove(chosen);
                        }
                    });
                    self.chosen = None;
                }
            }
            if let Some(zone) = self.chosen.and_then(|chosen| zones.get(chosen)) {
                let mut name = zone["name"].as_str().unwrap_or_default().to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut name).desired_width(160.0))
                    .changed()
                {
                    document.set(
                        &format!("{zones_at}/{}/name", self.chosen.unwrap_or(0)),
                        Value::String(name),
                    );
                }
            } else {
                ui.label(
                    RichText::new("Drag a zone to move it, an edge or a corner to size it.")
                        .color(palette.muted),
                );
            }
        });
        // The room around the chosen zone's window.
        if let Some(chosen) = self.chosen.filter(|chosen| *chosen < zones.len()) {
            crate::form::Form::new(document).only(ui, &format!("{zones_at}/{chosen}"), &["gap"]);
        }

        // What a layout is called, and the layouts single displays have
        // of their own: set here too, so that everything of the zones'
        // places is in this card and nowhere else.
        ui.add_space(6.0);
        let mut add_own = false;
        let mut remove_own = false;
        egui::CollapsingHeader::new("This layout's name, and layouts of single displays")
            .id_salt("the zones' layouts")
            .show(ui, |ui| {
                crate::form::Form::new(document).only(ui, &layout, &["name", "description"]);
                if self.which > 0 {
                    crate::form::Form::new(document).only(
                        ui,
                        &format!("{ZONES}/monitor_layouts/{}", self.which - 1),
                        &["enabled", "monitor"],
                    );
                }
                ui.horizontal(|ui| {
                    if gevrek_ui::widgets::text_button(
                        ui,
                        icons::PLUS,
                        "A layout of its own for a display",
                    )
                    .on_hover_text("Begun as a copy of the one shown; then say which display.")
                    .clicked()
                    {
                        add_own = true;
                    }
                    if self.which > 0
                        && gevrek_ui::widgets::text_button(
                            ui,
                            icons::TRASH,
                            "Take this display's own layout out",
                        )
                        .clicked()
                    {
                        remove_own = true;
                    }
                });
            });
        if add_own {
            let copy = document.get(&layout).cloned().unwrap_or(Value::Null);
            let mut own = serde_json::to_value(gevrek_model::settings::MonitorZoneLayout::default())
                .unwrap_or_else(|_| json!({}));
            if !copy.is_null() {
                own["layout"] = copy;
            }
            let mut count = 0;
            document.edit(&format!("{ZONES}/monitor_layouts"), |layouts| {
                if let Some(layouts) = layouts.as_array_mut() {
                    layouts.push(own);
                    count = layouts.len();
                }
            });
            if count > 0 {
                self.which = count;
                self.chosen = None;
                self.drag = None;
            }
        }
        if remove_own {
            let at = self.which - 1;
            document.edit(&format!("{ZONES}/monitor_layouts"), |layouts| {
                if let Some(layouts) = layouts.as_array_mut() {
                    if at < layouts.len() {
                        layouts.remove(at);
                    }
                }
            });
            self.which = 0;
            self.chosen = None;
            self.drag = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE: Held = Held {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };

    #[test]
    fn a_zone_is_held_by_its_middle_or_by_its_edges() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 100.0));
        assert!(held(rect, rect.center()).whole());
        let corner = held(rect, egui::pos2(102.0, 197.0));
        assert!(corner.left && corner.bottom && !corner.right && !corner.top);
        let edge = held(rect, egui::pos2(298.0, 150.0));
        assert!(edge.right && !edge.left && !edge.top && !edge.bottom);
        // A small zone keeps a middle.
        let small = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(12.0, 12.0));
        assert!(held(small, small.center()).whole());
    }

    #[test]
    fn a_zone_moved_stays_on_the_display_and_catches_on_edges() {
        let start = [0.1, 0.1, 0.4, 0.5];
        // Far to the right: its right edge at the display's.
        assert_eq!(dragged(start, WHOLE, (5.0, 0.0), &[]), [0.6, 0.1, 0.4, 0.5]);
        assert_eq!(
            dragged(start, WHOLE, (-5.0, -5.0), &[]),
            [0.0, 0.0, 0.4, 0.5]
        );
        // Near another zone's edge: on it, by the left side or the right.
        let other = [[0.705, 0.0, 0.295, 1.0]];
        assert_eq!(dragged(start, WHOLE, (0.2, 0.0), &other)[0], 0.305);
        let left = [[0.0, 0.0, 0.207, 1.0]];
        assert_eq!(dragged(start, WHOLE, (0.1, 0.0), &left)[0], 0.207);
        // Near nothing: a tidy number; its size is never changed.
        let free = dragged(start, WHOLE, (0.1234, 0.0), &[]);
        assert_eq!(free, [0.225, 0.1, 0.4, 0.5]);
    }

    #[test]
    fn an_edge_dragged_sizes_the_zone_and_no_smaller_than_the_least() {
        let start = [0.2, 0.2, 0.4, 0.4];
        let right = Held {
            right: true,
            ..WHOLE
        };
        assert_eq!(dragged(start, right, (0.1, 0.3), &[]), [0.2, 0.2, 0.5, 0.4]);
        assert_eq!(dragged(start, right, (9.0, 0.0), &[]), [0.2, 0.2, 0.8, 0.4]);
        assert_eq!(
            dragged(start, right, (-9.0, 0.0), &[]),
            [0.2, 0.2, LEAST, 0.4]
        );
        // The left edge: the right one stays where it is.
        let left = Held {
            left: true,
            ..WHOLE
        };
        assert_eq!(dragged(start, left, (-0.1, 0.0), &[]), [0.1, 0.2, 0.5, 0.4]);
        assert_eq!(
            dragged(start, left, (9.0, 0.0), &[]),
            [0.55, 0.2, LEAST, 0.4]
        );
        // A corner: both; it catches on another zone's edge.
        let corner = Held {
            right: true,
            bottom: true,
            ..WHOLE
        };
        let other = [[0.655, 0.0, 0.345, 1.0]];
        assert_eq!(
            dragged(start, corner, (0.05, 0.1), &other),
            [0.2, 0.2, 0.455, 0.5]
        );
    }

    #[test]
    fn a_zone_that_is_added_has_a_number_of_its_own() {
        let zones = vec![json!({"id": 0}), json!({"id": 2})];
        let zone = another(&zones);
        assert_eq!(zone["id"], 1);
        assert_eq!(place_of(&zone), [0.25, 0.25, 0.5, 0.5]);
        let mut zone = zone;
        put(&mut zone, [0.35, 0.0, 0.65, 1.0]);
        assert_eq!(zone["x_percent"], json!(0.35));
        assert_eq!(zone["width_percent"], json!(0.65));
    }
}
