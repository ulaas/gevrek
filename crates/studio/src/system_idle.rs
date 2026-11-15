//! The system's own idle times, on the Session page (Windows).
//!
//! `desktop.session`'s times for locking and for switching the displays
//! off are kept where gevrek is the session. On Windows the system decides,
//! by its power plan; what that says is shown beside the settings, marked
//! as the system's, so that nobody takes the fields below for what holds
//! on this machine. The plan is only read (`gevrek_system::idle`).

use gevrek_ui::theme;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long what was read is shown before it is read again: the page is
/// drawn many times a second, and the plan changes when a person changes it.
const KEPT: Duration = Duration::from_secs(5);

static READ: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

fn lines() -> Vec<String> {
    let mut read = READ.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match read.as_ref() {
        Some((at, lines)) if at.elapsed() < KEPT => lines.clone(),
        _ => {
            let lines = gevrek_system::idle::read()
                .map(|times| times.lines())
                .unwrap_or_default();
            *read = Some((Instant::now(), lines.clone()));
            lines
        }
    }
}

/// A line "Name: value" as its two halves.
fn halves(line: &str) -> (&str, &str) {
    line.split_once(": ").unwrap_or((line, ""))
}

/// The card's body.
pub fn show(ui: &mut egui::Ui) {
    let palette = theme::pal(ui.ctx());
    let lines = lines();
    if lines.is_empty() {
        ui.label(
            egui::RichText::new("Windows did not say which power plan is in use.")
                .color(palette.muted),
        );
    }
    for line in &lines {
        let (name, value) = halves(line);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).color(palette.muted));
            ui.label(value);
        });
    }
    ui.label(
        egui::RichText::new(
            "These are Windows' own, from the power plan in use (Settings, System, Power), and \
             are changed there. \"Lock after minutes\" and \"Displays off after minutes\" below \
             are kept on Linux only.",
        )
        .color(palette.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_shown_as_its_name_and_what_holds() {
        assert_eq!(
            halves("Displays off: after 10 minutes on mains, never on battery"),
            (
                "Displays off",
                "after 10 minutes on mains, never on battery"
            )
        );
        assert_eq!(halves("no colon"), ("no colon", ""));
    }
}
