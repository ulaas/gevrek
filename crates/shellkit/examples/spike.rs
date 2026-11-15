//! The spike of step 4.0: a panel-sized surface with a button, a text
//! field and a menu, and what its frames cost.
//!
//!     cargo run -p gevrek-shellkit --example spike            # until closed
//!     cargo run -p gevrek-shellkit --example spike -- 3       # three seconds, then the numbers
//!     cargo run -p gevrek-shellkit --example spike -- 12 still  # drawn only when needed: what a still panel costs
//!     cargo run -p gevrek-shellkit --example spike -- 3 free    # without keeping windows clear of it

#[cfg(not(any(target_os = "linux", windows)))]
fn main() {
    eprintln!("the surface host is there for Linux and Windows only");
}

#[cfg(any(target_os = "linux", windows))]
fn main() -> anyhow::Result<()> {
    use gevrek_shellkit::{host, Flow, Place};
    use gevrek_ui::{theme, Settings};
    env_logger::init();
    let seconds: Option<f64> = std::env::args().nth(1).and_then(|text| text.parse().ok());
    let still = std::env::args().any(|word| word == "still");
    let mut place = Place::panel("gevrek-spike", 40);
    place.reserve = !std::env::args().any(|word| word == "free");
    let started = std::time::Instant::now();
    let settings = Settings::default();
    let mut ready = false;
    let mut clicks = 0;
    let mut text = String::new();
    let mut choice = "Dwindle";
    let timings = host::run(place, move |ui| {
        let context = ui.ctx().clone();
        if !ready {
            theme::install_fonts(&context, false);
            ready = true;
        }
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut flow = Flow::Continue;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("gevrek").strong());
                    if ui.button(format!("Clicked {clicks}")).clicked() {
                        clicks += 1;
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text("type here")
                            .desired_width(160.0),
                    );
                    ui.menu_button(format!("Layout: {choice}"), |ui| {
                        for name in ["Dwindle", "Master and stack", "Columns", "Grid"] {
                            if ui.button(name).clicked() {
                                choice = name;
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            flow = Flow::Close;
                        }
                        ui.label(egui::RichText::new(&text).color(palette.muted));
                    });
                });
            });
        if seconds.is_some_and(|seconds| started.elapsed().as_secs_f64() >= seconds) {
            flow = Flow::Close;
        }
        match seconds {
            // Drawn again only to close when the time is up.
            Some(seconds) if still => {
                context.request_repaint_after(std::time::Duration::from_secs_f64(
                    (seconds - started.elapsed().as_secs_f64()).max(0.0),
                ))
            }
            // Keep drawing, so that there are frames to measure.
            Some(_) => context.request_repaint(),
            None => {}
        }
        flow
    })?;
    println!(
        "frames {}  mean {:.2} ms  worst {:.2} ms  first frame after {:.0} ms",
        timings.frames, timings.mean_ms, timings.worst_ms, timings.first_frame_ms
    );
    Ok(())
}
