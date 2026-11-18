//! Which of the tray's icons stand on the panel, and the list of all of
//! them: the same on every system.
//!
//! Programs put icons into the tray as they please, and a panel that shows
//! every one of them is soon full of what nobody looks at. So an icon
//! stands on the panel only when the person put it there; the tray's own
//! button (an arrowhead after the icons) opens the list of all of them,
//! where each has a switch for it and can be pressed as on the panel. What
//! was chosen is written down by the icons' lasting names and holds from
//! one session to the next.

use crate::tray::Item;
use gevrek_ui::theme;
use std::collections::{HashMap, HashSet};

/// What the tray's own button is known by among the icons.
pub(crate) const TRAY_MORE: &str = "gevrek:more";

/// Where the names of the icons that stand on the panel are written down.
pub(crate) fn tray_choice_file() -> std::path::PathBuf {
    gevrek_model::paths::state_dir().join("tray-on-panel.json")
}

/// The icons the person put on the panel, as written down; none where
/// nothing was written yet (an icon is not on the panel until it is put
/// there).
pub(crate) fn read_tray_choice() -> HashSet<String> {
    read_tray_choice_in(&tray_choice_file())
}

/// As `read_tray_choice`, from the file that is named.
fn read_tray_choice_in(file: &std::path::Path) -> HashSet<String> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|names| names.into_iter().collect())
        .unwrap_or_default()
}

pub(crate) fn keep_tray_choice(on_panel: &HashSet<String>) {
    let file = tray_choice_file();
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let mut names: Vec<&String> = on_panel.iter().collect();
    names.sort();
    if let Ok(text) = serde_json::to_string_pretty(&names) {
        if let Err(error) = std::fs::write(&file, text) {
            log::warn!("which icons stand on the panel was not written down: {error}");
        }
    }
}

/// The picture of the tray's own button: an arrowhead that points up, in
/// the colour of the panel's words.
pub(crate) fn tray_more_picture(colour: (u8, u8, u8)) -> gevrek_system::icons::Picture {
    const SIDE: usize = 32;
    let mut pixels = vec![0u8; SIDE * SIDE * 4];
    // Two strokes from the middle of the upper part down to the sides.
    let distance = |x: f32, y: f32, from: (f32, f32), to: (f32, f32)| -> f32 {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let along = (((x - from.0) * dx + (y - from.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((x - from.0 - along * dx).powi(2) + (y - from.1 - along * dy).powi(2)).sqrt()
    };
    for row in 0..SIDE {
        for column in 0..SIDE {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let near = distance(x, y, (16.0, 11.0), (7.0, 20.0)).min(distance(
                x,
                y,
                (16.0, 11.0),
                (25.0, 20.0),
            ));
            // A stroke three pixels wide, soft at its edge.
            let solid = (2.4 - near).clamp(0.0, 1.0);
            let at = (row * SIDE + column) * 4;
            pixels[at..at + 4].copy_from_slice(&[
                colour.0,
                colour.1,
                colour.2,
                (solid * 255.0) as u8,
            ]);
        }
    }
    gevrek_system::icons::Picture {
        width: SIDE as u32,
        height: SIDE as u32,
        pixels,
    }
}

/// What was done in the list of the tray's icons.
pub(crate) enum TrayAsk {
    Nothing,
    /// This icon (by its lasting name) is to stand on the panel, or not.
    Stand(String, bool),
    /// This icon (by its key) was pressed; `true`: for its menu.
    Press(String, bool),
}

/// How high a line of the list is, and how many are shown before the list
/// is scrolled, in points.
pub(crate) const TRAY_ROW: f32 = 32.0;
pub(crate) const TRAY_ROWS: usize = 14;

/// How large the list of the tray's icons is for so many icons.
pub(crate) fn tray_list_size(count: usize) -> egui::Vec2 {
    egui::vec2(340.0, 66.0 + count.clamp(1, TRAY_ROWS) as f32 * TRAY_ROW)
}

/// The list of every icon of the tray: its picture and words (a press
/// there is a press on the icon), and a switch for whether it stands on
/// the panel.
pub(crate) fn tray_list_draw(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    all: &[(Item, String)],
    on_panel: &HashSet<String>,
    pictures: &mut HashMap<String, (u64, egui::TextureHandle)>,
) -> TrayAsk {
    let mut asked = TrayAsk::Nothing;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                .corner_radius(10)
                .inner_margin(10),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Icons").strong().color(palette.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("on the panel")
                            .size(11.5)
                            .color(palette.faint),
                    );
                });
            });
            ui.add_space(4.0);
            if all.is_empty() {
                ui.label(egui::RichText::new("No program has an icon.").color(palette.muted));
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (item, name) in all {
                    let stands = on_panel.contains(name);
                    ui.horizontal(|ui| {
                        ui.set_height(TRAY_ROW - 4.0);
                        // The switch first, at the right end; what is left
                        // of the line is the icon's.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut now = stands;
                            if gevrek_ui::widgets::switch(ui, &mut now).changed() {
                                asked = TrayAsk::Stand(name.clone(), now);
                            }
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let side = 18.0;
                                    if let Some(picture) = &item.picture {
                                        let texture = match pictures.get(&item.key) {
                                            Some((stamp, texture)) if *stamp == item.stamp => {
                                                texture.clone()
                                            }
                                            _ => {
                                                let texture = ui.ctx().load_texture(
                                                    format!("tray-list-{}", item.key),
                                                    egui::ColorImage::from_rgba_unmultiplied(
                                                        [
                                                            picture.width as usize,
                                                            picture.height as usize,
                                                        ],
                                                        &picture.pixels,
                                                    ),
                                                    egui::TextureOptions::LINEAR,
                                                );
                                                pictures.insert(
                                                    item.key.clone(),
                                                    (item.stamp, texture.clone()),
                                                );
                                                texture
                                            }
                                        };
                                        ui.add(egui::Image::new((
                                            texture.id(),
                                            egui::vec2(side, side),
                                        )));
                                    } else {
                                        ui.add_space(side);
                                    }
                                    // (Its words, or its program's name where it has none.)
                                    let words = item.hint.lines().next().unwrap_or_default().trim();
                                    let words = if words.is_empty() {
                                        name.as_str()
                                    } else {
                                        words
                                    };
                                    let line = ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(words).color(palette.text),
                                        )
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                    );
                                    if line.clicked() {
                                        asked = TrayAsk::Press(item.key.clone(), false);
                                    } else if line.secondary_clicked() {
                                        asked = TrayAsk::Press(item.key.clone(), true);
                                    }
                                },
                            );
                        });
                    });
                }
            });
        });
    asked
}

/// What the panel shows of the tray: the icons the person put there, in
/// the order programs gave them, and the tray's own button after them
/// (while there is any icon at all). `colour`: the colour of the panel's
/// words, which the button's arrowhead has.
pub(crate) fn for_panel(
    all: &[(Item, String)],
    on_panel: &HashSet<String>,
    colour: (u8, u8, u8),
) -> Vec<Item> {
    let mut items: Vec<Item> = all
        .iter()
        // (One whose program has nothing to say now stays in the list;
        // one that wants to be looked at stands there, chosen or not.)
        .filter(|(item, name)| (on_panel.contains(name) && !item.passive) || item.attention)
        .map(|(item, _)| item.clone())
        .collect();
    if !all.is_empty() {
        let hidden = all.len() - items.len();
        items.push(Item {
            key: TRAY_MORE.to_string(),
            name: TRAY_MORE.to_string(),
            title: "Icons".to_string(),
            hint: if hidden == 0 {
                "The icons of programs".to_string()
            } else {
                format!("The icons of programs ({hidden} more)")
            },
            picture: Some(std::sync::Arc::new(tray_more_picture(colour))),
            // (Another picture when the colour is another.)
            stamp: u64::from(colour.0) << 16 | u64::from(colour.1) << 8 | u64::from(colour.2),
            menu_only: true,
            passive: false,
            attention: false,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(name: &str) -> (Item, String) {
        (
            Item {
                key: format!("key-of-{name}"),
                name: name.to_string(),
                title: name.to_string(),
                hint: String::new(),
                picture: None,
                stamp: 0,
                menu_only: false,
                passive: false,
                attention: false,
            },
            name.to_string(),
        )
    }

    #[test]
    fn only_the_icons_put_on_the_panel_stand_there_with_the_trays_own_button() {
        let all = vec![icon("mail"), icon("chat"), icon("sync")];
        // Nothing chosen: the button alone, which says how many there are.
        let none = for_panel(&all, &HashSet::new(), (200, 200, 200));
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].key, TRAY_MORE);
        assert!(none[0].hint.contains("3 more"), "{}", none[0].hint);
        // One chosen: it, in the programs' order, and the button last.
        let chosen: HashSet<String> = ["sync".to_string(), "mail".to_string()].into();
        let some = for_panel(&all, &chosen, (200, 200, 200));
        let keys: Vec<&str> = some.iter().map(|item| item.key.as_str()).collect();
        assert_eq!(keys, ["key-of-mail", "key-of-sync", TRAY_MORE]);
        assert!(some[2].hint.contains("1 more"));
        // An icon that wants to be looked at stands there unchosen; a
        // chosen one that has nothing to say does not.
        let mut states = vec![icon("mail"), icon("chat"), icon("sync")];
        states[1].0.attention = true;
        states[2].0.passive = true;
        let shown = for_panel(&states, &chosen, (200, 200, 200));
        let keys: Vec<&str> = shown.iter().map(|item| item.key.as_str()).collect();
        assert_eq!(keys, ["key-of-mail", "key-of-chat", TRAY_MORE]);
        // No icon at all: no button either.
        assert!(for_panel(&[], &chosen, (0, 0, 0)).is_empty());
        // The list is as large as its icons, up to a point.
        assert!(tray_list_size(3).y < tray_list_size(9).y);
        assert_eq!(tray_list_size(40).y, tray_list_size(TRAY_ROWS).y);
    }

    /// Which icons stand on the panel is a file anyone may have written:
    /// what is no list of names is no choice.
    #[test]
    fn a_choice_from_outside_is_lived_with() {
        let folder =
            std::env::temp_dir().join(format!("gevrek-tray-outside-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let file = folder.join("tray-on-panel.json");
        assert!(read_tray_choice_in(&file).is_empty(), "no file, no choice");
        std::fs::write(&file, r#"["b", "a", "b", ""]"#).expect("written");
        assert_eq!(read_tray_choice_in(&file).len(), 3);
        let long = format!(r#"["{}"]"#, "x".repeat(1_000_000));
        std::fs::write(&file, &long).expect("written");
        assert_eq!(read_tray_choice_in(&file).len(), 1);
        for nonsense in [
            "",
            "null",
            "7",
            "{}",
            r#"{"a": true}"#,
            r#"["a", 1]"#,
            r#"["a", null]"#,
            r#"[["a"]]"#,
            r#"["a""#,
            "[\"\\ud800\"]",
            "\u{feff}[]x",
        ] {
            std::fs::write(&file, nonsense).expect("written");
            assert!(read_tray_choice_in(&file).is_empty(), "{nonsense}");
        }
        std::fs::write(&file, [0xff, 0xfe, 0x00]).expect("written");
        assert!(read_tray_choice_in(&file).is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }
}
