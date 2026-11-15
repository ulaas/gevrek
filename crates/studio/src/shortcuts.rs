//! The list of shortcuts: `gevrek-studio --shortcuts`.
//!
//! A small window the window manager opens on a key (and closes on the
//! same key): what to press and what it does, as the running
//! configuration has it now, to search by typing. The window manager
//! knows the window by its title and keeps it floating in the middle.

use crate::link::{Event, Link, Request};
use egui::RichText;
use gevrek_model::shortcuts::{filtered, shortcuts_of, Shortcut, ShortcutSource};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;

/// The title the window manager knows the window by
/// (`gevrek_engine::outputs::SHORTCUTS_WINDOW_TITLE`).
pub const TITLE: &str = "Gevrek Shortcuts";

struct ShortcutsWindow {
    link: Link,
    /// The list, or why there is none yet.
    listing: Result<Vec<Shortcut>, String>,
    filter: String,
    appearance: gevrek_ui::Settings,
    fonts_ready: bool,
    accent: egui::Color32,
}

impl ShortcutsWindow {
    fn take_in(&mut self) {
        for event in self.link.poll() {
            match event {
                Event::Configuration(config, _) => {
                    let (red, green, blue) = config.desktop.appearance.active_border_color;
                    self.accent = egui::Color32::from_rgb(red, green, blue);
                    self.listing = Ok(shortcuts_of(&config));
                }
                // The list follows the configuration.
                Event::Changed { .. } => self.link.send(Request::Fetch),
                Event::Disconnected(reason) if self.listing.is_err() => {
                    self.listing = Err(format!("The window manager does not answer: {reason}"));
                }
                _ => {}
            }
        }
    }

    fn section(ui: &mut egui::Ui, title: &str, rows: &[&Shortcut]) {
        if rows.is_empty() {
            return;
        }
        let palette = theme::pal(ui.ctx());
        ui.add_space(10.0);
        widgets::caption(ui, title);
        ui.add_space(2.0);
        let height = ui.spacing().interact_size.y + 4.0;
        for shortcut in rows {
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), height),
                egui::Sense::hover(),
            );
            widgets::list_row_bg(ui, &response, rect, false);
            let end = widgets::keycaps(
                ui,
                egui::pos2(rect.left() + 6.0, rect.center().y),
                &shortcut.keys,
                palette.text,
            );
            let at = egui::pos2((end + 14.0).max(rect.left() + 230.0), rect.center().y);
            let after = ui
                .painter()
                .text(
                    at,
                    egui::Align2::LEFT_CENTER,
                    &shortcut.action,
                    egui::FontId::proportional(14.0),
                    palette.text,
                )
                .right();
            if shortcut.submap != "default" {
                ui.painter().text(
                    egui::pos2(after + 12.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("in the {} mode", shortcut.submap),
                    egui::FontId::proportional(12.0),
                    palette.accent_fg(),
                );
            }
        }
    }
}

impl eframe::App for ShortcutsWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.take_in();
        if !self.fonts_ready {
            theme::install_fonts(&context, self.appearance.system_font);
            self.fonts_ready = true;
        }
        let system = context.system_theme().unwrap_or(egui::Theme::Dark);
        let palette = self.appearance.resolve(system, self.accent);
        theme::apply(&context, &palette, &self.appearance);
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg).inner_margin(egui::Margin::symmetric(18, 12)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Shortcuts").size(20.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Esc closes").color(palette.faint).size(12.0));
                        // What is running: the commit this was built from.
                        ui.label(
                            RichText::new(format!("build {}", gevrek_build::COMMIT))
                                .color(palette.faint)
                                .size(12.0)
                                .monospace(),
                        );
                    });
                });
                match &self.listing {
                    Err(problem) if problem.is_empty() => {
                        ui.add_space(8.0);
                        ui.label(RichText::new("Asking the window manager…").color(palette.muted));
                    }
                    Err(problem) => {
                        ui.add_space(8.0);
                        ui.colored_label(palette.warn, problem);
                    }
                    Ok(list) if list.is_empty() => {
                        ui.add_space(8.0);
                        ui.label("The configuration assigns no keys.");
                    }
                    Ok(list) => {
                        ui.add_space(6.0);
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .hint_text(format!("{}  Type to search", icons::MAGNIFYING_GLASS))
                                .desired_width(f32::INFINITY),
                        );
                        // Typing goes to the search without a click first.
                        if !field.has_focus() && !context.egui_wants_keyboard_input() {
                            field.request_focus();
                        }
                        let rows = filtered(list, &self.filter);
                        let of = |source: ShortcutSource| -> Vec<&Shortcut> {
                            rows.iter().copied().filter(|shortcut| shortcut.source == source).collect()
                        };
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            if rows.is_empty() {
                                ui.add_space(8.0);
                                ui.label(RichText::new("Nothing matches.").color(palette.muted));
                            }
                            Self::section(ui, "Key bindings", &of(ShortcutSource::Binding));
                            Self::section(ui, "Automation graph", &of(ShortcutSource::Graph));
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("To change them: gevrek studio, Key bindings (or the key nodes of the graph).")
                                    .color(palette.faint)
                                    .size(12.0),
                            );
                        });
                    }
                }
            });
    }
}

/// Show the list until it is closed.
pub fn run(address: String, appearance: &std::path::Path) -> anyhow::Result<()> {
    let appearance = gevrek_ui::Settings::load(appearance);
    eframe::run_native(
        TITLE,
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("gevrek-shortcuts")
                .with_title(TITLE)
                .with_inner_size([640.0, 740.0])
                .with_min_inner_size([420.0, 300.0])
                .with_always_on_top(),
            centered: true,
            ..Default::default()
        },
        Box::new(move |creation| {
            let wake = creation.egui_ctx.clone();
            Ok(Box::new(ShortcutsWindow {
                link: Link::start(address, move || wake.request_repaint()),
                listing: Err(String::new()),
                filter: String::new(),
                appearance,
                fonts_ready: false,
                accent: egui::Color32::from_rgb(64, 224, 208),
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("the list of shortcuts could not be opened: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The search itself is the model's (`gevrek_model::shortcuts::filtered`),
    // and tested there: the shell's list uses the same.
    #[test]
    fn the_window_lists_what_the_search_gives() {
        let list = shortcuts_of(&gevrek_model::examples::starting_configuration());
        assert_eq!(filtered(&list, "").len(), list.len());
        assert!(filtered(&list, "close").len() < list.len());
    }

    #[test]
    fn the_title_is_the_one_the_window_manager_looks_for() {
        assert_eq!(TITLE, gevrek_engine::outputs::SHORTCUTS_WINDOW_TITLE);
    }
}
