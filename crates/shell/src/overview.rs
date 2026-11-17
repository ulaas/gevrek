//! The overview of every workspace, where the shell draws it (Windows).
//!
//! A compositor shows the workspaces themselves, small; on Windows a
//! window that is put away is not drawn by anybody, so there is nothing
//! live to show of a workspace that is not the one seen. The shell
//! therefore remembers a picture of each window as it is used (taken
//! when it gets and when it loses the keyboard, and of every window in
//! sight when the overview opens), and draws each display's workspaces in
//! miniature from those: every window in its place, with its picture
//! where one is remembered and its name where none is. A press on a
//! miniature goes to that workspace, a press on a window in it to that
//! window; the arrow keys and Enter do the same, the digits go to the
//! workspaces of the display with the keyboard, Escape puts it away.

use gevrek_system::pictures::Picture;
use gevrek_ui::theme;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A window in a miniature: where it is on its display, each from 0 to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub id: u64,
    pub title: String,
    pub part: [f32; 4],
}

/// One workspace of a display.
#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    /// Its place among the display's workspaces.
    pub index: usize,
    /// What it is known by (to the command that shows it).
    pub name: String,
    /// What is shown of it: its title, or its name where it has none.
    pub title: String,
    /// It is the one the display shows.
    pub shown: bool,
    pub panes: Vec<Pane>,
}

/// One display with its workspaces.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub name: String,
    /// It has the keyboard's window.
    pub focused: bool,
    /// Its width by its height.
    pub aspect: f32,
    pub spaces: Vec<Space>,
}

/// The displays of the window manager's state, each with its workspaces
/// and their windows.
pub fn displays(wm: &Value) -> Vec<Display> {
    let number = |value: &Value| value.as_f64().unwrap_or(0.0) as f32;
    wm["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|monitor| {
            let (left, top) = (number(&monitor["position"]["x"]), number(&monitor["position"]["y"]));
            let (width, height) = (
                number(&monitor["size"]["width"]),
                number(&monitor["size"]["height"]),
            );
            if width < 1.0 || height < 1.0 {
                return None;
            }
            let shown = &monitor["workspace"]["id"];
            let spaces = monitor["workspaces"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, workspace)| Space {
                    index,
                    name: workspace["name"].as_str().unwrap_or_default().to_string(),
                    title: workspace["title"]
                        .as_str()
                        .filter(|title| !title.trim().is_empty())
                        .or(workspace["name"].as_str())
                        .unwrap_or_default()
                        .to_string(),
                    shown: &workspace["id"] == shown,
                    panes: workspace["detailed_windows"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|window| !window["is_border_window"].as_bool().unwrap_or(false))
                        .filter(|window| !window["is_minimized"].as_bool().unwrap_or(false))
                        .filter_map(|window| {
                            let (wide, high) = (
                                number(&window["size"]["width"]),
                                number(&window["size"]["height"]),
                            );
                            if wide < 1.0 || high < 1.0 {
                                return None;
                            }
                            Some(Pane {
                                id: window["window_id"].as_u64()?,
                                title: window["title"].as_str().unwrap_or_default().to_string(),
                                part: [
                                    (number(&window["position"]["x"]) - left) / width,
                                    (number(&window["position"]["y"]) - top) / height,
                                    wide / width,
                                    high / height,
                                ],
                            })
                        })
                        .collect(),
                })
                .collect();
            Some(Display {
                name: monitor["name"].as_str().unwrap_or_default().to_string(),
                focused: monitor["is_focused"].as_bool().unwrap_or(false),
                aspect: width / height,
                spaces,
            })
        })
        .collect()
}

/// The windows that are in sight now: those of the workspaces shown.
pub fn in_sight(displays: &[Display]) -> Vec<u64> {
    displays
        .iter()
        .flat_map(|display| &display.spaces)
        .filter(|space| space.shown)
        .flat_map(|space| &space.panes)
        .map(|pane| pane.id)
        .collect()
}

/// The pictures of windows that are remembered.
#[derive(Default)]
pub struct Remembered {
    /// Each window's last picture, and a number that is another for
    /// another picture.
    pictures: HashMap<u64, (Arc<Picture>, u64)>,
    /// When a picture of a window was last asked for.
    asked: HashMap<u64, Instant>,
    stamp: u64,
}

impl Remembered {
    /// How many windows' pictures are kept at most.
    const KEPT: usize = 48;
    /// A window's picture is not taken again sooner than this.
    const APART: Duration = Duration::from_secs(2);

    /// Whether a picture of this window is to be taken now; said yes,
    /// it counts as asked for.
    pub fn wants(&mut self, window: u64, now: Instant) -> bool {
        let due = self
            .asked
            .get(&window)
            .is_none_or(|last| now.saturating_duration_since(*last) >= Self::APART);
        if due {
            self.asked.insert(window, now);
        }
        due
    }

    /// A picture was taken.
    pub fn keep(&mut self, window: u64, picture: Picture) {
        self.stamp += 1;
        self.pictures.insert(window, (Arc::new(picture), self.stamp));
    }

    pub fn of(&self, window: u64) -> Option<&(Arc<Picture>, u64)> {
        self.pictures.get(&window)
    }

    /// Only the windows that are still there are remembered, and of
    /// those not more than fit: the ones whose pictures are oldest go.
    pub fn only(&mut self, there: &[u64]) {
        self.pictures.retain(|window, _| there.contains(window));
        self.asked.retain(|window, _| there.contains(window));
        while self.pictures.len() > Self::KEPT {
            let Some(oldest) = self
                .pictures
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(window, _)| *window)
            else {
                break;
            };
            self.pictures.remove(&oldest);
        }
    }
}

/// Where the overview goes when something in it is chosen.
#[derive(Debug, Clone, PartialEq)]
pub enum Go {
    /// A workspace: its display, whether that has the keyboard, the
    /// workspace's place among the display's, and its name (it is shown
    /// by its name where it has one: `panel::show_workspace_at`).
    Space(String, bool, usize, String),
    Window(u64),
}

impl Go {
    /// The command that goes there.
    pub fn command(&self) -> String {
        match self {
            Go::Space(display, focused, index, name) => {
                crate::panel::show_workspace_at(display, *focused, *index, name)
            }
            Go::Window(window) => serde_json::json!({
                "command": "focus_window",
                "selector": {"kind": "handle", "value": window},
            })
            .to_string(),
        }
    }
}

/// What a frame of the overview asks for.
#[derive(Debug, Default, PartialEq)]
pub struct Asked {
    pub close: bool,
    pub go: Option<Go>,
}

/// The overview while it is open.
#[derive(Default)]
pub struct Overview {
    /// The workspace the keys are on: its display and its place there.
    chosen: Option<(usize, usize)>,
    /// The pictures as something to draw, by window, each with the
    /// number of the picture it was made of.
    drawn: HashMap<u64, (u64, egui::TextureHandle)>,
}

/// The workspace the keys go to from one, by so many steps, through all
/// the displays' workspaces in their order and round at the ends.
pub fn stepped(displays: &[Display], from: (usize, usize), by: isize) -> (usize, usize) {
    let all: Vec<(usize, usize)> = displays
        .iter()
        .enumerate()
        .flat_map(|(at, display)| (0..display.spaces.len()).map(move |space| (at, space)))
        .collect();
    if all.is_empty() {
        return from;
    }
    let now = all.iter().position(|place| *place == from).unwrap_or(0) as isize;
    all[(now + by).rem_euclid(all.len() as isize) as usize]
}

/// How wide a miniature is where so many stand in a room so wide.
pub fn miniature_width(room: f32, most: usize) -> f32 {
    let gap = 14.0;
    // (A little less than fits exactly: a row that is full to the last
    // point is broken.)
    let fit = (room - 6.0 - gap * most.saturating_sub(1) as f32) / most.max(1) as f32;
    fit.clamp(170.0, 340.0)
}

impl Overview {
    /// The size the overview's surface is to have for these displays in
    /// a room of this size (points): as large as what it shows, within
    /// the room.
    pub fn size(displays: &[Display], room: (f32, f32)) -> (u32, u32) {
        let most = displays
            .iter()
            .map(|display| display.spaces.len())
            .max()
            .unwrap_or(1)
            .clamp(1, 5);
        let wide =
            (room.0 * 0.9).min(most as f32 * 340.0 + (most as f32 - 1.0) * 14.0 + 58.0 + 8.0);
        let inside = wide - 58.0;
        // Its head, its margins, and each display's rows with its name.
        let mut high = 58.0 + 40.0;
        for display in displays {
            let each = miniature_width(inside, most);
            let across = ((inside + 14.0) / (each + 14.0)).floor().max(1.0) as usize;
            let rows = display.spaces.len().max(1).div_ceil(across);
            high += rows as f32 * (each / display.aspect.max(0.2) + 24.0 + 10.0) + 12.0;
            if displays.len() > 1 {
                high += 22.0;
            }
        }
        (wide.max(420.0) as u32, high.min(room.1 * 0.9).max(240.0) as u32)
    }

    /// Draw it. What was asked for in this frame.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        palette: &theme::Palette,
        displays: &[Display],
        remembered: &Remembered,
    ) -> Asked {
        use gevrek_ui::controls::one_line;
        let mut asked = Asked::default();
        let context = ui.ctx().clone();
        // The keys.
        let first = displays
            .iter()
            .enumerate()
            .find(|(_, display)| display.focused)
            .or(displays.iter().enumerate().next())
            .map(|(at, display)| {
                (
                    at,
                    display
                        .spaces
                        .iter()
                        .position(|space| space.shown)
                        .unwrap_or(0),
                )
            });
        let chosen = self.chosen.or(first);
        let (mut by, mut enter, mut digit) = (0isize, false, None);
        context.input(|input| {
            for event in &input.events {
                if let egui::Event::Key {
                    key, pressed: true, ..
                } = event
                {
                    match key {
                        egui::Key::ArrowRight | egui::Key::ArrowDown | egui::Key::Tab => by += 1,
                        egui::Key::ArrowLeft | egui::Key::ArrowUp => by -= 1,
                        egui::Key::Enter | egui::Key::Space => enter = true,
                        egui::Key::Escape => asked.close = true,
                        other => {
                            let digits = [
                                egui::Key::Num1,
                                egui::Key::Num2,
                                egui::Key::Num3,
                                egui::Key::Num4,
                                egui::Key::Num5,
                                egui::Key::Num6,
                                egui::Key::Num7,
                                egui::Key::Num8,
                                egui::Key::Num9,
                            ];
                            if let Some(at) = digits.iter().position(|digit| digit == other) {
                                digit = Some(at);
                            }
                        }
                    }
                }
            }
        });
        let chosen = match (chosen, by) {
            (Some(chosen), by) if by != 0 => Some(stepped(displays, chosen, by)),
            (chosen, _) => chosen,
        };
        self.chosen = chosen;
        let go_to = |place: (usize, usize)| {
            displays.get(place.0).and_then(|display| {
                display
                    .spaces
                    .get(place.1)
                    .map(|space| {
                        Go::Space(
                            display.name.clone(),
                            display.focused,
                            space.index,
                            space.name.clone(),
                        )
                    })
            })
        };
        if enter {
            asked.go = chosen.and_then(go_to);
        }
        if let (Some(digit), Some((display, _))) = (digit, first) {
            asked.go = go_to((display, digit)).or(asked.go.take());
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                    .corner_radius(18)
                    .inner_margin(28),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Workspaces")
                            .size(15.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Enter goes there  ·  Escape puts this away")
                                .size(12.0)
                                .color(palette.faint),
                        );
                    });
                });
                ui.add_space(6.0);
                let most = displays
                    .iter()
                    .map(|display| display.spaces.len())
                    .max()
                    .unwrap_or(1)
                    .clamp(1, 5);
                let wide = miniature_width(ui.available_width(), most);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (at, display) in displays.iter().enumerate() {
                            if displays.len() > 1 {
                                gevrek_ui::widgets::caption(ui, &display.name);
                            }
                            let high = wide / display.aspect.max(0.2);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(14.0, 10.0);
                                for (place, space) in display.spaces.iter().enumerate() {
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(wide, high + 24.0),
                                        egui::Sense::click(),
                                    );
                                    let screen =
                                        egui::Rect::from_min_size(rect.min, egui::vec2(wide, high));
                                    let painter = ui.painter().clone();
                                    painter.rect_filled(
                                        screen,
                                        8.0,
                                        theme::mix(palette.bg, egui::Color32::BLACK, 0.25),
                                    );
                                    // Its windows, each in its place.
                                    let mut under = None;
                                    for pane in &space.panes {
                                        let of = egui::Rect::from_min_size(
                                            screen.min
                                                + egui::vec2(
                                                    pane.part[0] * wide,
                                                    pane.part[1] * high,
                                                ),
                                            egui::vec2(pane.part[2] * wide, pane.part[3] * high),
                                        )
                                        .intersect(screen)
                                        .shrink(1.0);
                                        if of.width() < 2.0 || of.height() < 2.0 {
                                            continue;
                                        }
                                        if response
                                            .hover_pos()
                                            .is_some_and(|pointer| of.contains(pointer))
                                        {
                                            under = Some((pane.id, of));
                                        }
                                        match self.texture(&context, pane.id, remembered) {
                                            Some(texture) => {
                                                egui::Image::new(&texture)
                                                    .corner_radius(3)
                                                    .paint_at(ui, of);
                                            }
                                            None => {
                                                painter.rect(
                                                    of,
                                                    3.0,
                                                    palette.control,
                                                    egui::Stroke::new(1.0, palette.border),
                                                    egui::StrokeKind::Inside,
                                                );
                                                if of.width() > 40.0 && of.height() > 16.0 {
                                                    let words = one_line(
                                                        ui,
                                                        &pane.title,
                                                        egui::FontId::proportional(10.5),
                                                        palette.muted,
                                                        of.width() - 8.0,
                                                    );
                                                    painter.galley(
                                                        of.center() - words.size() / 2.0,
                                                        words,
                                                        palette.muted,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    // The window the pointer is on.
                                    if let Some((_, of)) = under {
                                        painter.rect_stroke(
                                            of,
                                            3.0,
                                            egui::Stroke::new(2.0, palette.text),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    // The one that is shown, the one the keys
                                    // are on, the one the pointer is on.
                                    let keys = chosen == Some((at, place));
                                    let edge = if space.shown {
                                        egui::Stroke::new(2.0, palette.accent)
                                    } else if response.hovered() {
                                        egui::Stroke::new(1.0, palette.border_strong())
                                    } else {
                                        egui::Stroke::new(1.0, palette.border)
                                    };
                                    painter.rect_stroke(screen, 8.0, edge, egui::StrokeKind::Inside);
                                    if keys {
                                        painter.rect_stroke(
                                            screen.expand(3.0),
                                            11.0,
                                            egui::Stroke::new(2.0, palette.text),
                                            egui::StrokeKind::Outside,
                                        );
                                    }
                                    let name = if space.title.is_empty() {
                                        (space.index + 1).to_string()
                                    } else {
                                        space.title.clone()
                                    };
                                    let words = one_line(
                                        ui,
                                        &name,
                                        egui::FontId::proportional(13.0),
                                        if space.shown { palette.text } else { palette.muted },
                                        wide - 60.0,
                                    );
                                    painter.galley(
                                        egui::pos2(rect.left() + 2.0, screen.bottom() + 5.0),
                                        words,
                                        palette.text,
                                    );
                                    let count = match space.panes.len() {
                                        0 => "empty".to_string(),
                                        1 => "1 window".to_string(),
                                        more => format!("{more} windows"),
                                    };
                                    painter.text(
                                        egui::pos2(rect.right() - 2.0, screen.bottom() + 6.0),
                                        egui::Align2::RIGHT_TOP,
                                        count,
                                        egui::FontId::proportional(11.5),
                                        palette.faint,
                                    );
                                    if response.clicked() {
                                        asked.go = Some(match under {
                                            Some((window, _)) => Go::Window(window),
                                            None => Go::Space(
                                                display.name.clone(),
                                                display.focused,
                                                space.index,
                                                space.name.clone(),
                                            ),
                                        });
                                    }
                                }
                            });
                            ui.add_space(12.0);
                        }
                    });
            });
        asked
    }

    /// A window's remembered picture as something to draw, made again
    /// when the picture is another.
    fn texture(
        &mut self,
        context: &egui::Context,
        window: u64,
        remembered: &Remembered,
    ) -> Option<egui::TextureHandle> {
        let (picture, stamp) = remembered.of(window)?;
        if let Some((made_of, texture)) = self.drawn.get(&window) {
            if made_of == stamp {
                return Some(texture.clone());
            }
        }
        if picture.width == 0 || picture.height == 0 {
            return None;
        }
        let texture = context.load_texture(
            format!("overview-{window}"),
            egui::ColorImage::from_rgba_unmultiplied(
                [picture.width as usize, picture.height as usize],
                &picture.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        self.drawn.insert(window, (*stamp, texture.clone()));
        Some(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two displays: the first with three workspaces (the second shown,
    /// with two windows side by side), the other with one.
    pub(crate) fn state() -> Value {
        let window = |id: u64, title: &str, x: i32, y: i32, width: i32, height: i32| {
            serde_json::json!({
                "window_id": id, "title": title, "class": "App", "is_border_window": false,
                "is_minimized": false,
                "position": {"x": x, "y": y}, "size": {"width": width, "height": height},
            })
        };
        serde_json::json!({"monitors": [
            {
                "name": "Monitor A", "is_focused": true,
                "position": {"x": 0, "y": 0}, "size": {"width": 2560, "height": 1440},
                "workspace": {"id": 2},
                "workspaces": [
                    {"id": 1, "name": "one", "detailed_windows": [
                        window(11, "Mail", 12, 54, 2536, 1326),
                    ]},
                    {"id": 2, "name": "two", "detailed_windows": [
                        window(21, "Editor", 12, 54, 1264, 1326),
                        window(22, "Browser", 1284, 54, 1264, 659),
                        window(23, "Terminal", 1284, 721, 1264, 659),
                        {"window_id": 99, "title": "", "is_border_window": true,
                         "position": {"x": 0, "y": 0}, "size": {"width": 10, "height": 10}},
                    ]},
                    {"id": 3, "name": "three", "detailed_windows": []},
                ],
            },
            {
                "name": "Monitor B", "is_focused": false,
                "position": {"x": 2560, "y": 0}, "size": {"width": 2560, "height": 1440},
                "workspace": {"id": 7},
                "workspaces": [
                    {"id": 7, "name": "", "detailed_windows": [
                        window(71, "Chat", 2572, 54, 2536, 1326),
                    ]},
                ],
            },
        ]})
    }

    #[test]
    fn the_displays_workspaces_and_windows_are_read_from_the_state() {
        let displays = displays(&state());
        assert_eq!(displays.len(), 2);
        let first = &displays[0];
        assert!(first.focused && (first.aspect - 2560.0 / 1440.0).abs() < 1e-4);
        let names: Vec<(&str, bool, usize)> = first
            .spaces
            .iter()
            .map(|space| (space.name.as_str(), space.shown, space.panes.len()))
            .collect();
        assert_eq!(names, [("one", false, 1), ("two", true, 3), ("three", false, 0)]);
        // What is shown of one is its title, or its name where it has none.
        assert_eq!(first.spaces[0].title, "one");
        let titled = super::displays(&serde_json::json!({"monitors": [{
            "name": "A", "size": {"width": 100, "height": 100}, "workspace": {"id": 1},
            "workspaces": [{"id": 1, "name": "web", "title": "Browsing", "detailed_windows": []}],
        }]}));
        assert_eq!(
            (titled[0].spaces[0].name.as_str(), titled[0].spaces[0].title.as_str()),
            ("web", "Browsing")
        );
        // A window's place is a part of its display.
        let browser = &first.spaces[1].panes[1];
        assert_eq!(browser.id, 22);
        assert!((browser.part[0] - 1284.0 / 2560.0).abs() < 1e-4);
        assert!((browser.part[2] - 1264.0 / 2560.0).abs() < 1e-4);
        // On the second display too, measured from that display's corner.
        let chat = &displays[1].spaces[0].panes[0];
        assert!((chat.part[0] - 12.0 / 2560.0).abs() < 1e-4);
        // In sight: the windows of the workspaces that are shown.
        assert_eq!(in_sight(&displays), [21, 22, 23, 71]);
        assert!(super::displays(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_keys_go_through_every_workspace_and_round() {
        let displays = displays(&state());
        assert_eq!(stepped(&displays, (0, 1), 1), (0, 2));
        assert_eq!(stepped(&displays, (0, 2), 1), (1, 0));
        assert_eq!(stepped(&displays, (1, 0), 1), (0, 0));
        assert_eq!(stepped(&displays, (0, 0), -1), (1, 0));
        assert_eq!(stepped(&[], (0, 0), 1), (0, 0));
        // Where it goes: a switch on the display with the keyboard, the
        // display named on another; a window by its number.
        let here = Go::Space("Monitor A".to_string(), true, 2, String::new()).command();
        assert!(here.contains("switch_workspace_selector"), "{here}");
        let there = Go::Space("Monitor B".to_string(), false, 0, String::new()).command();
        assert!(there.contains("Monitor B"), "{there}");
        // One with a name: by its name, on its display.
        let named = Go::Space("Monitor A".to_string(), true, 2, "three".to_string()).command();
        assert!(named.contains("Monitor A") && named.contains("three"), "{named}");
        assert!(!named.contains("switch_workspace_selector"), "{named}");
        assert!(Go::Window(22).command().contains("focus_window"));
    }

    #[test]
    fn pictures_are_remembered_of_the_windows_that_are_there() {
        let picture = |shade: u8| Picture {
            width: 1,
            height: 1,
            rgba: vec![shade, shade, shade, 255],
        };
        let start = Instant::now();
        let mut remembered = Remembered::default();
        assert!(remembered.wants(5, start));
        // Not again at once; again after a while.
        assert!(!remembered.wants(5, start + Duration::from_millis(500)));
        assert!(remembered.wants(5, start + Duration::from_secs(3)));
        remembered.keep(5, picture(10));
        remembered.keep(6, picture(20));
        let first = remembered.of(5).map(|(_, stamp)| *stamp);
        remembered.keep(5, picture(30));
        assert_ne!(remembered.of(5).map(|(_, stamp)| *stamp), first);
        assert_eq!(remembered.of(5).map(|(picture, _)| picture.rgba[0]), Some(30));
        // A window that is gone is forgotten.
        remembered.only(&[6]);
        assert!(remembered.of(5).is_none() && remembered.of(6).is_some());
        // Never more than fit.
        let many: Vec<u64> = (100..200).collect();
        for window in &many {
            remembered.keep(*window, picture(1));
        }
        remembered.only(&many);
        assert_eq!(remembered.pictures.len(), Remembered::KEPT);
        assert!(remembered.of(199).is_some() && remembered.of(100).is_none());
    }

    /// The overview drawn into a picture, to be looked at:
    ///
    ///     GEVREK_LOOK_OUT=overview.png cargo test -p gevrek-shell \
    ///         overview_is_drawn -- --ignored
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_overview_is_drawn_to_be_looked_at() {
        let out = std::path::PathBuf::from(
            std::env::var("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"),
        );
        let displays = displays(&state());
        let mut remembered = Remembered::default();
        // (Pictures of two of the windows: a plain colour each.)
        for (window, colour) in [(21u64, [70u8, 110, 190]), (11, [190, 120, 60])] {
            let mut rgba = Vec::new();
            for _ in 0..(64 * 40) {
                rgba.extend_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
            remembered.keep(
                window,
                Picture {
                    width: 64,
                    height: 40,
                    rgba,
                },
            );
        }
        let size = Overview::size(&displays, (2560.0, 1440.0));
        let mut painter = gevrek_ui::picture::Painter::sized(size)
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut overview = Overview::default();
        let mut pixels = Vec::new();
        for step in 0..4 {
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.4, Vec::new(), |ui| {
                overview.draw(ui, &palette, &displays, &remembered);
            });
        }
        painter.save(&pixels, &out).expect("the picture is written");
        println!("drawn: {} ({} by {})", out.display(), size.0, size.1);
    }

    #[test]
    fn it_draws_without_a_window_and_a_key_goes_somewhere() {
        let displays = displays(&state());
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        let mut overview = Overview::default();
        let frame = |overview: &mut Overview, keys: &[egui::Key]| {
            let events = keys
                .iter()
                .map(|key| egui::Event::Key {
                    key: *key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect();
            let mut asked = Asked::default();
            let mut drawn = context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    asked = overview.draw(ui, &palette, &displays, &Remembered::default());
                },
            );
            drawn.textures_delta.clear();
            asked
        };
        assert_eq!(frame(&mut overview, &[]), Asked::default());
        // Right, and Enter: the workspace after the one shown.
        assert_eq!(frame(&mut overview, &[egui::Key::ArrowRight]), Asked::default());
        assert_eq!(
            frame(&mut overview, &[egui::Key::Enter]).go,
            Some(Go::Space("Monitor A".to_string(), true, 2, "three".to_string()))
        );
        // A digit: that workspace of the display with the keyboard.
        assert_eq!(
            frame(&mut overview, &[egui::Key::Num1]).go,
            Some(Go::Space("Monitor A".to_string(), true, 0, "one".to_string()))
        );
        assert!(frame(&mut overview, &[egui::Key::Escape]).close);
    }
}
