//! The fonts card of the Appearance page: the font of the interface, of
//! monospace text and of symbols, the font of each area of the desktop
//! that wants its own, and the font of the stack bar's tabs.
//!
//! The first are kept in the appearance file (`gevrek_ui::fonts`), which
//! every program of gevrek reads as it starts; the stack bar's is a
//! setting of the configuration, as the rest of the stack bar.

use gevrek_ui::fonts::{self, Area, Choice, Installed};
use gevrek_ui::{icons, theme, widgets};
use serde_json::Value;

use crate::document::Document;

/// Where the stack bar's font is kept in the configuration.
pub const STACKBAR_FONT: &str = "/desktop/appearance/stackbar/font";

/// What was picked in a list of fonts.
enum Pick {
    Default,
    Builtin,
    Font(&'static Installed),
}

#[derive(Default)]
pub struct FontsCard {
    /// What is typed in the search field of the list that is open.
    search: String,
    /// The frame a list was last drawn in: one not drawn in the frame
    /// before has just opened.
    drawn: u64,
}

impl FontsCard {
    /// Draws the card. Returns whether the appearance's fonts changed (the
    /// stack bar's goes into `document`, as any setting does).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        appearance: &mut gevrek_ui::Settings,
        document: &mut Document,
        highlight: Option<&str>,
    ) -> bool {
        let before = appearance.fonts.clone();
        let chosen = &mut appearance.fonts;
        let interface = if appearance.system_font {
            "The desktop's"
        } else {
            "gevrek's built-in"
        };
        for (label, help, choice, default, builtin) in [
            (
                "Interface",
                "The font of every area that has none of its own below.",
                &mut chosen.text,
                interface,
                Some("gevrek's built-in"),
            ),
            (
                "Monospace",
                "Where text is written in letters of one width (keys, paths, the raw file).",
                &mut chosen.mono,
                interface,
                Some("gevrek's built-in"),
            ),
            (
                "Symbols",
                "Draws only the signs no other font has, as ⏻ ⌘ ★ in a window's title.",
                &mut chosen.symbols,
                "The system's symbol font",
                Some("None"),
            ),
        ] {
            let shown = fonts::shown_name(choice, default);
            let (pick, reset) =
                widgets::row(ui, label, help, *choice != Choice::Default, false, |ui| {
                    self.picker(ui, label, &shown, default, builtin)
                });
            if reset {
                *choice = Choice::Default;
            }
            if let Some(pick) = pick {
                *choice = to_choice(pick);
            }
        }
        ui.add_space(6.0);
        widgets::caption(
            ui,
            "Each area in the interface's font, or a font of its own.",
        );
        for area in Area::ALL {
            let choice = chosen.area(area).clone();
            let default = "As the interface";
            let shown = fonts::shown_name(&choice, default);
            let (pick, reset) = widgets::row(
                ui,
                area.name(),
                "",
                choice != Choice::Default,
                false,
                |ui| self.picker(ui, area.key(), &shown, default, Some("gevrek's built-in")),
            );
            if reset {
                chosen.set_area(area, Choice::Default);
            }
            if let Some(pick) = pick {
                chosen.set_area(area, to_choice(pick));
            }
        }
        // The stack bar's tabs: drawn by the window manager, which knows a
        // font by its family.
        let family = document
            .get(STACKBAR_FONT)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let default = "The system's";
        let shown = if family.is_empty() {
            default
        } else {
            family.as_str()
        };
        let (pick, reset) = widgets::row(
            ui,
            "Stack bar tabs",
            "The titles on the tabs over a stack of windows, drawn by the window manager.",
            !family.is_empty(),
            highlight == Some(STACKBAR_FONT),
            |ui| self.picker(ui, "stackbar", shown, default, None),
        );
        if reset {
            document.reset(STACKBAR_FONT);
        }
        match pick {
            Some(Pick::Font(font)) => {
                document.set(STACKBAR_FONT, Value::String(font.family.clone()));
            }
            Some(Pick::Default | Pick::Builtin) => {
                document.set(STACKBAR_FONT, Value::String(String::new()));
            }
            None => {}
        }
        ui.add_space(4.0);
        widgets::caption(
            ui,
            "The panel, the lock screen and the other surfaces take a font as they next start; the stack bar takes it when the settings are applied.",
        );
        appearance.fonts != before
    }

    /// The font shown, and the list of the system's fonts when it is
    /// pressed, with a field to look for one by its name. `builtin`: the
    /// name of the choice of egui's own font, where there is one.
    fn picker(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        shown: &str,
        default: &str,
        builtin: Option<&str>,
    ) -> Option<Pick> {
        let mut pick = None;
        egui::ComboBox::from_id_salt(("font", id))
            .selected_text(shown)
            .width(260.0)
            .height(340.0)
            .show_ui(ui, |ui| {
                let frame = ui.ctx().cumulative_frame_nr();
                let opened = self.drawn + 1 < frame;
                self.drawn = frame;
                if opened {
                    self.search.clear();
                }
                let field = widgets::search_field(ui, &mut self.search, "Look for a font", 240.0);
                if opened {
                    field.request_focus();
                }
                ui.separator();
                let needle = self.search.to_lowercase();
                if needle.is_empty() {
                    if ui.selectable_label(false, default).clicked() {
                        pick = Some(Pick::Default);
                    }
                    if let Some(builtin) = builtin {
                        if ui.selectable_label(false, builtin).clicked() {
                            pick = Some(Pick::Builtin);
                        }
                    }
                    ui.separator();
                }
                let families = fonts::families();
                if families.is_empty() {
                    let muted = theme::pal(ui.ctx()).muted;
                    ui.colored_label(
                        muted,
                        format!("{}  No font of the system's was found.", icons::INFO),
                    );
                }
                for font in families {
                    if !needle.is_empty() && !font.family.to_lowercase().contains(&needle) {
                        continue;
                    }
                    if ui
                        .selectable_label(font.family == shown, &font.family)
                        .clicked()
                    {
                        pick = Some(Pick::Font(font));
                    }
                }
            });
        pick
    }
}

fn to_choice(pick: Pick) -> Choice {
    match pick {
        Pick::Default => Choice::Default,
        Pick::Builtin => Choice::Builtin,
        Pick::Font(font) => Choice::File(font.path.clone()),
    }
}
