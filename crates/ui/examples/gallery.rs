//! The design system in a window: `cargo run -p gevrek-ui --example gallery`.
//!
//! `GEVREK_UI_THEME=<key>` picks a palette (`studio-dark`, `studio-light`,
//! `catppuccin-mocha`, `tokyo-night`, ...), `GEVREK_UI_DENSITY=compact` the
//! compact spacing.

use gevrek_ui::gallery::Gallery;
use gevrek_ui::{theme, Density, Preset, Settings};

struct App {
    gallery: Gallery,
    settings: Settings,
    applied: bool,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        if !self.applied {
            theme::install_fonts(&context, self.settings.system_font);
            self.applied = true;
        }
        let system = context.system_theme().unwrap_or(egui::Theme::Dark);
        let palette = self
            .settings
            .resolve(system, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &self.settings);
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(900.0);
                self.gallery.show(ui);
            });
        });
    }
}

fn main() -> eframe::Result {
    let mut settings = Settings::default();
    if let Ok(key) = std::env::var("GEVREK_UI_THEME") {
        if let Some(preset) = Preset::ALL.into_iter().find(|preset| preset.key() == key) {
            settings.preset = preset;
        }
    }
    if std::env::var("GEVREK_UI_DENSITY").as_deref() == Ok("compact") {
        settings.density = Density::Compact;
    }
    eframe::run_native(
        "gevrek-ui gallery",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("gevrek-ui-gallery")
                .with_inner_size([960.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |_| {
            Ok(Box::new(App {
                gallery: Gallery::default(),
                settings,
                applied: false,
            }))
        }),
    )
}
