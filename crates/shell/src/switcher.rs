//! The window switcher: the windows that are open, to go to one of them.
//!
//! It comes up on its key (Alt+Tab in the examples). While the key's
//! modifier is held, Tab goes on to the next window and Shift+Tab back;
//! letting the modifier go shows the window the choice is on. Called
//! without a modifier held (from a command, or a key that has none), it
//! stays until Enter or a click chooses, or Escape puts it away.
//!
//! The window with the keyboard is at the front, then the others in the
//! order they last had it (so that one Tab is "the window before"); those
//! not used since the shell began follow, the shown workspace's first.

use crate::launcher::one_line;
use gevrek_ui::theme;
use serde_json::Value;

/// A window to choose.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub title: String,
    pub class: String,
    /// The workspace it is on, and the display, for the line below.
    pub place: String,
}

/// The windows the window manager's state lists, in the order they are
/// offered in.
pub fn entries(wm: &Value) -> Vec<Entry> {
    let mut focused: Vec<Entry> = Vec::new();
    let mut here: Vec<Entry> = Vec::new();
    let mut elsewhere: Vec<Entry> = Vec::new();
    let monitors = wm["monitors"].as_array().cloned().unwrap_or_default();
    let several = monitors.len() > 1;
    for monitor in &monitors {
        let display = monitor["name"].as_str().unwrap_or_default();
        let shown = &monitor["workspace"]["id"];
        let has_keyboard = monitor["is_focused"].as_bool().unwrap_or(false);
        for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
            // (What is shown of a workspace: its title, or its name.)
            let name = workspace["title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .or(workspace["name"].as_str())
                .unwrap_or_default();
            let is_shown = &workspace["id"] == shown;
            for window in workspace["detailed_windows"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let Some(id) = window["window_id"].as_u64() else {
                    continue;
                };
                if window["is_border_window"].as_bool().unwrap_or(false) {
                    continue;
                }
                let entry = Entry {
                    id,
                    title: window["title"].as_str().unwrap_or_default().to_string(),
                    class: window["class"].as_str().unwrap_or_default().to_string(),
                    place: if several {
                        format!("{name}  ·  {display}")
                    } else {
                        name.to_string()
                    },
                };
                if is_shown && has_keyboard && window["is_focused"].as_bool().unwrap_or(false) {
                    focused.push(entry);
                } else if is_shown {
                    here.push(entry);
                } else {
                    elsewhere.push(entry);
                }
            }
        }
    }
    focused.extend(here);
    focused.extend(elsewhere);
    focused
}

/// The window that has the keyboard, by the window manager's state.
pub fn with_keyboard(wm: &Value) -> Option<u64> {
    let monitors = wm["monitors"].as_array()?;
    let monitor = monitors
        .iter()
        .find(|monitor| monitor["is_focused"].as_bool().unwrap_or(false))?;
    let shown = &monitor["workspace"]["id"];
    monitor["workspaces"]
        .as_array()?
        .iter()
        .filter(|workspace| &workspace["id"] == shown)
        .flat_map(|workspace| {
            workspace["detailed_windows"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .find(|window| {
            window["is_focused"].as_bool().unwrap_or(false)
                && !window["is_border_window"].as_bool().unwrap_or(false)
        })
        .and_then(|window| window["window_id"].as_u64())
}

/// The windows in the order they last had the keyboard, the newest first:
/// what the switcher offers them in. (Kept by the shell from the states
/// it is sent, so it begins when the shell does.)
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Used(Vec<u64>);

impl Used {
    /// How many windows are remembered.
    const KEPT: usize = 64;

    /// This window has the keyboard now.
    pub fn note(&mut self, window: u64) {
        if self.0.first() == Some(&window) {
            return;
        }
        self.0.retain(|known| *known != window);
        self.0.insert(0, window);
        self.0.truncate(Self::KEPT);
    }

    /// The entries with those that were used first, the last used at the
    /// front; the others keep the order they came in, after them.
    pub fn order(&self, mut entries: Vec<Entry>) -> Vec<Entry> {
        let rank = |entry: &Entry| {
            self.0
                .iter()
                .position(|known| *known == entry.id)
                .unwrap_or(usize::MAX)
        };
        // (The sort keeps the order of those it finds equal. The window
        // with the keyboard stays first whatever is remembered.)
        let first = (!entries.is_empty()).then(|| entries.remove(0));
        entries.sort_by_key(rank);
        first.into_iter().chain(entries).collect()
    }
}

/// A small picture of a window: red, green, blue and how solid, row by
/// row. (One without a size: the window gave none.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub title: String,
    pub app_id: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The switcher while it is open.
pub struct Switcher {
    pub entries: Vec<Entry>,
    pub chosen: usize,
    /// Pictures of the windows, once they have been taken, each with
    /// what it is drawn from once it has been drawn.
    pub thumbs: Vec<(Thumb, Option<egui::TextureHandle>)>,
    /// A modifier was seen held since it came up: letting it go chooses.
    held: bool,
}

/// What a frame of the switcher asks for.
#[derive(Debug, Default, PartialEq)]
pub struct Asked {
    /// Put it away without going anywhere.
    pub close: bool,
    /// Go to this window.
    pub go: Option<u64>,
}

impl Switcher {
    /// With the choice on the second window: the one used before the one
    /// that has the keyboard.
    pub fn new(entries: Vec<Entry>) -> Self {
        let chosen = usize::from(entries.len() > 1);
        Self {
            entries,
            chosen,
            thumbs: Vec::new(),
            held: false,
        }
    }

    /// Which picture is the window's the choice is on: its place among
    /// the pictures.
    fn picture_of_chosen(&self) -> Option<usize> {
        let chosen = self.entries.get(self.chosen)?;
        // Windows of one title and program: by their order in the list
        // the window manager gave (the order the pictures were taken in).
        let mut same: Vec<u64> = self
            .entries
            .iter()
            .filter(|entry| entry.title == chosen.title && entry.class == chosen.class)
            .map(|entry| entry.id)
            .collect();
        same.sort_unstable();
        let nth = same.iter().position(|id| *id == chosen.id)?;
        self.thumbs
            .iter()
            .enumerate()
            .filter(|(_, (thumb, _))| thumb.title == chosen.title && thumb.app_id == chosen.class)
            .nth(nth)
            .map(|(index, _)| index)
            .filter(|index| self.thumbs[*index].0.width > 0 && self.thumbs[*index].0.height > 0)
    }

    pub fn step(&mut self, by: i32) {
        let count = self.entries.len() as i32;
        if count > 0 {
            self.chosen = (self.chosen as i32 + by).rem_euclid(count) as usize;
        }
    }

    /// Take in what the keys did in this frame. `modifier_held`: Alt or
    /// the Super key is down now.
    pub fn keys(
        &mut self,
        modifier_held: bool,
        next: u32,
        previous: u32,
        enter: bool,
        escape: bool,
    ) -> Asked {
        for _ in 0..next {
            self.step(1);
        }
        for _ in 0..previous {
            self.step(-1);
        }
        let chosen = self.entries.get(self.chosen).map(|entry| entry.id);
        if escape {
            return Asked {
                close: true,
                go: None,
            };
        }
        if enter {
            return Asked {
                close: true,
                go: chosen,
            };
        }
        if modifier_held {
            self.held = true;
        } else if self.held {
            // Let go: there.
            return Asked {
                close: true,
                go: chosen,
            };
        }
        Asked::default()
    }
}

/// How large the switcher's surface is for so many windows, in logical
/// pixels.
pub fn size(count: usize) -> (u32, u32) {
    (
        LIST_WIDTH as u32 + PREVIEW_WIDTH as u32 + 40,
        (28 + count.clamp(1, SHOWN) as u32 * 46).max(PREVIEW_HEIGHT as u32 + 28),
    )
}

/// How wide the list of windows is, and the room beside it for the
/// picture of the window the choice is on.
const LIST_WIDTH: f32 = 520.0;
const PREVIEW_WIDTH: f32 = 320.0;
const PREVIEW_HEIGHT: f32 = 200.0;

/// How many lines are shown at once.
pub const SHOWN: usize = 10;

/// Draw the switcher and take in its keys.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub fn draw(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    switcher: &mut Switcher,
    icon: &mut dyn FnMut(&egui::Context, &str) -> Option<egui::TextureHandle>,
) -> Asked {
    draw_held(ui, palette, switcher, icon, None)
}

/// As [`draw`], with whether a modifier is held said by who knows better
/// than the keys the surface was told of (Windows: a surface is not told
/// of a key let go that was pressed before it had the keyboard).
pub fn draw_held(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    switcher: &mut Switcher,
    icon: &mut dyn FnMut(&egui::Context, &str) -> Option<egui::TextureHandle>,
    modifier_held: Option<bool>,
) -> Asked {
    let context = ui.ctx().clone();
    let (modifiers, next, previous, enter, escape) = context.input_mut(|input| {
        let mut next = 0;
        let mut previous = 0;
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            {
                match key {
                    egui::Key::Tab if modifiers.shift => previous += 1,
                    egui::Key::Tab | egui::Key::ArrowDown | egui::Key::ArrowRight => next += 1,
                    egui::Key::ArrowUp | egui::Key::ArrowLeft => previous += 1,
                    _ => {}
                }
            }
        }
        (
            input.modifiers,
            next,
            previous,
            input.key_pressed(egui::Key::Enter),
            input.key_pressed(egui::Key::Escape),
        )
    });
    let held = modifier_held
        .unwrap_or(modifiers.alt || modifiers.command || modifiers.mac_cmd || modifiers.ctrl);
    let mut asked = switcher.keys(held, next, previous, enter, escape);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                .corner_radius(14)
                .inner_margin(14),
        )
        .show(ui, |ui| {
            // The lines are as far apart as `size` reckons with (42 for a
            // line and 4 between two), whatever the look in use gives
            // between things: with more, the last lines were cut off at
            // the surface's lower edge.
            ui.spacing_mut().item_spacing.y = 4.0;
            if switcher.entries.is_empty() {
                ui.label(egui::RichText::new("No window is open.").color(palette.muted));
                return;
            }
            // Beside the list: the window the choice is on, as it looks.
            let whole = ui.max_rect();
            let room = egui::Rect::from_min_size(
                egui::pos2(whole.right() - PREVIEW_WIDTH, whole.top()),
                egui::vec2(PREVIEW_WIDTH, whole.height()),
            );
            ui.painter().rect_filled(room, 10.0, palette.control);
            if let Some(index) = switcher.picture_of_chosen() {
                let (thumb, texture) = &mut switcher.thumbs[index];
                let texture = texture.get_or_insert_with(|| {
                    context.load_texture(
                        format!("switcher-{index}"),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [thumb.width as usize, thumb.height as usize],
                            &thumb.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                });
                let size = texture.size_vec2();
                let inside = room.shrink(8.0);
                let fits = (inside.width() / size.x).min(inside.height() / size.y);
                let place = egui::Rect::from_center_size(inside.center(), size * fits);
                ui.painter().image(
                    texture.id(),
                    place,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            ui.set_max_width(LIST_WIDTH);
            // The lines around the choice, when there are more than fit.
            let first = switcher
                .chosen
                .saturating_sub(SHOWN - 1)
                .min(switcher.entries.len().saturating_sub(SHOWN));
            for (index, entry) in switcher.entries.iter().enumerate().skip(first).take(SHOWN) {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(LIST_WIDTH, 42.0), egui::Sense::click());
                if index == switcher.chosen {
                    ui.painter().rect_filled(
                        rect,
                        8.0,
                        theme::mix(palette.surface, palette.accent, 0.22),
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 8.0, palette.control_hover());
                }
                let icon_place = egui::Rect::from_center_size(
                    rect.left_center() + egui::vec2(26.0, 0.0),
                    egui::vec2(30.0, 30.0),
                );
                match icon(&context, &entry.class) {
                    Some(texture) => {
                        let whole =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter()
                            .image(texture.id(), icon_place, whole, egui::Color32::WHITE);
                    }
                    None => {
                        ui.painter().rect_filled(icon_place, 8.0, palette.control);
                        let letter: String = entry
                            .class
                            .chars()
                            .chain(entry.title.chars())
                            .take(1)
                            .collect::<String>()
                            .to_uppercase();
                        ui.painter().text(
                            icon_place.center(),
                            egui::Align2::CENTER_CENTER,
                            letter,
                            egui::FontId::proportional(15.0),
                            palette.muted,
                        );
                    }
                }
                ui.painter().text(
                    rect.left_center() + egui::vec2(52.0, -8.0),
                    egui::Align2::LEFT_CENTER,
                    one_line(&entry.title, 52),
                    egui::FontId::proportional(16.0),
                    palette.text,
                );
                ui.painter().text(
                    rect.left_center() + egui::vec2(52.0, 11.0),
                    egui::Align2::LEFT_CENTER,
                    one_line(&format!("{}  ·  {}", entry.class, entry.place), 64),
                    egui::FontId::proportional(12.0),
                    palette.muted,
                );
                if response.clicked() {
                    asked = Asked {
                        close: true,
                        go: Some(entry.id),
                    };
                }
            }
        });
    // (Drawn again soon: letting the modifier go has no event of its own
    // that would wake the drawing.)
    context.request_repaint_after(std::time::Duration::from_millis(40));
    asked
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> Value {
        let window = |id: u64, title: &str, focused: bool| json!({"window_id": id, "title": title, "class": "app", "is_focused": focused});
        json!({"monitors": [
            {"name": "DP-1", "is_focused": true, "workspace": {"id": 10}, "workspaces": [
                {"id": 10, "name": "1", "detailed_windows": [window(1, "editor", false), window(2, "terminal", true)]},
                {"id": 11, "name": "2", "detailed_windows": [window(3, "mail", true)]},
            ]},
            {"name": "DP-2", "is_focused": false, "workspace": {"id": 20}, "workspaces": [
                {"id": 20, "name": "1", "detailed_windows": [window(4, "video", true)]},
            ]},
        ]})
    }

    #[test]
    fn the_picture_shown_is_the_chosen_windows() {
        let entry = |id: u64, title: &str| Entry {
            id,
            title: title.to_string(),
            class: "foot".to_string(),
            place: String::new(),
        };
        let thumb = |title: &str, width: u32| {
            (
                Thumb {
                    title: title.to_string(),
                    app_id: "foot".to_string(),
                    width,
                    height: 1,
                    rgba: Vec::new(),
                },
                None,
            )
        };
        // Two windows of one title: the one that came first has the
        // first picture of that title, whatever the order of the list.
        let mut switcher = Switcher::new(vec![
            entry(7, "shell"),
            entry(3, "shell"),
            entry(5, "editor"),
        ]);
        assert_eq!(switcher.picture_of_chosen(), None, "no pictures yet");
        switcher.thumbs = vec![thumb("shell", 1), thumb("editor", 2), thumb("shell", 3)];
        let width_at = |switcher: &Switcher| {
            switcher
                .picture_of_chosen()
                .map(|index| switcher.thumbs[index].0.width)
        };
        // (The choice begins on the second line: window 3.)
        assert_eq!(width_at(&switcher), Some(1));
        switcher.step(1);
        assert_eq!(width_at(&switcher), Some(2));
        switcher.step(1);
        assert_eq!(width_at(&switcher), Some(3));
        // A window there is no picture of.
        switcher.thumbs.truncate(1);
        assert_eq!(width_at(&switcher), None);
    }

    #[test]
    fn the_windows_come_in_the_order_they_were_used() {
        assert_eq!(with_keyboard(&state()), Some(2));
        let mut used = Used::default();
        // Mail was used, then the video, then the terminal (which has
        // the keyboard now); the editor never.
        for window in [3, 4, 2, 2] {
            used.note(window);
        }
        let order: Vec<u64> = used
            .order(entries(&state()))
            .iter()
            .map(|entry| entry.id)
            .collect();
        assert_eq!(order, [2, 4, 3, 1]);
        // Nothing remembered: the order of the state.
        let plain: Vec<u64> = Used::default()
            .order(entries(&state()))
            .iter()
            .map(|entry| entry.id)
            .collect();
        assert_eq!(plain, [2, 1, 4, 3]);
        // A window that is gone is no trouble, and the list has an end.
        for window in 100..300 {
            used.note(window);
        }
        assert_eq!(used.0.len(), Used::KEPT);
        assert_eq!(used.order(entries(&state())).len(), 4);
    }

    #[test]
    fn the_window_with_the_keyboard_is_first_then_what_is_shown_then_the_rest() {
        let found = entries(&state());
        let order: Vec<u64> = found.iter().map(|entry| entry.id).collect();
        assert_eq!(order, [2, 1, 4, 3]);
        assert_eq!(found[0].place, "1  ·  DP-1");
        assert_eq!(found[3].title, "mail");
    }

    #[test]
    fn tab_goes_on_and_letting_the_modifier_go_chooses() {
        let mut switcher = Switcher::new(entries(&state()));
        // It comes up on the window used before the one with the keys.
        assert_eq!(switcher.chosen, 1);
        // Held, and Tab once more: the third; round the end with more.
        assert_eq!(switcher.keys(true, 1, 0, false, false), Asked::default());
        assert_eq!(switcher.chosen, 2);
        assert_eq!(switcher.keys(true, 2, 0, false, false), Asked::default());
        assert_eq!(switcher.chosen, 0);
        assert_eq!(switcher.keys(true, 0, 1, false, false), Asked::default());
        assert_eq!(switcher.chosen, 3);
        // Let go: there.
        assert_eq!(
            switcher.keys(false, 0, 0, false, false),
            Asked {
                close: true,
                go: Some(3)
            }
        );
    }

    #[test]
    fn without_a_modifier_it_stays_until_enter_or_escape() {
        let mut switcher = Switcher::new(entries(&state()));
        for _ in 0..20 {
            assert_eq!(switcher.keys(false, 0, 0, false, false), Asked::default());
        }
        assert_eq!(
            switcher.keys(false, 1, 0, true, false),
            Asked {
                close: true,
                go: Some(4)
            }
        );
        let mut other = Switcher::new(entries(&state()));
        assert_eq!(
            other.keys(true, 0, 0, false, true),
            Asked {
                close: true,
                go: None
            }
        );
        // No window at all: nothing to go to.
        let mut none = Switcher::new(Vec::new());
        assert_eq!(
            none.keys(false, 3, 0, true, false),
            Asked {
                close: true,
                go: None
            }
        );
    }
}
