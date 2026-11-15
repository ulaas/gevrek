//! The on-screen display: a few words, or a level, shown for a moment
//! near the bottom of the display, and gone by itself.
//!
//! What it says and how it is drawn is the same on every system; the
//! surface it is drawn on, and what makes it say something, are each
//! system's own.

use gevrek_system::sound::{Device, Level};
use gevrek_ui::theme::{self, icons};

/// What the on-screen display says.
#[derive(Debug, Clone, PartialEq)]
pub struct Osd {
    pub icon: &'static str,
    pub text: String,
    /// A level from 0 to 1, where there is one to show.
    pub level: Option<f32>,
}

/// How long the on-screen display stays after the last thing it said.
pub const STAY: std::time::Duration = std::time::Duration::from_millis(1400);

/// The size of its surface, in logical pixels, and how far it is from
/// the display's edge.
pub const SIZE: (u32, u32) = (300, 64);
pub const MARGIN: i32 = 70;

impl Osd {
    /// Words, without a level.
    pub fn words(icon: &'static str, text: impl Into<String>) -> Self {
        Self {
            icon,
            text: text.into(),
            level: None,
        }
    }

    /// How loud a device is; `None`: nobody could be asked.
    pub fn sound(device: Device, level: Option<Level>) -> Self {
        match level {
            Some(level) => Self {
                icon: match (device, level.muted, level.percent) {
                    (Device::Microphone, true, _) => icons::MICROPHONE_SLASH,
                    (Device::Microphone, false, _) => icons::MICROPHONE,
                    (_, true, _) | (_, _, 0) => icons::SPEAKER_X,
                    (_, _, 1..=45) => icons::SPEAKER_LOW,
                    _ => icons::SPEAKER_HIGH,
                },
                text: if level.muted {
                    "Muted".to_string()
                } else {
                    format!("{} %", level.percent)
                },
                level: Some(if level.muted {
                    0.0
                } else {
                    level.percent.min(100) as f32 / 100.0
                }),
            },
            None => Self::words(
                icons::SPEAKER_X,
                if cfg!(windows) {
                    "No sound device answers"
                } else {
                    "No sound server answers"
                },
            ),
        }
    }

    /// How bright the display is; `None`: this machine has no display
    /// whose brightness can be set, or it could not be set.
    pub fn brightness(now: Option<gevrek_system::brightness::Brightness>) -> Self {
        match now {
            Some(now) => Self {
                icon: icons::SUN,
                text: format!("{} %", now.percent()),
                level: Some(now.percent().min(100) as f32 / 100.0),
            },
            None => Self::words(icons::SUN, "No display here has a brightness to set"),
        }
    }

    /// What a command asked to be shown (`gevrek shell osd`): words, and
    /// a level from 0 to 100 where one was said. A level that is no
    /// number is left out.
    pub fn said(text: &str, level: Option<&str>) -> Self {
        let level = level
            .and_then(|level| level.trim().parse::<f32>().ok())
            .filter(|level| level.is_finite())
            .map(|level| level.clamp(0.0, 100.0) / 100.0);
        Self {
            icon: if level.is_some() {
                icons::SLIDERS
            } else {
                icons::INFO
            },
            text: text.to_string(),
            level,
        }
    }
}

/// Draw what the on-screen display says.
pub fn draw(ui: &mut egui::Ui, palette: &theme::Palette, osd: &Osd) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                .corner_radius(14)
                .inner_margin(egui::Margin::symmetric(18, 10)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(osd.icon).size(24.0).color(palette.text));
                ui.add_space(6.0);
                if let Some(level) = osd.level {
                    // The rail gives way to words that are longer than a
                    // number ("40 %"), down to a stub.
                    let words = ui.painter().layout_no_wrap(
                        osd.text.clone(),
                        egui::FontId::proportional(14.0),
                        palette.text,
                    );
                    let room = ui.available_width() - words.size().x - 6.0;
                    let (rail, _) = ui.allocate_exact_size(
                        egui::vec2(room.clamp(40.0, 170.0), 8.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(rail, 4.0, palette.control);
                    let mut filled = rail;
                    filled.set_width(rail.width() * level.clamp(0.0, 1.0));
                    ui.painter().rect_filled(filled, 4.0, palette.accent);
                    ui.add_space(6.0);
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&osd.text)
                            .size(14.0)
                            .color(palette.text),
                    )
                    .truncate(),
                );
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_devices_level_is_said_in_words_and_as_a_level() {
        let at = |percent, muted| Osd::sound(Device::Speaker, Some(Level { percent, muted }));
        assert_eq!(at(40, false).text, "40 %");
        assert_eq!(at(40, false).level, Some(0.4));
        assert_eq!(at(40, false).icon, icons::SPEAKER_LOW);
        assert_eq!(at(80, false).icon, icons::SPEAKER_HIGH);
        assert_eq!(at(0, false).icon, icons::SPEAKER_X);
        // Muted shows as nothing, however loud the device is set.
        assert_eq!(at(80, true).text, "Muted");
        assert_eq!(at(80, true).level, Some(0.0));
        assert_eq!(at(80, true).icon, icons::SPEAKER_X);
        // A device driven harder than full fills the rail and no more.
        assert_eq!(at(125, false).level, Some(1.0));
        let microphone = Osd::sound(
            Device::Microphone,
            Some(Level {
                percent: 50,
                muted: true,
            }),
        );
        assert_eq!(microphone.icon, icons::MICROPHONE_SLASH);
        assert_eq!(Osd::sound(Device::Speaker, None).level, None);
    }

    #[test]
    fn what_a_command_says_is_shown_with_its_level_when_that_is_a_number() {
        assert_eq!(Osd::said("Volume", Some("40")).level, Some(0.4));
        assert_eq!(Osd::said("Volume", Some("400")).level, Some(1.0));
        assert_eq!(Osd::said("Volume", Some("-3")).level, Some(0.0));
        assert_eq!(Osd::said("Grid", None).level, None);
        assert_eq!(Osd::said("Grid", Some("loud")).level, None);
        assert_eq!(Osd::said("Grid", None).text, "Grid");
    }

    #[test]
    fn it_is_drawn_with_and_without_a_level() {
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        for osd in [Osd::said("Volume", Some("40")), Osd::said("Grid", None)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(SIZE.0 as f32, SIZE.1 as f32),
                )),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| draw(ui, &palette, &osd));
            output.textures_delta.clear();
        }
    }
}
