//! The night light's card: when the displays turn warm, and how warm.
//!
//! Its four settings are a switch, two times and a number, and a form of
//! four lines says them badly: the two times are one span of the day that
//! goes through midnight, and the number is a colour. So the span is a
//! band of the day with its two ends to drag (and each end a time whose
//! hour and minute are chosen, never a text), and the temperature a track
//! in the colours it stands for, with the usual ones by name. What it
//! would look like is shown on the displays themselves for a few seconds
//! (`DispatchCommand::NightLightPreview`), without applying anything.

use crate::document::Document;
use egui::{Color32, RichText, Ui};
use gevrek_model::night_light::{factors, minutes, NEUTRAL};
use gevrek_ui::controls::{self, DayBand, Stop, StopSlider};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;
use serde_json::Value;
use std::time::{Duration, Instant};

const PLACE: &str = "/desktop/night_light";

/// The warmest the track goes (the model takes less; nobody wants it).
const WARMEST: f32 = 1500.0;

/// How long a temperature is shown on the displays to be looked at.
const SHOWN_FOR: u32 = 4;

/// The temperatures that have a name.
const STOPS: [Stop; 6] = [
    Stop {
        value: 2000.0,
        name: "Candle",
    },
    Stop {
        value: 2700.0,
        name: "Lamp",
    },
    Stop {
        value: 3400.0,
        name: "Warm",
    },
    Stop {
        value: 4000.0,
        name: "Soft",
    },
    Stop {
        value: 5000.0,
        name: "Mild",
    },
    Stop {
        value: 6500.0,
        name: "Daylight",
    },
];

/// White as a display shows it at a colour temperature.
pub fn white_at(kelvin: f32) -> Color32 {
    let (red, green, blue) = factors(kelvin.round().clamp(1000.0, NEUTRAL as f32) as u32);
    let part = |factor: f32| (factor.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(part(red), part(green), part(blue))
}

/// What the settings come to, in a sentence.
pub fn said(enabled: bool, hours: Option<(u32, u32)>, kelvin: u32) -> String {
    if !enabled {
        return "Off: the displays keep their own colours.".to_string();
    }
    let warmth = controls::stop_at(&STOPS, kelvin as f32, (WARMEST, NEUTRAL as f32))
        .map_or(format!("{kelvin} K"), |stop| {
            format!("{} ({kelvin} K)", STOPS[stop].name.to_lowercase())
        });
    match hours {
        Some((from, to)) => format!(
            "The displays turn {warmth} at {} and are themselves again at {}: {} every day.",
            controls::clock(from),
            controls::clock(to),
            controls::span_words(controls::span(from, to)),
        ),
        None => format!("The displays are {warmth} all day."),
    }
}

/// The card, and what it holds between frames.
#[derive(Default)]
pub struct Card {
    /// A temperature is shown on the displays, until then.
    shown_until: Option<Instant>,
    /// The hours that were set before "all day" was chosen: chosen back,
    /// they are there again.
    hours_before: Option<(u32, u32)>,
}

impl Card {
    /// Draw the card's inside. What it asks of the window manager (a
    /// command as JSON), if anything.
    pub fn show(&mut self, ui: &mut Ui, document: &mut Document, connected: bool) -> Option<String> {
        let palette = theme::pal(ui.ctx());
        let at = |name: &str| format!("{PLACE}/{name}");
        let text = |document: &Document, name: &str| {
            document
                .get(&at(name))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let mut enabled = document
            .get(&at("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let kelvin = document
            .get(&at("temperature"))
            .and_then(Value::as_u64)
            .unwrap_or(4000) as u32;
        let (from_text, to_text) = (text(document, "from"), text(document, "to"));
        // (Times that cannot be read are no times: the whole day, as the
        // window manager takes them.)
        let hours = minutes(&from_text).zip(minutes(&to_text));
        let mut asked = None;

        // On or off, and what it comes to.
        ui.horizontal(|ui| {
            if widgets::switch(ui, &mut enabled).changed() {
                document.set(&at("enabled"), Value::Bool(enabled));
            }
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(said(enabled, hours, kelvin)).color(if enabled {
                        palette.text
                    } else {
                        palette.muted
                    }),
                )
                .wrap(),
            );
        });
        ui.add_space(10.0);

        // When.
        widgets::caption(ui, "When");
        let choice = widgets::segmented(
            ui,
            ui.scope_id().with("night light hours"),
            Some(&hours.is_some()),
            &[(true, "From evening to morning"), (false, "All day")],
            false,
        );
        match choice {
            Some(true) if hours.is_none() => {
                let (from, to) = self.hours_before.unwrap_or((20 * 60, 7 * 60));
                document.set(&at("from"), Value::String(controls::clock(from)));
                document.set(&at("to"), Value::String(controls::clock(to)));
            }
            Some(false) if hours.is_some() => {
                self.hours_before = hours;
                document.set(&at("from"), Value::String(String::new()));
                document.set(&at("to"), Value::String(String::new()));
            }
            _ => {}
        }
        if let Some((mut from, mut to)) = hours {
            ui.add_space(6.0);
            let look = DayBand {
                fill: white_at(kelvin.max(2400) as f32),
                now: Some(minutes_now()),
                dim: !enabled,
                ..DayBand::default()
            };
            let mut changed = controls::day_band(ui, &mut from, &mut to, &look).changed();
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Warm from").color(palette.muted));
                changed |= controls::time_field(ui, &mut from).changed();
                ui.label(RichText::new("until").color(palette.muted));
                changed |= controls::time_field(ui, &mut to).changed();
                ui.label(
                    RichText::new(controls::span_words(controls::span(from, to)))
                        .color(palette.faint),
                );
            });
            if changed {
                document.set(&at("from"), Value::String(controls::clock(from)));
                document.set(&at("to"), Value::String(controls::clock(to)));
            }
        } else if !from_text.trim().is_empty() || !to_text.trim().is_empty() {
            // Something is written there that is no time.
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "The times written in the file (\"{from_text}\", \"{to_text}\") are not two \
                     times of the day: it is taken as all day."
                ))
                .color(palette.warn),
            );
        }
        ui.add_space(12.0);

        // How warm.
        widgets::caption(ui, "How warm");
        let mut warmth = (kelvin as f32).clamp(WARMEST, NEUTRAL as f32);
        let paint = |value: f32| white_at(value);
        let slider = StopSlider::new(&mut warmth, WARMEST..=NEUTRAL as f32)
            .stops(&STOPS)
            .painted(&paint)
            .suffix(" K")
            .step(50.0);
        if ui.add(slider).changed() {
            document.set(&at("temperature"), Value::from(warmth.round() as u64));
        }
        ui.add_space(8.0);

        // What it looks like: a white page as it is and as it would be,
        // and the displays themselves for a moment.
        ui.horizontal(|ui| {
            let (sample, _) = ui.allocate_exact_size(egui::vec2(190.0, 40.0), egui::Sense::hover());
            let (own, warm) = sample.split_left_right_at_fraction(0.5);
            let painter = ui.painter();
            painter.rect_filled(
                own,
                egui::CornerRadius {
                    nw: 8,
                    sw: 8,
                    ne: 0,
                    se: 0,
                },
                white_at(NEUTRAL as f32),
            );
            painter.rect_filled(
                warm,
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: 8,
                    se: 8,
                },
                white_at(warmth),
            );
            for (half, words) in [(own, "white now"), (warm, "white then")] {
                painter.text(
                    half.center(),
                    egui::Align2::CENTER_CENTER,
                    words,
                    egui::FontId::proportional(12.5),
                    Color32::from_gray(40),
                );
            }
            ui.add_space(8.0);

            let showing = self
                .shown_until
                .map(|until| until.saturating_duration_since(Instant::now()))
                .filter(|left| !left.is_zero());
            if showing.is_none() {
                self.shown_until = None;
            }
            match showing {
                Some(left) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                    ui.label(
                        RichText::new(format!(
                            "{}  On the displays now; back in {} s",
                            icons::EYE,
                            left.as_secs() + 1
                        ))
                        .color(palette.muted),
                    );
                }
                None => {
                    let button = egui::Button::new(format!(
                        "{}  Show on the displays for {SHOWN_FOR} s",
                        icons::EYE
                    ));
                    let pressed = ui
                        .add_enabled(connected, button)
                        .on_hover_text(
                            "The displays turn this warm for a moment and then are as they \
                             were. Nothing is applied.",
                        )
                        .on_disabled_hover_text("When a window manager is connected");
                    if pressed.clicked() {
                        self.shown_until =
                            Some(Instant::now() + Duration::from_secs(u64::from(SHOWN_FOR)));
                        asked = Some(
                            serde_json::json!({
                                "command": "night_light_preview",
                                "temperature": warmth.round() as u32,
                                "seconds": SHOWN_FOR,
                            })
                            .to_string(),
                        );
                    }
                }
            }
        });

        // Back to what it is without anybody's doing.
        if document.differs_from_default(PLACE) {
            ui.add_space(6.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("{}  As it comes", icons::ARROW_COUNTER_CLOCKWISE))
                            .size(12.5)
                            .color(palette.muted),
                    )
                    .frame(false),
                )
                .on_hover_text("Off, soft, from 20:00 to 07:00")
                .clicked()
            {
                document.reset(PLACE);
            }
        }
        asked
    }
}

/// Minutes since midnight, by the machine's clock.
fn minutes_now() -> u32 {
    let now = jiff::Zoned::now();
    now.hour() as u32 * 60 + now.minute() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_are_said_in_a_sentence() {
        assert!(said(false, Some((1200, 420)), 4000).starts_with("Off"));
        let night = said(true, Some((20 * 60, 7 * 60)), 4000);
        assert!(night.contains("soft (4000 K)"), "{night}");
        assert!(night.contains("20:00") && night.contains("07:00") && night.contains("11 h"));
        let day = said(true, None, 3150);
        assert_eq!(day, "The displays are 3150 K all day.");
    }

    #[test]
    fn white_is_warmer_the_lower_the_temperature() {
        assert_eq!(white_at(6500.0), Color32::from_rgb(255, 255, 255));
        let candle = white_at(2000.0);
        let soft = white_at(4000.0);
        assert!(candle.b() < soft.b() && soft.b() < 255);
        assert!(candle.r() >= soft.g());
        // The set points are in order, within the track, and the model's
        // usual temperature is one of them.
        assert!(STOPS.windows(2).all(|pair| pair[0].value < pair[1].value));
        assert!(STOPS.iter().all(|stop| stop.value >= WARMEST && stop.value <= NEUTRAL as f32));
        let usual = gevrek_model::night_light::NightLightConfig::default().temperature;
        assert!(STOPS.iter().any(|stop| stop.value == usual as f32));
    }

    #[test]
    fn the_card_changes_the_document_as_it_is_pressed() {
        let mut document = Document::new(&gevrek_model::Config::default());
        let context = egui::Context::default();
        let mut card = Card::default();
        let draw = |card: &mut Card, document: &mut Document| {
            let mut drawn = context.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    card.show(ui, document, false);
                });
            });
            drawn.textures_delta.clear();
        };
        draw(&mut card, &mut document);
        draw(&mut card, &mut document);
        // Drawn alone, nothing is changed.
        assert!(!document.differs_from_default(PLACE));
        // Times that are no times are said, and nothing panics.
        document.set("/desktop/night_light/from", Value::String("evening".into()));
        draw(&mut card, &mut document);
        assert_eq!(
            document.get("/desktop/night_light/from").and_then(Value::as_str),
            Some("evening")
        );
    }
}
