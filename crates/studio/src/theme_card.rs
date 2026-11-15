//! The theme's card: a palette chosen by looking at it, and the accent
//! chosen among the palette's own colours.
//!
//! The two settings are names (`Gruvbox Dark`, `base0D`), and a name says
//! nothing of a colour: the palettes there are stand side by side as
//! their sixteen colours, and the accent is one of the chosen palette's
//! colours, pressed where it is. The wallpaper's own colour is the one
//! accent that is no palette's.

use crate::document::Document;
use egui::{Color32, RichText, Sense, Ui};
use gevrek_model::theme::{
    accent_index, find_palette, Base16Palette, BASE16_PALETTES, DEFAULT_ACCENT_INDEX,
    WALLPAPER_ACCENT,
};
use gevrek_ui::theme;
use gevrek_ui::widgets;
use serde_json::Value;

const PLACE: &str = "/desktop/appearance/theme";

fn colour((red, green, blue): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

/// The slot's name as the configuration has it: `base0D`.
pub fn slot_name(index: usize) -> String {
    format!("base{index:02X}")
}

/// A palette as a tile: its sixteen colours in a strip, its name under
/// them. `None`: no palette (the colours set by hand hold).
fn palette_tile(ui: &mut Ui, palette: Option<&Base16Palette>, chosen: bool) -> egui::Response {
    let p = theme::pal(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(150.0, 58.0), Sense::click());
    let painter = ui.painter();
    painter.rect(
        rect,
        10.0,
        if response.hovered() {
            p.control_hover()
        } else {
            p.control
        },
        egui::Stroke::new(
            if chosen { 2.0 } else { 1.0 },
            if chosen { p.accent } else { p.border },
        ),
        egui::StrokeKind::Inside,
    );
    let strip = egui::Rect::from_min_size(
        rect.min + egui::vec2(10.0, 10.0),
        egui::vec2(rect.width() - 20.0, 16.0),
    );
    match palette {
        Some(palette) => {
            let each = strip.width() / 16.0;
            for (index, slot) in palette.colors.iter().enumerate() {
                let piece = egui::Rect::from_min_size(
                    strip.min + egui::vec2(index as f32 * each, 0.0),
                    egui::vec2(each + 0.5, strip.height()),
                );
                let round = match index {
                    0 => egui::CornerRadius {
                        nw: 4,
                        sw: 4,
                        ne: 0,
                        se: 0,
                    },
                    15 => egui::CornerRadius {
                        nw: 0,
                        sw: 0,
                        ne: 4,
                        se: 4,
                    },
                    _ => egui::CornerRadius::ZERO,
                };
                painter.rect_filled(piece, round, colour(*slot));
            }
        }
        None => {
            painter.rect_stroke(
                strip,
                4.0,
                egui::Stroke::new(1.0, p.border_strong()),
                egui::StrokeKind::Inside,
            );
        }
    }
    painter.text(
        egui::pos2(rect.left() + 10.0, rect.bottom() - 10.0),
        egui::Align2::LEFT_BOTTOM,
        palette.map_or("None", |palette| palette.name),
        egui::FontId::proportional(12.5),
        if chosen { p.text } else { p.muted },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::RadioButton,
            ui.is_enabled(),
            chosen,
            palette.map_or("None", |palette| palette.name),
        )
    });
    response
}

/// Draw the card's inside.
pub fn show(ui: &mut Ui, document: &mut Document) {
    let palette_at = format!("{PLACE}/palette");
    let accent_at = format!("{PLACE}/accent");
    let p = theme::pal(ui.ctx());
    let named = document
        .get(&palette_at)
        .and_then(Value::as_str)
        .map(str::to_string);
    let accent = document
        .get(&accent_at)
        .and_then(Value::as_str)
        .map(str::to_string);
    let palette = named.as_deref().and_then(find_palette);

    widgets::caption(ui, "Palette");
    // The chosen one is what is seen; the others come when it is
    // pressed, and go again with a choice.
    let open_id = ui.scope_id().with("the palettes are open");
    let mut open: bool = ui.data(|data| data.get_temp(open_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if palette_tile(ui, palette, true)
            .on_hover_text(if open {
                "The palette in use"
            } else {
                "The palette in use. A press shows the others."
            })
            .clicked()
        {
            open = !open;
        }
        if widgets::text_button(
            ui,
            if open {
                gevrek_ui::icons::CARET_UP
            } else {
                gevrek_ui::icons::CARET_DOWN
            },
            if open { "Put the others away" } else { "Choose another" },
        )
        .clicked()
        {
            open = !open;
        }
    });
    if open {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            if palette_tile(ui, None, named.is_none())
                .on_hover_text("No palette: the colours set by hand, or gevrek's own.")
                .clicked()
            {
                if named.is_some() {
                    document.set(&palette_at, Value::Null);
                }
                open = false;
            }
            for known in BASE16_PALETTES {
                let chosen = palette.is_some_and(|palette| palette.name == known.name);
                if palette_tile(ui, Some(known), chosen).clicked() {
                    if !chosen {
                        document.set(&palette_at, Value::String(known.name.to_string()));
                    }
                    open = false;
                }
            }
        });
    }
    ui.data_mut(|data| data.insert_temp(open_id, open));
    // A palette the file names that is none of these.
    if let (Some(name), None) = (&named, palette) {
        ui.label(
            RichText::new(format!(
                "The file names the palette \"{name}\", which is none of these: it is taken as no \
                 palette."
            ))
            .color(p.warn),
        );
    }
    ui.add_space(8.0);

    widgets::caption(ui, "Accent");
    let from_wallpaper = accent.as_deref() == Some(WALLPAPER_ACCENT);
    let slot = accent.as_deref().and_then(accent_index);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        if let Some(palette) = palette {
            // The palette's colours: the one that is the accent is ringed
            // (its usual one faintly, while none is named).
            for (index, colours) in palette.colors.iter().enumerate() {
                let is = slot == Some(index);
                let usual = slot.is_none() && !from_wallpaper && index == DEFAULT_ACCENT_INDEX;
                let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::click());
                let painter = ui.painter();
                painter.circle_filled(rect.center(), 10.0, colour(*colours));
                if is || usual {
                    painter.circle_stroke(
                        rect.center(),
                        12.5,
                        egui::Stroke::new(2.0, if is { p.text } else { p.faint }),
                    );
                } else if response.hovered() {
                    painter.circle_stroke(
                        rect.center(),
                        12.5,
                        egui::Stroke::new(1.0, p.border_strong()),
                    );
                }
                let name = slot_name(index);
                if response
                    .on_hover_text(if usual {
                        format!("{name}, the palette's usual accent")
                    } else {
                        name.clone()
                    })
                    .clicked()
                    && !is
                {
                    document.set(&accent_at, Value::String(name));
                }
            }
        } else {
            ui.label(RichText::new("With a palette, one of its colours.").color(p.muted));
        }
    });
    ui.horizontal(|ui| {
        let chosen = if from_wallpaper {
            1
        } else if accent.is_none() {
            0
        } else {
            2
        };
        let options = [
            (0, if palette.is_some() { "The palette's usual" } else { "None" }),
            (1, "The wallpaper's colour"),
        ];
        // (A colour of the palette is chosen above: neither of these two.)
        let selected = (chosen < 2).then_some(chosen);
        if let Some(picked) = widgets::segmented(
            ui,
            ui.scope_id().with("theme accent"),
            selected.as_ref(),
            &options,
            false,
        ) {
            document.set(
                &accent_at,
                if picked == 1 {
                    Value::String(WALLPAPER_ACCENT.to_string())
                } else {
                    Value::Null
                },
            );
        }
    });
    // An accent the file names that is no slot.
    if let Some(name) = accent
        .as_deref()
        .filter(|name| *name != WALLPAPER_ACCENT && accent_index(name).is_none())
    {
        ui.label(
            RichText::new(format!(
                "The file names the accent \"{name}\", which is no colour of a palette."
            ))
            .color(p.warn),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_is_named_as_the_configuration_names_it() {
        assert_eq!(slot_name(0), "base00");
        assert_eq!(slot_name(0x0D), "base0D");
        assert_eq!(slot_name(15), "base0F");
        // Every name the card writes is one the model reads back.
        for index in 0..16 {
            assert_eq!(accent_index(&slot_name(index)), Some(index));
        }
        // And every palette's name is one the model finds.
        for palette in BASE16_PALETTES {
            assert!(find_palette(palette.name).is_some(), "{}", palette.name);
        }
    }

    #[test]
    fn the_card_draws_and_changes_nothing_by_itself() {
        let mut document = Document::new(&gevrek_model::Config::default());
        let context = egui::Context::default();
        for palette in [Value::Null, Value::String("Ocean".into()), Value::String("no such".into())] {
            document.set("/desktop/appearance/theme/palette", palette.clone());
            for _ in 0..2 {
                let mut drawn = context.run_ui(egui::RawInput::default(), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, &mut document));
                });
                drawn.textures_delta.clear();
            }
            assert_eq!(
                document.get("/desktop/appearance/theme/palette").cloned().unwrap_or(Value::Null),
                palette
            );
        }
    }
}
