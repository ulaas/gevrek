//! The key bindings page: a list to find a binding in, and the one that
//! is chosen in full.
//!
//! The list is drawn by hand, because a hundred bindings want to be read
//! at a glance (the keys as key caps, what they do in words, a mark where
//! two bindings fight over the same keys). The chosen binding's fields are
//! the form's, so a command's arguments have their controls without this
//! page knowing any command.

use crate::document::Document;
use crate::form::Form;
use egui::{RichText, Ui};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;
use serde_json::Value;
use std::collections::HashMap;

const LIST: &str = "/desktop/bindings";

#[derive(Default)]
pub struct BindingsPage {
    filter: String,
    /// Only the bindings of this mode (submap); empty: all.
    mode: String,
    chosen: Option<usize>,
    /// The next key pressed becomes the chosen binding's key.
    recording: bool,
    /// While recording, every key is the studio's (`all_keys.rs`): the
    /// modifiers held with the key are recorded too. Without it the
    /// window manager keeps its own bindings, so only the key is
    /// recorded and the modifiers are set by hand.
    pub every_key: bool,
    /// Super is held, by the keys seen while recording.
    super_down: bool,
    /// What was pressed, where the keys do not come through the toolkit
    /// (Windows: they are listened to before the window manager).
    pub heard: Option<Heard>,
}

/// What was pressed while a binding's keys were recorded.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    /// Escape: the binding stays as it is.
    Leave,
    /// The model's name of the key, and the modifiers held with it by
    /// the model's names for them.
    Key(String, [(&'static str, bool); 4]),
}

impl Heard {
    /// A key as Windows numbers it, with the modifiers held. `None` for
    /// a key the model has no name for.
    pub fn of_windows(key: u16, win: bool, ctrl: bool, alt: bool, shift: bool) -> Option<Self> {
        // Escape
        if key == 0x1B {
            return Some(Heard::Leave);
        }
        let name = gevrek_model::keys::vk_to_key_name(key)?;
        Some(Heard::Key(
            name,
            [
                ("super", win),
                ("ctrl", ctrl),
                ("alt", alt),
                ("shift", shift),
            ],
        ))
    }
}

/// What a binding asks to be done outside the page.
pub enum Wish {
    /// Run this command now (JSON).
    Run(String),
}

impl BindingsPage {
    /// Whether the next key is being waited for.
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// No keys are waited for any more, and what was heard is nobody's.
    pub fn stop_recording(&mut self) {
        self.recording = false;
        self.heard = None;
    }

    /// Wait for the keys of this binding (what its button does).
    pub fn record(&mut self, binding: usize) {
        self.chosen = Some(binding);
        self.recording = true;
        self.super_down = false;
        self.heard = None;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        highlight: Option<&str>,
    ) -> Option<Wish> {
        let palette = theme::pal(ui.ctx());
        let bindings: Vec<Value> = document
            .get(LIST)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // A setting found by search opens its binding.
        if let Some(index) = highlight
            .and_then(|pointer| pointer.strip_prefix("/desktop/bindings/"))
            .and_then(|rest| rest.split('/').next())
            .and_then(|index| index.parse::<usize>().ok())
        {
            self.chosen = Some(index);
        }
        if self.chosen.is_some_and(|index| index >= bindings.len()) {
            self.chosen = None;
            self.recording = false;
        }
        let clashes = clashes(&bindings);
        let mut wish = None;

        // What the names of the commands are, from the model.
        let names: Vec<String> = document
            .config()
            .map(|config| {
                config
                    .desktop
                    .bindings
                    .iter()
                    .map(|binding| binding.command.display_name().to_string())
                    .collect()
            })
            .unwrap_or_default();

        let mut modes: Vec<String> = bindings
            .iter()
            .filter_map(|binding| binding["submap"].as_str().map(str::to_string))
            .collect();
        modes.sort();
        modes.dedup();

        widgets::card_frame(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text(format!(
                            "{}  Find by key, name or command",
                            icons::MAGNIFYING_GLASS
                        ))
                        .desired_width(300.0),
                );
                if modes.len() > 1 {
                    egui::ComboBox::from_id_salt("mode")
                        .selected_text(if self.mode.is_empty() {
                            "Every mode".to_string()
                        } else {
                            format!("Mode: {}", self.mode)
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.mode, String::new(), "Every mode");
                            for mode in &modes {
                                ui.selectable_value(&mut self.mode, mode.clone(), mode);
                            }
                        });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary_button(ui, format!("{}  New binding", icons::PLUS))
                        .clicked()
                    {
                        let new = gevrek_schema::schema()
                            .at(LIST)
                            .and_then(|node| node.child("0"))
                            .map(|item| item.blank());
                        if let Some(new) = new {
                            document.edit(LIST, |list| {
                                if let Value::Array(items) = list {
                                    items.push(new);
                                }
                            });
                            self.chosen = Some(bindings.len());
                            self.filter.clear();
                        }
                    }
                    if !clashes.is_empty() {
                        ui.colored_label(
                            palette.warn,
                            format!("{}  {} on the same keys", icons::WARNING, clashes.len()),
                        );
                    }
                    ui.label(
                        RichText::new(format!("{} bindings", bindings.len())).color(palette.muted),
                    );
                });
            });
            ui.add_space(6.0);
            let needle = self.filter.trim().to_lowercase();
            let row_height = ui.spacing().interact_size.y + 6.0;
            let shown: Vec<usize> = bindings
                .iter()
                .enumerate()
                .filter(|(index, binding)| {
                    if !self.mode.is_empty()
                        && binding["submap"].as_str() != Some(self.mode.as_str())
                    {
                        return false;
                    }
                    if needle.is_empty() {
                        return true;
                    }
                    let command = names.get(*index).map(String::as_str).unwrap_or("");
                    let hay = format!(
                        "{} {} {} {}",
                        chord(binding),
                        binding["name"].as_str().unwrap_or(""),
                        command,
                        binding["command"]
                    )
                    .to_lowercase();
                    needle.split_whitespace().all(|word| hay.contains(word))
                })
                .map(|(index, _)| index)
                .collect();
            // As high as the window leaves: down to its lower edge where
            // no binding is open under the list, and the upper part of
            // that room where one is (its own card stands below).
            // (The page does not move: the list does, and under it the
            // card of the binding that is open, each in its own room.)
            let room = ui.available_height();
            let high = if self.chosen.is_some() {
                (room - 440.0).clamp(120.0, (room * 0.5).max(120.0))
            } else {
                (room - 44.0).max(120.0)
            };
            egui::ScrollArea::vertical()
                .max_height(high)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, shown.len(), |ui, rows| {
                    for &index in &shown[rows] {
                        let binding = &bindings[index];
                        let selected = self.chosen == Some(index);
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_height),
                            egui::Sense::click(),
                        );
                        widgets::list_row_bg(ui, &response, rect, selected);
                        let enabled = binding["enabled"].as_bool().unwrap_or(true);
                        let colour = if enabled { palette.text } else { palette.faint };
                        let left = egui::pos2(rect.left() + 8.0, rect.center().y);
                        let end = widgets::keycaps(ui, left, &chord(binding), colour);
                        let name_at =
                            egui::pos2((end + 14.0).max(rect.left() + 230.0), rect.center().y);
                        let painter = ui.painter();
                        let name = binding["name"].as_str().unwrap_or("");
                        let after = painter
                            .text(
                                name_at,
                                egui::Align2::LEFT_CENTER,
                                name,
                                egui::FontId::proportional(14.0),
                                colour,
                            )
                            .right();
                        let command = names.get(index).map(String::as_str).unwrap_or("");
                        if !command.is_empty() && command != name {
                            painter.text(
                                egui::pos2(after + 14.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                command,
                                egui::FontId::proportional(13.0),
                                palette.muted,
                            );
                        }
                        let mut right = rect.right() - 10.0;
                        if clashes.contains_key(&index) {
                            painter.text(
                                egui::pos2(right, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                icons::WARNING,
                                egui::FontId::proportional(15.0),
                                palette.warn,
                            );
                            right -= 24.0;
                        }
                        let mode = binding["submap"].as_str().unwrap_or("default");
                        if mode != "default" {
                            painter.text(
                                egui::pos2(right, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                mode,
                                egui::FontId::proportional(12.0),
                                palette.accent_fg(),
                            );
                        }
                        if response.clicked() {
                            self.chosen = if selected { None } else { Some(index) };
                            self.recording = false;
                        }
                    }
                });
            if shown.is_empty() {
                ui.label(RichText::new("No binding like that.").color(palette.muted));
            }
        });

        let Some(index) = self.chosen else {
            ui.add_space(4.0);
            ui.label(RichText::new("Choose a binding to change it.").color(palette.muted));
            return wish;
        };
        let place = format!("{LIST}/{index}");
        // (A binding made in this very pass is in the document and not in
        // the list this pass drew from: it is shown from the next one on.)
        let Some(binding) = bindings.get(index).cloned() else {
            return wish;
        };
        let mut remove = false;
        let mut copy = false;
        egui::ScrollArea::vertical()
            .id_salt("the binding that is open")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                widgets::card(ui, binding["name"].as_str().unwrap_or("Binding"), |ui| {
                    if let Some(others) = clashes.get(&index) {
                        let names: Vec<&str> = others
                            .iter()
                            .filter_map(|other| bindings[*other]["name"].as_str())
                            .collect();
                        ui.colored_label(
                            palette.warn,
                            format!(
                                "{}  The same keys, in the same mode, also do: {}",
                                icons::WARNING,
                                names.join(", ")
                            ),
                        );
                        ui.add_space(4.0);
                    }
                    // The keys: the modifiers as switches, the key by pressing it.
                    let ((), _) = widgets::row(
                        ui,
                        "Keys",
                        "Hold these, then press the key",
                        false,
                        false,
                        |ui| {
                            for (field, label) in [
                                ("super", "Super"),
                                ("ctrl", "Ctrl"),
                                ("alt", "Alt"),
                                ("shift", "Shift"),
                            ] {
                                let mut held = binding[field].as_bool().unwrap_or(false);
                                if ui.toggle_value(&mut held, label).changed() {
                                    document.set(&format!("{place}/{field}"), Value::Bool(held));
                                }
                            }
                            ui.label("+");
                            let key = binding["key"].as_str().unwrap_or("");
                            let text = if self.recording {
                                "press a key…".to_string()
                            } else if key.is_empty() {
                                "none".to_string()
                            } else {
                                key.to_string()
                            };
                            if ui
                        .add(
                            egui::Button::new(RichText::new(text).monospace())
                                .selected(self.recording),
                        )
                        .on_hover_text(if self.every_key {
                            "Click, then press the keys together, Super and the others with \
                             them (Escape leaves it as it is)"
                        } else {
                            "Click, then press the key (Escape leaves it as it is)"
                        })
                        .clicked()
                    {
                        self.recording = !self.recording;
                        self.super_down = false;
                        self.heard = None;
                    }
                            let mut typed = key.to_string();
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut typed)
                                    .desired_width(110.0)
                                    .hint_text("or its name"),
                            );
                            if field.changed() {
                                document.set(
                                    &format!("{place}/key"),
                                    Value::String(typed.trim().to_uppercase()),
                                );
                            }
                        },
                    );
                    if self.recording {
                        let heard = self
                            .heard
                            .take()
                            .or_else(|| heard_now(ui, &mut self.super_down));
                        if let Some(heard) = heard {
                            if let Heard::Key(name, modifiers) = heard {
                                log::info!(
                                    "recorded for binding {index}: {name} with {modifiers:?}"
                                );
                                document.set(&format!("{place}/key"), Value::String(name));
                                // With every key the studio's, the modifiers
                                // that are held are the binding's too.
                                if self.every_key {
                                    for (field, on) in modifiers {
                                        document.set(&format!("{place}/{field}"), Value::Bool(on));
                                    }
                                }
                            }
                            self.recording = false;
                            self.super_down = false;
                        }
                    }
                    let mut form = Form::new(document);
                    form.highlight = highlight;
                    form.only(ui, &place, &["name", "enabled", "command", "trigger"]);
                    if binding["trigger"].as_str() == Some("long_press") {
                        form.only(ui, &place, &["long_press_ms"]);
                    }
                    form.only(ui, &place, &["submap", "device", "consume"]);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!("{}  Run now", icons::PLAY))
                            .on_hover_text("Do what the binding does, once")
                            .clicked()
                        {
                            wish = Some(Wish::Run(binding["command"].to_string()));
                        }
                        if widgets::secondary_button(ui, format!("{}  Duplicate", icons::COPY))
                            .clicked()
                        {
                            copy = true;
                        }
                        if widgets::secondary_button(ui, format!("{}  Remove", icons::TRASH))
                            .clicked()
                        {
                            remove = true;
                        }
                    });
                });
            });
        if copy {
            let mut twin = binding.clone();
            twin["name"] =
                Value::String(format!("{} (copy)", binding["name"].as_str().unwrap_or("")));
            document.edit(LIST, |list| {
                if let Value::Array(items) = list {
                    items.insert(index + 1, twin);
                }
            });
            self.chosen = Some(index + 1);
        }
        if remove {
            document.edit(LIST, |list| {
                if let Value::Array(items) = list {
                    if index < items.len() {
                        items.remove(index);
                    }
                }
            });
            self.chosen = None;
        }
        wish
    }
}

/// The keys of a binding as they are read: `Super+Shift+J`.
pub fn chord(binding: &Value) -> String {
    let mut parts = Vec::new();
    for (field, label) in [
        ("super", "Super"),
        ("ctrl", "Ctrl"),
        ("alt", "Alt"),
        ("shift", "Shift"),
    ] {
        if binding[field].as_bool().unwrap_or(false) {
            parts.push(label.to_string());
        }
    }
    let key = binding["key"].as_str().unwrap_or("");
    if !key.is_empty() {
        parts.push(nice_key(key));
    }
    parts.join("+")
}

fn nice_key(key: &str) -> String {
    match key {
        "RETURN" | "ENTER" => "Enter".to_string(),
        "ESCAPE" | "ESC" => "Esc".to_string(),
        "SPACE" => "Space".to_string(),
        "SLASH" => "/".to_string(),
        "BACKSLASH" => "\\".to_string(),
        "COMMA" => ",".to_string(),
        "PERIOD" => ".".to_string(),
        "MINUS" => "-".to_string(),
        "EQUALS" => "=".to_string(),
        "SEMICOLON" => ";".to_string(),
        "QUOTE" => "'".to_string(),
        "BACKTICK" => "`".to_string(),
        "LEFTBRACKET" => "[".to_string(),
        "RIGHTBRACKET" => "]".to_string(),
        "LEFT" => "←".to_string(),
        "RIGHT" => "→".to_string(),
        "UP" => "↑".to_string(),
        "DOWN" => "↓".to_string(),
        other if other.len() == 1 => other.to_string(),
        other => {
            // PAGEUP reads as Pageup, VOLUME_UP as Volume up.
            let lower = other.replace('_', " ").to_lowercase();
            let mut letters = lower.chars();
            letters.next().map_or(lower.clone(), |first| {
                first.to_uppercase().chain(letters).collect()
            })
        }
    }
}

/// Bindings that are switched on and share keys, mode, device and
/// trigger with another: each with the others it shares them with.
pub fn clashes(bindings: &[Value]) -> HashMap<usize, Vec<usize>> {
    let mut by_keys: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, binding) in bindings.iter().enumerate() {
        if !binding["enabled"].as_bool().unwrap_or(true)
            || binding["key"].as_str().unwrap_or("").is_empty()
        {
            continue;
        }
        let keys = format!(
            "{}|{}|{}|{}|{}",
            chord(binding).to_uppercase(),
            binding["key"].as_str().unwrap_or("").to_uppercase(),
            binding["submap"].as_str().unwrap_or("default"),
            binding["device"].as_str().unwrap_or("keyboard"),
            binding["trigger"].as_str().unwrap_or("press"),
        );
        by_keys.entry(keys).or_default().push(index);
    }
    let mut found = HashMap::new();
    for group in by_keys.values().filter(|group| group.len() > 1) {
        for index in group {
            found.insert(
                *index,
                group
                    .iter()
                    .copied()
                    .filter(|other| other != index)
                    .collect(),
            );
        }
    }
    found
}

/// What the keyboard says this frame to something that records keys (a
/// binding, a node that starts at a key): the first key that is no
/// modifier with what was held, or Escape. `super_down` is kept by the
/// caller between frames.
pub fn heard_now(ui: &Ui, super_down: &mut bool) -> Option<Heard> {
    let events = ui.input(|input| input.events.clone());
    match pressed_key(&events, super_down)? {
        Pressed::Leave => Some(Heard::Leave),
        Pressed::Key(name, held) => Some(Heard::Key(name, recorded_modifiers(held, *super_down))),
    }
}

/// A key's name, chosen by pressing the key: a button that listens for
/// the next key (Escape leaves the name as it is), and the name itself
/// to type for a key this keyboard has not. For a key's name wherever a
/// form has one; whether it listens is kept by the toolkit under `salt`.
/// Says whether the name was changed.
pub fn key_field(ui: &mut Ui, salt: &str, name: &mut String) -> bool {
    let id = ui.scope_id().with(("a key by pressing", salt));
    let (mut listening, mut super_down): (bool, bool) =
        ui.data(|data| data.get_temp(id)).unwrap_or_default();
    let mut changed = false;
    let text = if listening {
        "press a key…".to_string()
    } else if name.is_empty() {
        "none".to_string()
    } else {
        name.clone()
    };
    if ui
        .add(egui::Button::new(RichText::new(text).monospace()).selected(listening))
        .on_hover_text("Click, then press the key (Escape leaves it as it is)")
        .clicked()
    {
        listening = !listening;
        super_down = false;
    } else if listening {
        match heard_now(ui, &mut super_down) {
            Some(Heard::Key(key, _)) => {
                *name = key;
                changed = true;
                listening = false;
            }
            Some(Heard::Leave) => listening = false,
            None => {}
        }
    }
    let mut typed = name.clone();
    if ui
        .add(
            egui::TextEdit::singleline(&mut typed)
                .desired_width(110.0)
                .hint_text("or its name"),
        )
        .changed()
    {
        *name = typed.trim().to_uppercase();
        changed = true;
        listening = false;
    }
    ui.data_mut(|data| data.insert_temp(id, (listening, super_down)));
    changed
}

enum Pressed {
    /// A key, and the modifiers held as it was pressed.
    Key(String, egui::Modifiers),
    /// Escape: the key stays as it is.
    Leave,
}

/// The modifiers of a binding as they were held when its key was
/// pressed, by the names the model has for them.
fn recorded_modifiers(held: egui::Modifiers, super_held: bool) -> [(&'static str, bool); 4] {
    [
        ("super", super_held),
        ("ctrl", held.ctrl),
        ("alt", held.alt),
        ("shift", held.shift),
    ]
}

/// What the keys of a frame mean for a recording: the first key that is
/// no modifier, by the name the model has for it, with the modifiers held
/// as it was pressed. Super is followed across frames in `super_down`
/// (the toolkit tells of it as a key, not among the modifiers).
fn pressed_key(events: &[egui::Event], super_down: &mut bool) -> Option<Pressed> {
    for event in events {
        let egui::Event::Key {
            key,
            pressed,
            repeat: false,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        match key {
            egui::Key::SuperLeft | egui::Key::SuperRight => *super_down = *pressed,
            // The other modifiers are told with the key itself.
            egui::Key::ShiftLeft
            | egui::Key::ShiftRight
            | egui::Key::ControlLeft
            | egui::Key::ControlRight
            | egui::Key::AltLeft
            | egui::Key::AltRight => {}
            egui::Key::Escape if *pressed => return Some(Pressed::Leave),
            other if *pressed => return Some(Pressed::Key(key_name(*other), *modifiers)),
            _ => {}
        }
    }
    None
}

/// The model's name for a key of the toolkit.
fn key_name(key: egui::Key) -> String {
    use egui::Key;
    match key {
        Key::Enter => "RETURN".to_string(),
        Key::Space => "SPACE".to_string(),
        Key::Tab => "TAB".to_string(),
        Key::Backspace => "BACKSPACE".to_string(),
        Key::Delete => "DELETE".to_string(),
        Key::Insert => "INSERT".to_string(),
        Key::Home => "HOME".to_string(),
        Key::End => "END".to_string(),
        Key::PageUp => "PAGEUP".to_string(),
        Key::PageDown => "PAGEDOWN".to_string(),
        Key::ArrowUp => "UP".to_string(),
        Key::ArrowDown => "DOWN".to_string(),
        Key::ArrowLeft => "LEFT".to_string(),
        Key::ArrowRight => "RIGHT".to_string(),
        Key::Minus => "MINUS".to_string(),
        Key::Equals | Key::Plus => "EQUALS".to_string(),
        Key::Comma => "COMMA".to_string(),
        Key::Period => "PERIOD".to_string(),
        Key::Semicolon | Key::Colon => "SEMICOLON".to_string(),
        Key::Slash | Key::Questionmark => "SLASH".to_string(),
        Key::Backslash | Key::Pipe => "BACKSLASH".to_string(),
        Key::Quote => "QUOTE".to_string(),
        Key::Backtick => "BACKTICK".to_string(),
        Key::OpenBracket => "LEFTBRACKET".to_string(),
        Key::CloseBracket => "RIGHTBRACKET".to_string(),
        // Letters, digits and F keys are named as the toolkit names them
        // (`A`, `Num4`, `F5`).
        other => other.name().trim_start_matches("Num").to_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keys_name_in_a_form_is_chosen_by_pressing_the_key() {
        let context = egui::Context::default();
        let mut name = "T".to_string();
        let mut changes = 0;
        let corner = std::cell::Cell::new(egui::Pos2::ZERO);
        let frame = |events: Vec<egui::Event>, name: &mut String, changes: &mut usize| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut drawn = context.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        corner.set(ui.cursor().min);
                        if key_field(ui, "/x/key", name) {
                            *changes += 1;
                        }
                    });
                });
            });
            drawn.textures_delta.clear();
        };
        let key = |key: egui::Key, pressed: bool| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let press = |at: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame(Vec::new(), &mut name, &mut changes);
        frame(Vec::new(), &mut name, &mut changes);
        let button = corner.get() + egui::vec2(8.0, 8.0);
        // A key pressed while it does not listen is nothing to it.
        frame(vec![key(egui::Key::K, true)], &mut name, &mut changes);
        assert_eq!((name.as_str(), changes), ("T", 0));
        // Pressed on, then a key: that key's name.
        frame(press(button), &mut name, &mut changes);
        frame(Vec::new(), &mut name, &mut changes);
        frame(
            vec![key(egui::Key::ArrowLeft, true)],
            &mut name,
            &mut changes,
        );
        assert_eq!((name.as_str(), changes), ("LEFT", 1));
        // It listens no more.
        frame(vec![key(egui::Key::K, true)], &mut name, &mut changes);
        assert_eq!((name.as_str(), changes), ("LEFT", 1));
        // Pressed on again, then Escape: as it was.
        frame(press(button), &mut name, &mut changes);
        frame(vec![key(egui::Key::Escape, true)], &mut name, &mut changes);
        frame(vec![key(egui::Key::K, true)], &mut name, &mut changes);
        assert_eq!((name.as_str(), changes), ("LEFT", 1));
    }

    #[test]
    fn a_recording_takes_the_first_key_that_is_no_modifier() {
        let key = |key: egui::Key, pressed: bool, modifiers: egui::Modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        // Super, then 2 with Shift held: one chord.
        let mut super_down = false;
        let events = [
            key(egui::Key::SuperLeft, true, egui::Modifiers::NONE),
            key(egui::Key::ShiftLeft, true, egui::Modifiers::SHIFT),
            key(egui::Key::Num2, true, egui::Modifiers::SHIFT),
        ];
        let Some(Pressed::Key(name, held)) = pressed_key(&events, &mut super_down) else {
            panic!("a key");
        };
        assert_eq!(name, "2");
        assert!(super_down && held.shift && !held.ctrl);
        assert_eq!(
            recorded_modifiers(held, super_down),
            [
                ("super", true),
                ("ctrl", false),
                ("alt", false),
                ("shift", true)
            ]
        );
        // Super pressed in one frame, the key in the next.
        let mut super_down = false;
        assert!(pressed_key(&events[..1], &mut super_down).is_none());
        assert!(super_down);
        assert!(pressed_key(&events[2..], &mut super_down).is_some());
        // Super let go before the key: not part of the chord.
        let mut super_down = true;
        let after = [
            key(egui::Key::SuperLeft, false, egui::Modifiers::NONE),
            key(egui::Key::K, true, egui::Modifiers::NONE),
        ];
        assert!(
            matches!(pressed_key(&after, &mut super_down), Some(Pressed::Key(name, _)) if name == "K")
        );
        assert!(!super_down);
        // Escape leaves the binding as it is; a release records nothing.
        assert!(matches!(
            pressed_key(
                &[key(egui::Key::Escape, true, egui::Modifiers::NONE)],
                &mut super_down
            ),
            Some(Pressed::Leave)
        ));
        assert!(pressed_key(
            &[key(egui::Key::K, false, egui::Modifiers::NONE)],
            &mut super_down
        )
        .is_none());
    }
    use serde_json::json;

    #[test]
    fn keys_read_as_a_person_says_them() {
        assert_eq!(
            chord(&json!({"super": true, "shift": true, "key": "J"})),
            "Super+Shift+J"
        );
        assert_eq!(
            chord(&json!({"ctrl": true, "alt": true, "key": "RETURN"})),
            "Ctrl+Alt+Enter"
        );
        assert_eq!(chord(&json!({"super": true, "key": "SLASH"})), "Super+/");
        assert_eq!(chord(&json!({"key": "VOLUME_UP"})), "Volume up");
        assert_eq!(chord(&json!({"super": true, "key": ""})), "Super");
    }

    #[test]
    fn bindings_on_the_same_keys_are_found() {
        let bindings = vec![
            json!({"name": "a", "enabled": true, "super": true, "key": "T", "submap": "default"}),
            json!({"name": "b", "enabled": true, "super": true, "key": "t", "submap": "default"}),
            // Another mode, switched off, other keys: no clash.
            json!({"name": "c", "enabled": true, "super": true, "key": "T", "submap": "resize"}),
            json!({"name": "d", "enabled": false, "super": true, "key": "T", "submap": "default"}),
            json!({"name": "e", "enabled": true, "super": true, "shift": true, "key": "T", "submap": "default"}),
        ];
        let found = clashes(&bindings);
        assert_eq!(found.len(), 2);
        assert_eq!(found[&0], vec![1]);
        assert_eq!(found[&1], vec![0]);
    }

    #[test]
    fn gevreks_own_bindings_do_not_fight() {
        // The standard keys, as the form would see them.
        let config = gevrek_model::examples::starting_configuration();
        let bindings: Vec<Value> = serde_json::to_value(&config.desktop.bindings)
            .expect("JSON")
            .as_array()
            .cloned()
            .expect("a list");
        let found = clashes(&bindings);
        let names: Vec<&str> = found
            .keys()
            .filter_map(|index| bindings[*index]["name"].as_str())
            .collect();
        assert!(found.is_empty(), "on the same keys: {names:?}");
    }

    #[test]
    fn a_pressed_key_has_the_models_name() {
        assert_eq!(key_name(egui::Key::A), "A");
        assert_eq!(key_name(egui::Key::Num4), "4");
        assert_eq!(key_name(egui::Key::F5), "F5");
        assert_eq!(key_name(egui::Key::ArrowLeft), "LEFT");
        for key in [
            egui::Key::Enter,
            egui::Key::Slash,
            egui::Key::PageUp,
            egui::Key::Q,
            egui::Key::Num0,
            egui::Key::F12,
        ] {
            let name = key_name(key);
            assert!(
                gevrek_model::keys::key_name_to_vk(&name).is_some(),
                "{name} is a key the model knows"
            );
        }
    }
}
