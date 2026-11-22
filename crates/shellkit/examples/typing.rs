//! A surface with a text field that has the keyboard, to type into: dead
//! keys and input methods compose as in any program, Command-C, X and V
//! copy, cut and paste. What was typed is printed at the end.
//!
//!     cargo run -p gevrek-shellkit --example typing -- 10   # ten seconds

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn main() {
    eprintln!("the surface host is there for Linux, Windows and macOS only");
}

#[cfg(any(target_os = "linux", windows, target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    use gevrek_shellkit::{host, Edge, Flow, Keyboard, Level, Place};
    use std::cell::RefCell;
    use std::rc::Rc;

    env_logger::init();
    let seconds: f64 = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(10.0);
    let place = Place {
        name: "gevrek-typing-example".to_string(),
        edge: Edge::Centre,
        level: Level::Above,
        width: 420,
        height: 120,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        output: None,
    };
    let text = Rc::new(RefCell::new(String::new()));
    let typed = Rc::clone(&text);
    let mut shell: host::Shell<()> = host::Shell::new()?;
    shell.open(place, move |ui, _| {
        let mut flow = Flow::Continue;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Type here:");
            let field =
                ui.add(egui::TextEdit::singleline(&mut *typed.borrow_mut()).desired_width(380.0));
            if !field.has_focus() {
                field.request_focus();
            }
        });
        if ui.input(|input| input.time) > seconds {
            flow = Flow::Close;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        flow
    });
    shell.run(true, |(), _| {})?;
    println!("typed: {:?}", text.borrow());
    Ok(())
}
