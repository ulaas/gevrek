//! A panel with a menu that is a surface of its own (a popup), as the
//! shell's panel opens them: it reaches beyond the panel, a press
//! elsewhere or Escape closes it, and its corners are round because what
//! is not drawn on is seen through.
//!
//!     cargo run -p gevrek-shellkit --example menu            # until closed
//!     cargo run -p gevrek-shellkit --example menu -- 5       # five seconds

#[cfg(not(any(target_os = "linux", windows)))]
fn main() {
    eprintln!("the surface host is there for Linux and Windows only");
}

#[cfg(any(target_os = "linux", windows))]
fn main() -> anyhow::Result<()> {
    use gevrek_shellkit::host::{Control, Shell, SurfaceId};
    use gevrek_shellkit::{Edge, Flow, Keyboard, Level, Place};
    use gevrek_ui::{theme, Settings};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const LAYOUTS: [&str; 4] = ["Dwindle", "Master and stack", "Columns", "Grid"];

    fn dress(context: &egui::Context, ready: &mut bool) -> gevrek_ui::Palette {
        let settings = Settings::default();
        if !*ready {
            theme::install_fonts(context, false);
            *ready = true;
        }
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(context, &palette, &settings);
        palette
    }

    env_logger::init();
    let seconds: Option<f64> = std::env::args().nth(1).and_then(|text| text.parse().ok());
    let mut shell: Shell<()> = Shell::new()?;
    // The panel's own name, known once it is open; the menu, while open.
    let panel: Rc<Cell<Option<SurfaceId>>> = Rc::new(Cell::new(None));
    let menu: Rc<Cell<Option<SurfaceId>>> = Rc::new(Cell::new(None));
    let choice = Rc::new(RefCell::new(LAYOUTS[0]));
    let place = Place {
        name: "gevrek-menu-example".to_string(),
        edge: Edge::Top,
        level: Level::Above,
        width: 0,
        height: 40,
        reserve: false,
        keyboard: Keyboard::Never,
        margin: 0,
        output: None,
    };
    let (own, open, chosen) = (Rc::clone(&panel), Rc::clone(&menu), Rc::clone(&choice));
    let mut ready = false;
    let id = shell.open(place, move |ui, control: &mut Control| {
        let context = ui.ctx().clone();
        let palette = dress(&context, &mut ready);
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
                    let button = ui.button(format!("Layout: {}", chosen.borrow()));
                    // A menu closed from outside (a press elsewhere) is
                    // no longer the open one.
                    let shown = open.get().filter(|menu| control.is_open(*menu));
                    if button.clicked() {
                        match (shown, own.get()) {
                            // A second press puts the menu away.
                            (Some(menu), _) => {
                                control.close(menu);
                                open.set(None);
                            }
                            (None, Some(panel)) => {
                                let (shown, picked) = (Rc::clone(&open), Rc::clone(&chosen));
                                let mut ready = false;
                                let size = egui::vec2(220.0, 12.0 + 36.0 * LAYOUTS.len() as f32);
                                let menu = control.popup(
                                    panel,
                                    button.rect,
                                    size,
                                    move |ui, control: &mut Control| {
                                        let context = ui.ctx().clone();
                                        let palette = dress(&context, &mut ready);
                                        let mut flow = Flow::Continue;
                                        // The whole surface with round
                                        // corners: beyond them it is
                                        // not drawn on.
                                        ui.painter().rect_filled(
                                            ui.max_rect(),
                                            10.0,
                                            palette.chrome,
                                        );
                                        egui::CentralPanel::default()
                                            .frame(
                                                egui::Frame::new()
                                                    .inner_margin(egui::Margin::same(6)),
                                            )
                                            .show(ui, |ui| {
                                                for name in LAYOUTS {
                                                    let entry = ui.add_sized(
                                                        [ui.available_width(), 26.0],
                                                        egui::Button::new(name),
                                                    );
                                                    if entry.clicked() {
                                                        *picked.borrow_mut() = name;
                                                        shown.set(None);
                                                        // The panel shows the choice.
                                                        control.redraw_all();
                                                        flow = Flow::Close;
                                                    }
                                                }
                                            });
                                        flow
                                    },
                                );
                                open.set(Some(menu));
                            }
                            (None, None) => {}
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            flow = Flow::Close;
                        }
                    });
                });
            });
        flow
    });
    panel.set(Some(id));
    // The time is kept by another thread, which tells the shell when it
    // is up: as the window manager's news reaches the real shell.
    if let Some(seconds) = seconds {
        let sender = shell.sender();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
            sender.send(());
        });
    }
    let timings = shell.run(true, |(), control| control.quit())?;
    println!(
        "frames {}  mean {:.2} ms  worst {:.2} ms  first frame after {:.0} ms  chosen {}",
        timings.frames,
        timings.mean_ms,
        timings.worst_ms,
        timings.first_frame_ms,
        choice.borrow()
    );
    Ok(())
}
