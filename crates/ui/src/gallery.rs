//! Every widget of the design system on one page: to look at while a
//! palette or a widget is changed, and what the tests draw in every
//! palette and density to prove that nothing in it panics or goes missing.

use crate::theme::{self, icons};
use crate::widgets;
use egui::Ui;

/// What the gallery's own controls hold.
#[derive(Debug, Clone)]
pub struct Gallery {
    pub switch: bool,
    pub checkbox: bool,
    pub gaps: f32,
    pub count: u32,
    pub layout: &'static str,
    pub colour: String,
    pub text: String,
    /// A span of the day, a time, and a value along a painted track.
    pub from: u32,
    pub to: u32,
    pub time: u32,
    pub warmth: f32,
    pub speed: f32,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            switch: true,
            checkbox: false,
            gaps: 12.0,
            count: 3,
            layout: "Dwindle",
            colour: "#40e0d0".to_string(),
            text: "kitty --hold".to_string(),
            from: 21 * 60,
            to: 6 * 60 + 30,
            time: 8 * 60 + 15,
            warmth: 3400.0,
            speed: 1.0,
        }
    }
}

impl Gallery {
    /// Draw every widget once.
    pub fn show(&mut self, ui: &mut Ui) {
        let palette = theme::pal(ui.ctx());
        widgets::heading(ui, "Design system", "Every widget, in the palette in use.");

        widgets::card(ui, "Status", |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::pill(ui, icons::CHECK_CIRCLE, "valid", palette.ok);
                widgets::pill(ui, icons::WARNING, "2 warnings", palette.warn);
                widgets::pill(ui, icons::X_CIRCLE, "1 error", palette.error);
                widgets::pill(ui, icons::PLUGS_CONNECTED, "connected", palette.accent_fg());
                widgets::dot(ui, true, palette.accent_fg());
                widgets::dot(ui, false, palette.faint);
            });
            ui.horizontal(|ui| {
                widgets::kbd(ui, "Super");
                widgets::kbd(ui, "Shift");
                widgets::kbd(ui, "T");
            });
        });

        widgets::card(ui, "Controls", |ui| {
            widgets::caption(ui, "Settings rows");
            let (_, reset) =
                widgets::row(ui, "Gaps", "Space between windows.", true, false, |ui| {
                    ui.add(widgets::Slider::new(&mut self.gaps, 0.0..=64.0).suffix(" px"))
                });
            if reset {
                self.gaps = 12.0;
            }
            widgets::row(ui, "Windows in the master area", "", false, false, |ui| {
                ui.add(widgets::Slider::new(&mut self.count, 1..=8))
            });
            widgets::row(
                ui,
                "Follow the mouse",
                "The window under the pointer gets the focus.",
                self.switch,
                true,
                |ui| widgets::switch(ui, &mut self.switch),
            );
            widgets::row(ui, "Layout", "", true, false, |ui| {
                let options = [
                    ("Dwindle", "Dwindle"),
                    ("Columns", "Columns"),
                    ("Grid", "Grid"),
                ];
                if let Some(picked) = widgets::segmented(
                    ui,
                    ui.scope_id().with("layout"),
                    Some(&self.layout),
                    &options,
                    false,
                ) {
                    self.layout = picked;
                }
            });
            widgets::row(ui, "Border colour", "", true, false, |ui| {
                if let Some(colour) =
                    widgets::color_field(ui, ui.scope_id().with("colour"), Some(&self.colour))
                {
                    self.colour = colour;
                }
            });
            widgets::row(ui, "Program", "What Super+T starts.", true, false, |ui| {
                ui.text_edit_singleline(&mut self.text)
            });
            widgets::checkbox(ui, &mut self.checkbox, "Run startup programs");
        });

        widgets::card(ui, "Times and tracks", |ui| {
            use crate::controls::{self, DayBand, Stop, StopSlider};
            widgets::caption(ui, "A span of the day");
            controls::day_band(
                ui,
                &mut self.from,
                &mut self.to,
                &DayBand {
                    now: Some(15 * 60 + 40),
                    ..DayBand::default()
                },
            );
            ui.horizontal(|ui| {
                ui.label("From");
                controls::time_field(ui, &mut self.from);
                ui.label("until");
                controls::time_field(ui, &mut self.to);
                ui.label(
                    egui::RichText::new(controls::span_words(controls::span(self.from, self.to)))
                        .color(palette.faint),
                );
            });
            widgets::row(ui, "A time", "Its hour and its minute, each chosen.", true, false, |ui| {
                controls::time_field(ui, &mut self.time)
            });
            widgets::caption(ui, "A track that shows what its values are, with set points");
            let warm = |value: f32| {
                let part = ((value - 1500.0) / 5000.0).clamp(0.0, 1.0);
                theme::mix(
                    egui::Color32::from_rgb(255, 130, 20),
                    egui::Color32::WHITE,
                    part.sqrt(),
                )
            };
            let warmths = [
                Stop {
                    value: 2000.0,
                    name: "Candle",
                },
                Stop {
                    value: 3400.0,
                    name: "Warm",
                },
                Stop {
                    value: 5000.0,
                    name: "Mild",
                },
            ];
            ui.add(
                StopSlider::new(&mut self.warmth, 1500.0..=6500.0)
                    .stops(&warmths)
                    .painted(&warm)
                    .suffix(" K")
                    .step(50.0),
            );
            widgets::caption(ui, "The same in the accent, where a value has no colour");
            let speeds = [
                Stop {
                    value: 0.5,
                    name: "Slow",
                },
                Stop {
                    value: 1.0,
                    name: "Usual",
                },
                Stop {
                    value: 2.0,
                    name: "Fast",
                },
            ];
            ui.add(
                StopSlider::new(&mut self.speed, 0.25..=3.0)
                    .stops(&speeds)
                    .suffix("x")
                    .step(0.05),
            );
        });

        widgets::card(ui, "On a surface with little room", |ui| {
            use crate::controls::{self, Tile};
            ui.set_max_width(360.0);
            ui.horizontal(|ui| {
                for (icon, name, state, on, more) in [
                    (icons::WIFI_HIGH, "Wireless", "home-5G", true, Some(false)),
                    (icons::MOON, "Night light", "Off", false, None),
                ] {
                    controls::tile(
                        ui,
                        egui::vec2(172.0, 56.0),
                        &Tile {
                            icon,
                            name,
                            state,
                            on,
                            more,
                            busy: false,
                        },
                    );
                }
            });
            controls::level(
                ui,
                icons::SPEAKER_HIGH,
                "Mute",
                None,
                &mut self.gaps,
                0.0..=64.0,
                Some("62 %"),
            );
            controls::level(
                ui,
                icons::SPEAKER_LOW,
                "Mute this program",
                Some("A program"),
                &mut self.gaps,
                0.0..=64.0,
                None,
            );
            controls::chosen_line(ui, "Plays on", "Speakers (a device with a long name)", true);
            controls::list_under(ui, "gallery devices", 3, 6, |ui| {
                controls::choice_row(ui, None, "Headphones", None, false);
                controls::choice_row(ui, None, "Speakers (a device with a long name)", None, true);
                controls::choice_row(ui, Some(icons::WIFI_LOW), "A network", Some(icons::LOCK), false);
            });
        });

        widgets::card(ui, "Fields", |ui| {
            widgets::row(ui, "A text", "", true, false, |ui| {
                widgets::text_field(ui, &mut self.text, 260.0)
            });
            widgets::row(ui, "To search by", "", false, false, |ui| {
                let mut nothing = String::new();
                widgets::search_field(ui, &mut nothing, "A setting", 260.0)
            });
            widgets::row(ui, "A folder", "Said whether it is there.", true, false, |ui| {
                let mut here = ".".to_string();
                widgets::path_field(ui, &mut here, widgets::PathKind::Folder, None, 260.0)
            });
            widgets::row(ui, "A file that is missing", "", true, false, |ui| {
                let mut gone = "no/such/picture.png".to_string();
                widgets::path_field(ui, &mut gone, widgets::PathKind::File, None, 260.0)
            });
            widgets::row(ui, "A number alone", "", false, false, |ui| {
                widgets::number_box(ui, &mut self.count, 1.0)
            });
            widgets::row(ui, "One of many", "", true, false, |ui| {
                let options: Vec<(&str, String, String)> = ["Dwindle", "Columns", "Grid", "Spiral", "Monocle"]
                    .iter()
                    .map(|name| (*name, (*name).to_string(), String::new()))
                    .collect();
                if let Some(picked) = widgets::dropdown(ui, "gallery layout", &self.layout, &options, 200.0) {
                    self.layout = picked;
                }
            });
            widgets::banner(ui, palette.warn, icons::WARNING, |ui| {
                ui.label("This configuration is on trial for 12 more seconds.");
                widgets::secondary_button(ui, "Keep");
                widgets::busy(ui);
            });
        });

        widgets::card(ui, "Buttons", |ui| {
            ui.horizontal(|ui| {
                widgets::primary_button(ui, true, "Apply");
                widgets::primary_button(ui, false, "Applied");
                widgets::secondary_button(ui, "Secondary");
                widgets::text_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "As it comes");
                widgets::icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Undo");
                widgets::small_icon_button(ui, icons::TRASH, "Delete");
                ui.add_enabled(false, widgets::icon_only(icons::ARROW_CLOCKWISE));
            });
        });

        widgets::card(ui, "Text", |ui| {
            ui.label("Body text in the palette's text colour.");
            ui.label(egui::RichText::new("Muted, for what explains.").color(palette.muted));
            ui.label(egui::RichText::new("Faint, for what is off.").color(palette.faint));
            ui.monospace("{\"command\": \"exec\", \"program\": \"kitty\"}");
            ui.label(widgets::ellipsize(
                "A long window title that does not fit where it is shown and is cut",
                180.0,
            ));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Density, Preset, Settings};

    /// One pass of the gallery in a context of its own, as a window of the
    /// given size. Returns how many shapes it drew.
    fn draw(settings: &Settings, system: egui::Theme) -> usize {
        let context = egui::Context::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(system, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, settings);
        let mut gallery = Gallery::default();
        let mut shapes = 0;
        // Twice: the first pass only measures.
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 900.0),
                )),
                ..Default::default()
            };
            let output = context.run_ui(input, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| gallery.show(ui));
            });
            let mut output = output;
            // No renderer takes the font texture in this test.
            output.textures_delta.clear();
            shapes = output.shapes.len();
        }
        shapes
    }

    #[test]
    fn every_widget_draws_in_every_palette_and_density() {
        for preset in Preset::ALL {
            for density in [Density::Comfortable, Density::Compact] {
                for system in [egui::Theme::Dark, egui::Theme::Light] {
                    let settings = Settings {
                        preset,
                        density,
                        ..Settings::default()
                    };
                    let shapes = draw(&settings, system);
                    assert!(
                        shapes > 50,
                        "{preset:?} {density:?} {system:?}: only {shapes} shapes"
                    );
                }
            }
        }
    }

    #[test]
    fn text_is_readable_on_its_background_in_every_palette() {
        for preset in Preset::ALL {
            for system in [egui::Theme::Dark, egui::Theme::Light] {
                let palette = preset.palette(system);
                for (name, background) in [
                    ("bg", palette.bg),
                    ("surface", palette.surface),
                    ("chrome", palette.chrome),
                ] {
                    let ratio = theme::contrast(palette.text, background);
                    // WCAG AA for body text; large-text level for what only explains.
                    assert!(
                        ratio >= 4.5,
                        "{preset:?} {system:?}: text on {name} is {ratio:.1}:1"
                    );
                    let muted = theme::contrast(palette.muted, background);
                    assert!(
                        muted >= 3.0,
                        "{preset:?} {system:?}: muted on {name} is {muted:.1}:1"
                    );
                }
                let accent = theme::contrast(palette.accent_fg(), palette.bg);
                assert!(
                    accent >= 3.0,
                    "{preset:?} {system:?}: accent on bg is {accent:.1}:1"
                );
            }
        }
    }

    #[test]
    fn preferences_survive_being_saved_and_read() {
        let folder = std::env::temp_dir().join(format!("gevrek-ui-{}", std::process::id()));
        let path = folder.join("studio.conf");
        use crate::fonts::{Area, Choice};
        let mut fonts = crate::fonts::Fonts {
            text: Choice::File("C:\\Windows\\Fonts\\Segoe UI.ttf".into()),
            mono: Choice::Builtin,
            ..Default::default()
        };
        fonts.set_area(Area::Panel, Choice::File("/usr/share/fonts/a b/Inter.otf".into()));
        fonts.set_area(Area::LockScreen, Choice::Builtin);
        let saved = Settings {
            preset: Preset::TokyoNight,
            accent: crate::Accent::Custom(egui::Color32::from_rgb(255, 128, 0)),
            density: Density::Compact,
            radius: 3,
            system_font: false,
            custom_titlebar: false,
            reduce_motion: false,
            fonts,
        };
        saved.save(&path).unwrap();
        assert_eq!(Settings::load(&path), saved);
        assert_eq!(
            Settings::load(&folder.join("missing.conf")),
            Settings::default()
        );
        std::fs::remove_dir_all(folder).unwrap();
    }
}
