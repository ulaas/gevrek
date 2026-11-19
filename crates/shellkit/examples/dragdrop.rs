//! A small surface to try drag and drop on: it takes what is dragged
//! onto it (and says what came), and a file of its own can be dragged out
//! of it into a file manager or any other program.
//!
//! Dropped on its left half a thing is copied, on its right half moved,
//! unless the keys say otherwise (Ctrl copies, Shift moves): the pointer
//! of the program the drag comes from shows which. Nothing is done with
//! what is dropped; it is only named, on the surface and on the terminal.
//!
//!     cargo run -p gevrek-shellkit --example dragdrop            # until closed
//!     cargo run -p gevrek-shellkit --example dragdrop -- 30      # thirty seconds

#[cfg(not(any(target_os = "linux", windows)))]
fn main() {
    eprintln!("the surface host is there for Linux and Windows only");
}

#[cfg(any(target_os = "linux", windows))]
fn main() -> anyhow::Result<()> {
    use gevrek_shellkit::dnd::{
        DragAction, DragActions, DragEvent, DragImage, DragPayload, DragStart, DropAnswer,
    };
    use gevrek_shellkit::host::{Control, Shell, SurfaceId};
    use gevrek_shellkit::{Edge, Flow, Keyboard, Level, Place};
    use std::cell::Cell;
    use std::rc::Rc;

    /// The picture that follows the pointer: a rounded card, more solid
    /// towards its middle, made for the display's own pixels.
    fn card(scale: f32) -> DragImage {
        let (width, height) = ((96.0 * scale) as u32, (64.0 * scale) as u32);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let across = (x as f32 / width as f32 - 0.5).abs() * 2.0;
                let down = (y as f32 / height as f32 - 0.5).abs() * 2.0;
                let solid = (1.0 - across.max(down).powi(4)).clamp(0.0, 1.0);
                rgba.extend_from_slice(&[64, 224, 208, (solid * 220.0) as u8]);
            }
        }
        DragImage {
            width,
            height,
            rgba,
            hotspot: (width as i32 / 2, height as i32 / 2),
            scale,
        }
    }

    /// A file to drag, written anew each time.
    fn file_to_drag() -> std::io::Result<String> {
        let path = std::env::temp_dir().join("gevrek-dragdrop-example.txt");
        std::fs::write(&path, "Dragged out of a surface of gevrek's shell.\n")?;
        Ok(path.to_string_lossy().into_owned())
    }

    env_logger::init();
    let seconds: Option<f64> = std::env::args().nth(1).and_then(|text| text.parse().ok());
    let mut shell: Shell<()> = Shell::new()?;
    let own: Rc<Cell<Option<SurfaceId>>> = Rc::new(Cell::new(None));
    let place = Place {
        name: "gevrek-dragdrop-example".to_string(),
        edge: Edge::Centre,
        level: Level::Above,
        width: 420,
        height: 300,
        reserve: false,
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: None,
    };
    let me = Rc::clone(&own);
    let mut asked = false;
    let mut said: Vec<String> = Vec::new();
    let mut over: Option<egui::Pos2> = None;
    let id = shell.open(place, move |ui, control: &mut Control| {
        let Some(id) = me.get() else {
            return Flow::Continue;
        };
        if !asked {
            // Anything may be dropped, anywhere on the surface; where
            // exactly decides between a copy and a move (below).
            control.accept_drops(id, Some(DropAnswer::accept(DragActions::ALL)));
            asked = true;
        }
        let width = ui.ctx().content_rect().width();
        for event in control.drag_events(id) {
            let line = match &event {
                DragEvent::Entered { at, offered } => {
                    over = Some(*at);
                    format!(
                        "entered at {at:?}: {} kinds, files: {}, allowed: {:?}",
                        offered.kinds.len(),
                        offered.files,
                        offered.allowed
                    )
                }
                DragEvent::Moved { at } => {
                    over = Some(*at);
                    let prefer = if at.x < width / 2.0 {
                        DragAction::Copy
                    } else {
                        DragAction::Move
                    };
                    control.answer_drag(id, DropAnswer::preferring(DragActions::ALL, prefer));
                    println!("moved to {at:?}");
                    continue;
                }
                DragEvent::Left => {
                    over = None;
                    "left".to_string()
                }
                DragEvent::Dropped {
                    at,
                    payload,
                    action,
                } => {
                    over = None;
                    format!(
                        "dropped at {at:?} to {action:?}: paths {:?}, addresses {:?}, text {:?}, {} of the system's own",
                        payload.paths,
                        payload.uris,
                        payload.text,
                        payload.native.len()
                    )
                }
                DragEvent::Ended { outcome } => format!("the drag out ended: {outcome:?}"),
            };
            println!("{line}");
            said.push(line);
            if said.len() > 8 {
                said.remove(0);
            }
        }
        let mut flow = Flow::Continue;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Drag and drop");
                if ui.button("Close").clicked() {
                    flow = Flow::Close;
                }
            });
            ui.label("Drop files here: left half copies, right half moves.");
            let handle = ui.add(
                egui::Button::new("Drag this out: a text file")
                    .sense(egui::Sense::drag())
                    .min_size(egui::vec2(220.0, 36.0)),
            );
            if handle.drag_started() {
                match file_to_drag() {
                    Ok(path) => control.start_drag(
                        id,
                        DragStart {
                            payload: DragPayload::files([path]),
                            image: Some(card(ui.ctx().pixels_per_point())),
                        },
                    ),
                    Err(error) => said.push(format!("no file to drag: {error}")),
                }
            }
            ui.separator();
            if let Some(at) = over {
                ui.label(format!("over the surface at {:.0}, {:.0}", at.x, at.y));
            }
            for line in &said {
                ui.label(egui::RichText::new(line).small());
            }
        });
        if seconds.is_some_and(|seconds| ui.input(|input| input.time) > seconds) {
            flow = Flow::Close;
        }
        if seconds.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        flow
    });
    own.set(Some(id));
    shell.run(true, |(), _| {})?;
    Ok(())
}
