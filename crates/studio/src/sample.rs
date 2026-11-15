//! A sample window: something for the sandbox to arrange. It stands for
//! a program without being one (an editor, a browser, a terminal by its
//! title and a few drawn lines), so that a draft's layout, gaps and
//! borders are seen at once, with no program of the user's started in a
//! desktop that is only tried out.

use eframe::egui;
use gevrek_ui::theme;

/// The windows the sandbox opens: what each is called.
pub const NAMES: [&str; 3] = ["Editor", "Browser", "Notes"];

/// The colour a sample is told apart by, by its name: one of a few,
/// always the same for the same name.
pub fn tint(name: &str) -> egui::Color32 {
    const TINTS: [(u8, u8, u8); 5] = [
        (86, 156, 214),
        (220, 160, 80),
        (120, 190, 130),
        (200, 120, 190),
        (210, 110, 110),
    ];
    // The sandbox's own by their order; any other name by its letters.
    let which = NAMES
        .iter()
        .position(|known| *known == name)
        .unwrap_or_else(|| name.bytes().map(usize::from).sum());
    let (red, green, blue) = TINTS[which % TINTS.len()];
    egui::Color32::from_rgb(red, green, blue)
}

struct Sample {
    name: String,
    fonts: bool,
    /// What it says of itself under its name.
    says: &'static str,
}

impl eframe::App for Sample {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        if !self.fonts {
            theme::install_fonts(&context, false);
            self.fonts = true;
        }
        let colour = tint(&self.name);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(28, 29, 36))
                    .inner_margin(18),
            )
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(&self.name)
                        .size(22.0)
                        .strong()
                        .color(colour),
                );
                ui.label(
                    egui::RichText::new(self.says)
                        .color(egui::Color32::from_gray(150)),
                );
                ui.add_space(12.0);
                // A few lines, as text in a program would stand.
                let painter = ui.painter().clone();
                let area = ui.available_rect_before_wrap();
                for line in 0..((area.height() / 18.0) as usize).min(40) {
                    let wide = area.width() * (0.35 + 0.6 * (((line * 37) % 100) as f32 / 100.0));
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            area.min + egui::vec2(0.0, line as f32 * 18.0),
                            egui::vec2(wide.min(area.width()), 8.0),
                        ),
                        3.0,
                        colour.gamma_multiply(if line % 5 == 0 { 0.55 } else { 0.22 }),
                    );
                }
            });
    }
}

/// Open a sample window called `name` and stay until it is closed.
pub fn run(name: &str) -> anyhow::Result<()> {
    open(name, "gevrek-sample", "A sample window of the sandbox", None)
}

/// Open a practice window of the tutorial called `name`: a sample
/// window that says whose it is, and that the window manager knows by
/// its own name.
pub fn practice(name: &str) -> anyhow::Result<()> {
    // Held by whoever opened it: gone when they let go, or are gone.
    if std::env::var_os(gevrek_tutorial::HELD).is_some() {
        std::thread::spawn(|| {
            use std::io::Read;
            let mut held = [0u8; 64];
            let mut input = std::io::stdin();
            while matches!(input.read(&mut held), Ok(read) if read > 0) {}
            std::process::exit(0);
        });
    }
    // On the main display (its corner is where the desktop's numbers
    // begin), each a little further in than the one before: the system
    // would put it on whichever display it likes (seen: one of three
    // came on the other display), and the tutorial is on the main one.
    // (Where the system does not let a window choose, nothing comes of
    // it: there the window manager puts it where the keyboard is.)
    let step = ["Notes", "Browser", "Music", "Mail", "Files", "Photos"]
        .iter()
        .position(|known| *known == name)
        .unwrap_or(name.len() % 6);
    open(
        name,
        "gevrek-practice",
        "A practice window of the tutorial. Do with it what you like.",
        Some([140.0 + step as f32 * 44.0, 120.0 + step as f32 * 36.0]),
    )
}

fn open(
    name: &str,
    app: &str,
    says: &'static str,
    at: Option<[f32; 2]>,
) -> anyhow::Result<()> {
    let title = name.to_string();
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id(app)
        .with_title(name)
        .with_inner_size([640.0, 420.0]);
    if let Some(at) = at {
        viewport = viewport.with_position(at);
    }
    eframe::run_native(
        name,
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |_| {
            Ok(Box::new(Sample {
                name: title,
                fonts: false,
                says,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("the sample window could not be opened: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_has_its_colour_by_its_name() {
        assert_eq!(tint("Editor"), tint("Editor"));
        // The three the sandbox opens are told apart.
        let tints: std::collections::HashSet<[u8; 4]> =
            NAMES.iter().map(|name| tint(name).to_array()).collect();
        assert_eq!(tints.len(), NAMES.len());
    }
}
