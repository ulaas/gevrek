//! Something to look at, since nobody has: a surface below the windows
//! with three see-through grounds on it, as the zones of a desktop are,
//! and the calls that give such a surface its look and its place:
//!
//! - the desktop soft behind each ground and nowhere else
//!   (`set_blur_regions`), with round corners; one ground can be set
//!   moving, to see how what lies behind it keeps up;
//! - the surface raised over every window and put back (`set_level`),
//!   with the keyboard while it is up (`set_keyboard`);
//! - the surface left out of screenshots and recordings
//!   (`exclude_from_capture`; on Linux by a layer rule of the
//!   configuration, see docs/zones/milestone-3-surfaces.md).
//!
//! What to look for is in that note's checklist.
//!
//!     cargo run -p gevrek-shellkit --example look            # until closed
//!     cargo run -p gevrek-shellkit --example look -- 20      # twenty seconds
//!
//! It is in the middle of the display the pointer is on, below the
//! windows: put them aside to see it, or press "Raise" on it.

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn main() {
    eprintln!("the surface host is there for Linux, Windows and macOS only");
}

#[cfg(any(target_os = "linux", windows, target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    use gevrek_shellkit::host::{Control, Shell, SurfaceId};
    use gevrek_shellkit::look::BlurRegion;
    use gevrek_shellkit::{Edge, Flow, Keyboard, Level, Place};
    use std::cell::Cell;
    use std::rc::Rc;

    env_logger::init();
    let seconds: Option<f64> = std::env::args().nth(1).and_then(|text| text.parse().ok());
    let mut shell: Shell<()> = Shell::new()?;
    let own: Rc<Cell<Option<SurfaceId>>> = Rc::new(Cell::new(None));
    let place = Place {
        name: "gevrek-look-example".to_string(),
        edge: Edge::Centre,
        level: Level::Below,
        width: 960,
        height: 600,
        reserve: false,
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: None,
    };

    // What the buttons switch, and what the surface was last told.
    let (mut soft, mut raised, mut hidden, mut moving) = (true, false, false, false);
    let mut told: Option<Vec<BlurRegion>> = None;
    let mut began: Option<f64> = None;
    let known = Rc::clone(&own);
    let id = shell.open(place, move |ui, control: &mut Control| {
        let Some(id) = known.get() else {
            ui.ctx().request_repaint();
            return Flow::Continue;
        };
        let now = ui.input(|input| input.time);
        let began = *began.get_or_insert(now);
        if seconds.is_some_and(|seconds| now - began > seconds) {
            return Flow::Close;
        }

        // The three grounds: round, square, and very round (which may
        // move). Each is what the desktop is to be soft behind.
        let slide = if moving {
            ((now - began) * 1.2).sin() as f32 * 90.0
        } else {
            0.0
        };
        let grounds = [
            BlurRegion::new(
                egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(400.0, 300.0)),
                12.0,
            ),
            BlurRegion::new(
                egui::Rect::from_min_size(egui::pos2(480.0, 50.0), egui::vec2(440.0, 170.0)),
                0.0,
            ),
            BlurRegion::new(
                egui::Rect::from_min_size(
                    egui::pos2(520.0 + slide, 260.0),
                    egui::vec2(300.0, 290.0),
                ),
                24.0,
            ),
        ];
        let wanted: Vec<BlurRegion> = if soft { grounds.to_vec() } else { Vec::new() };
        if told.as_ref() != Some(&wanted) {
            control.set_blur_regions(id, &wanted);
            told = Some(wanted);
        }

        // Where nothing can be shown behind a ground, it is more solid:
        // the plain see-through ground that is left.
        let available = control.blur_available();
        let ground = if soft && available {
            egui::Color32::from_rgba_unmultiplied(18, 22, 28, 110)
        } else {
            egui::Color32::from_rgba_unmultiplied(18, 22, 28, 215)
        };
        let rim = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 60);
        let painter = ui.painter().clone();
        for part in &grounds {
            painter.rect_filled(part.rect, part.radius, ground);
            painter.rect_stroke(
                part.rect,
                part.radius,
                egui::Stroke::new(1.0, rim),
                egui::StrokeKind::Inside,
            );
        }
        let white = egui::Color32::from_gray(235);
        let text = |at: egui::Pos2, words: String| {
            painter.text(
                at,
                egui::Align2::LEFT_TOP,
                words,
                egui::FontId::proportional(14.0),
                white,
            );
        };
        text(
            grounds[1].rect.min + egui::vec2(16.0, 14.0),
            format!(
                "Square corners.\nSoft behind a ground can be shown here: {}.\nLevel: {}.",
                if available { "yes" } else { "no" },
                if raised {
                    "over every window"
                } else {
                    "below the windows"
                }
            ),
        );
        text(
            grounds[2].rect.min + egui::vec2(20.0, 20.0),
            "Corners of 24 points\n(the system's largest on Windows).".to_string(),
        );

        let mut flow = Flow::Continue;
        let buttons = grounds[0].rect.shrink(18.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(buttons), |ui| {
            ui.visuals_mut().override_text_color = Some(white);
            ui.label("Corners of 12 points. Between the grounds the desktop is as it is.");
            ui.add_space(8.0);
            if ui
                .button(if soft { "Soft: on" } else { "Soft: off" })
                .clicked()
            {
                soft = !soft;
            }
            if ui
                .button(if raised {
                    "Put back below the windows"
                } else {
                    "Raise over every window"
                })
                .clicked()
            {
                raised = !raised;
                control.set_level(id, if raised { Level::Overlay } else { Level::Below });
                control.set_keyboard(
                    id,
                    if raised {
                        Keyboard::Exclusive
                    } else {
                        Keyboard::OnDemand
                    },
                );
            }
            if ui
                .button(if hidden {
                    "In screenshots again"
                } else {
                    "Leave out of screenshots"
                })
                .clicked()
            {
                hidden = !hidden;
                control.exclude_from_capture(id, hidden);
            }
            if ui
                .button(if moving {
                    "Stop the third ground"
                } else {
                    "Move the third ground"
                })
                .clicked()
            {
                moving = !moving;
            }
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                flow = Flow::Close;
            }
            // (Escape closes it too, when it has the keyboard.)
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                flow = Flow::Close;
            }
        });
        if moving {
            ui.ctx().request_repaint();
        } else if seconds.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        flow
    });
    own.set(Some(id));
    let timings = shell.run(true, |(), _| {})?;
    println!("{} frames drawn", timings.frames);
    Ok(())
}
