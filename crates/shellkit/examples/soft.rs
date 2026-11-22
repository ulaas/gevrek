//! A surface over the windows with two grounds the desktop is seen soft
//! through, with round corners (`Control::set_blur_regions`):
//!
//!     cargo run -p gevrek-shellkit --example soft -- 5    # five seconds
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn main() {}

#[cfg(any(target_os = "linux", windows, target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    use gevrek_shellkit::look::BlurRegion;
    use gevrek_shellkit::{host, Edge, Flow, Keyboard, Level, Place};
    env_logger::init();
    let seconds: f64 = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(5.0);
    let place = Place {
        name: "gevrek-soft-example".to_string(),
        edge: Edge::Centre,
        level: Level::Above,
        width: 520,
        height: 260,
        reserve: false,
        keyboard: Keyboard::Never,
        margin: 0,
        output: None,
    };
    let began = std::time::Instant::now();
    let mut asked = false;
    // (The surface's own name, known once it is open.)
    let own = std::rc::Rc::new(std::cell::Cell::new(None));
    let mut shell: host::Shell<()> = host::Shell::new()?;
    let known = std::rc::Rc::clone(&own);
    let id = shell.open(place, move |ui, control| {
        let grounds = [
            egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(220.0, 220.0)),
            egui::Rect::from_min_size(egui::pos2(280.0, 20.0), egui::vec2(220.0, 220.0)),
        ];
        for ground in grounds {
            ui.painter()
                .rect_filled(ground, 16.0, egui::Color32::from_black_alpha(60));
        }
        if let (false, true, Some(id)) = (asked, control.blur_available(), known.get()) {
            asked = true;
            let regions: Vec<BlurRegion> = grounds
                .iter()
                .map(|ground| BlurRegion::new(*ground, 16.0))
                .collect();
            control.set_blur_regions(id, &regions);
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
        if began.elapsed().as_secs_f64() > seconds {
            Flow::Close
        } else {
            Flow::Continue
        }
    });
    own.set(Some(id));
    let timings = shell.run(true, |(), _| {})?;
    println!("frames {}", timings.frames);
    Ok(())
}
