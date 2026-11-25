//! The list of shortcuts as a surface of the shell: what to press and
//! what it does, as the running configuration has it now, to search by
//! typing.
//!
//! The list is the model's (`gevrek_model::shortcuts::shortcuts_of`) and
//! so is the search (`filtered`); the settings program's window
//! (`gevrek-studio --shortcuts`) shows the same where the shell does not
//! run. On Windows the window manager asks the shell for it on the key
//! for the list; on Linux that key still opens the window.

use gevrek_model::shortcuts::{filtered, super_key_name, Shortcut, ShortcutSource};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;

/// The size of the surface, in logical pixels (less where the display
/// has less).
pub const SIZE: (u32, u32) = (640, 720);

/// How much of the list is shown for what is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// The lines that answer what is typed, and all there are.
    pub lines: usize,
    pub all: usize,
    /// Of each, those whose keys have the Super key in them.
    pub with_super: usize,
    pub super_of_all: usize,
}

pub fn counts(list: &[Shortcut], typed: &str) -> Counts {
    let name = super_key_name().to_lowercase();
    let has_super = |shortcut: &Shortcut| shortcut.keys.to_lowercase().contains(&name);
    let shown = filtered(list, typed);
    Counts {
        lines: shown.len(),
        all: list.len(),
        with_super: shown.iter().filter(|shortcut| has_super(shortcut)).count(),
        super_of_all: list.iter().filter(|shortcut| has_super(shortcut)).count(),
    }
}

fn section(ui: &mut egui::Ui, title: &str, rows: &[&Shortcut]) {
    if rows.is_empty() {
        return;
    }
    ui.add_space(10.0);
    widgets::caption(ui, title);
    ui.add_space(2.0);
    for shortcut in rows {
        let mode = (shortcut.submap != "default").then_some(shortcut.submap.as_str());
        widgets::shortcut_row(ui, &shortcut.keys, &shortcut.action, mode);
    }
}

/// What the list says of the build it is drawn by: the commit, so that
/// whoever opens it knows what is running.
pub fn build_line() -> String {
    format!("build {}", gevrek_build::COMMIT)
}

/// Draw the list with its search. `list` is `None` while the window
/// manager has not said what the configuration is. What is typed goes to
/// the search without a click first. (Escape is the surface's to see to.)
pub fn draw(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    list: Option<&[Shortcut]>,
    typed: &mut String,
) {
    let context = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.bg)
                .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                .corner_radius(14)
                .inner_margin(egui::Margin::symmetric(18, 12)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Shortcuts")
                        .size(20.0)
                        .strong()
                        .color(palette.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Esc closes")
                            .color(palette.faint)
                            .size(12.0),
                    );
                    // What is running: the commit this was built from.
                    ui.label(
                        egui::RichText::new(build_line())
                            .color(palette.faint)
                            .size(12.0)
                            .monospace(),
                    );
                });
            });
            let Some(list) = list else {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Asking the window manager…").color(palette.muted),
                );
                return;
            };
            if list.is_empty() {
                ui.add_space(8.0);
                ui.label("The configuration assigns no keys.");
                return;
            }
            ui.add_space(6.0);
            let field = ui.add(
                egui::TextEdit::singleline(typed)
                    .hint_text(format!("{}  Type to search", icons::MAGNIFYING_GLASS))
                    .desired_width(f32::INFINITY),
            );
            if !field.has_focus() && !context.egui_wants_keyboard_input() {
                field.request_focus();
            }
            let rows = filtered(list, typed);
            let of = |source: ShortcutSource| -> Vec<&Shortcut> {
                rows.iter()
                    .copied()
                    .filter(|shortcut| shortcut.source == source)
                    .collect()
            };
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if rows.is_empty() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Nothing matches.").color(palette.muted));
                    }
                    section(ui, "Key bindings", &of(ShortcutSource::Binding));
                    section(ui, "Automation graph", &of(ShortcutSource::Graph));
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(
                            "To change them: gevrek studio, Key bindings (or the key nodes of the graph).",
                        )
                        .color(palette.faint)
                        .size(12.0),
                    );
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::shortcuts::shortcuts_of;

    #[test]
    fn what_is_typed_narrows_the_list_and_the_super_key_is_found_by_its_names() {
        let list = shortcuts_of(&gevrek_model::examples::starting_configuration());
        let all = counts(&list, "");
        assert_eq!(all.lines, all.all);
        assert_eq!(all.with_super, all.super_of_all);
        assert!(all.super_of_all > 0);
        let narrowed = counts(&list, "close");
        assert!(narrowed.lines > 0 && narrowed.lines < all.all);
        // "Super" in the documentation, "Win" on the keyboard: either
        // finds every chord with that key.
        for name in ["super", "win"] {
            let found = counts(&list, name);
            assert_eq!(found.with_super, all.super_of_all, "{name}");
        }
        assert_eq!(counts(&list, "no such thing").lines, 0);
    }

    #[test]
    fn a_long_description_is_broken_into_lines_within_the_window() {
        let mut list = shortcuts_of(&gevrek_model::examples::starting_configuration());
        let long = "Move the chosen items to the zone on the right, and keep them in \
                    the order they had, even where that zone sorts by name";
        list[0].action = long.to_string();
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        let mut typed = String::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SIZE.0 as f32, SIZE.1 as f32),
            )),
            ..Default::default()
        };
        // (The first frame only measures; the second is what is seen.)
        let mut first = context.run_ui(input.clone(), |ui| {
            draw(ui, &palette, Some(&list), &mut typed)
        });
        first.textures_delta.clear();
        let mut output = context.run_ui(input, |ui| draw(ui, &palette, Some(&list), &mut typed));
        output.textures_delta.clear();
        // Every word drawn lies inside the window; the long one is on
        // more than one line.
        let mut lines_of_it = 0;
        for clipped in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape {
                if text.galley.job.text.is_empty() {
                    continue;
                }
                // (Where it is seen: a galley set to the right reaches
                // left of its place.)
                let right = clipped.shape.visual_bounding_rect().right();
                assert!(
                    right <= SIZE.0 as f32,
                    "drawn past the window: {right} {:?}",
                    text.galley.job.text
                );
                if text.galley.job.text.starts_with("Move the chosen items") {
                    lines_of_it = text.galley.rows.len();
                }
            }
        }
        assert!(
            lines_of_it > 1,
            "the long description was not broken: {lines_of_it}"
        );
    }

    #[test]
    fn the_list_is_drawn_and_what_is_typed_goes_to_the_search() {
        let list = shortcuts_of(&gevrek_model::examples::starting_configuration());
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        let mut typed = String::new();
        let mut frame = |events: Vec<egui::Event>, list: Option<&[Shortcut]>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(SIZE.0 as f32, SIZE.1 as f32),
                )),
                focused: true,
                events,
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| draw(ui, &palette, list, &mut typed));
            output.textures_delta.clear();
        };
        // Not told yet, an empty list, and the list.
        frame(Vec::new(), None);
        frame(Vec::new(), Some(&[]));
        frame(Vec::new(), Some(&list));
        // The search has the keyboard from the second frame on.
        frame(vec![egui::Event::Text("clo".to_string())], Some(&list));
        frame(Vec::new(), Some(&list));
        assert_eq!(typed, "clo");
    }
}
