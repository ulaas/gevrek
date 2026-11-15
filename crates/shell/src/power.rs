//! Ending the session, and the machine's sleep, restart and switching off.
//!
//! What the power menu offers and how it is drawn is the same on every
//! system. Who carries a choice out is not: on Linux the window manager
//! (locking, ending the session) and the login service on the system's
//! bus (`org.freedesktop.login1`), which decides whether this user may,
//! and tells every program that asked to be told first; on Windows the
//! system's own calls (`gevrek_system::session`).

use gevrek_ui::theme::{self, icons};

/// What the power menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Lock,
    LogOut,
    Suspend,
    Restart,
    ShutDown,
}

impl Choice {
    pub const ALL: [Choice; 5] = [
        Choice::Lock,
        Choice::LogOut,
        Choice::Suspend,
        Choice::Restart,
        Choice::ShutDown,
    ];

    pub fn words(self) -> &'static str {
        match self {
            Choice::Lock => "Lock",
            // Each system's own word for it.
            Choice::LogOut if cfg!(windows) => "Sign out",
            Choice::LogOut => "Log out",
            Choice::Suspend => "Sleep",
            Choice::Restart => "Restart",
            Choice::ShutDown => "Shut down",
        }
    }

    /// The key that chooses it.
    pub fn key(self) -> egui::Key {
        match self {
            Choice::Lock => egui::Key::L,
            Choice::LogOut => egui::Key::O,
            Choice::Suspend => egui::Key::S,
            Choice::Restart => egui::Key::R,
            Choice::ShutDown => egui::Key::P,
        }
    }

    /// Whether choosing it once is enough. What closes every program
    /// without asking them is chosen twice.
    pub fn at_once(self) -> bool {
        matches!(self, Choice::Lock | Choice::Suspend)
    }

    /// The command the window manager carries it out with, for the
    /// choices that are the window manager's.
    #[cfg(target_os = "linux")]
    pub fn command(self) -> Option<&'static str> {
        match self {
            Choice::Lock => Some(r#"{"command":"shell","action":"lock"}"#),
            Choice::LogOut => Some(r#"{"command":"quit"}"#),
            _ => None,
        }
    }

    /// The login service's call for it, for the choices that are the
    /// machine's.
    #[cfg(target_os = "linux")]
    fn call(self) -> Option<&'static str> {
        match self {
            Choice::Suspend => Some("Suspend"),
            Choice::Restart => Some("Reboot"),
            Choice::ShutDown => Some("PowerOff"),
            _ => None,
        }
    }

    /// What Windows is asked for it: there every choice is the system's
    /// (signing out ends the user's session, not only gevrek).
    #[cfg(any(windows, test))]
    pub fn end(self) -> gevrek_system::session::End {
        use gevrek_system::session::End;
        match self {
            Choice::Lock => End::Lock,
            Choice::LogOut => End::SignOut,
            Choice::Suspend => End::Sleep,
            Choice::Restart => End::Restart,
            Choice::ShutDown => End::ShutDown,
        }
    }
}

/// Ask the machine to sleep, restart or switch off.
#[cfg(target_os = "linux")]
pub fn ask_machine(choice: Choice) -> Result<(), String> {
    let call = choice.call().ok_or("that is not the machine's to do")?;
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| format!("the system's bus does not answer: {error}"))?;
    connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            call,
            // `true`: the login service may ask for a password where its
            // rules want one.
            &(true,),
        )
        .map(|_| ())
        .map_err(|error| format!("{}: {error}", choice.words()))
}

/// The size of the power menu's surface, in logical pixels.
pub const SIZE: (u32, u32) = (620, 150);

/// The line under the choices: each one's key and what it does.
pub fn hint() -> String {
    let mut said: Vec<String> = Choice::ALL
        .iter()
        .map(|choice| format!("{} {}", choice.key().name(), choice.words().to_lowercase()))
        .collect();
    said.push("Esc closes".to_string());
    said.join(" · ")
}

/// What a press on a choice leads to: carried out now, or kept waiting
/// for a second press on the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    CarryOut(Choice),
    Arm(Choice),
}

/// A choice was pressed while `armed` (if any) waited for its second
/// press.
pub fn step(chosen: Choice, armed: Option<Choice>) -> Step {
    if chosen.at_once() || armed == Some(chosen) {
        Step::CarryOut(chosen)
    } else {
        Step::Arm(chosen)
    }
}

/// Draw the power menu: the choices side by side, the one that waits
/// for its second press marked, and under them the keys, or what went
/// wrong with the last choice. Returns the choice that was pressed or
/// whose key was, if one was. (Escape is the surface's to see to.)
pub fn draw(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    armed: Option<Choice>,
    problem: &str,
) -> Option<Choice> {
    let context = ui.ctx().clone();
    let mut chosen = None;
    for choice in Choice::ALL {
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, choice.key())) {
            chosen = Some(choice);
        }
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                .corner_radius(14)
                .inner_margin(16),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for choice in Choice::ALL {
                    let waiting = armed == Some(choice);
                    let icon = match choice {
                        Choice::Lock => icons::LOCK,
                        Choice::LogOut => icons::SIGN_OUT,
                        Choice::Suspend => icons::MOON,
                        Choice::Restart => icons::ARROW_CLOCKWISE,
                        Choice::ShutDown => icons::POWER,
                    };
                    let words = if waiting {
                        "Again to confirm".to_string()
                    } else {
                        choice.words().to_string()
                    };
                    let label = egui::RichText::new(format!("{icon}\n{words}"))
                        .size(13.5)
                        .color(palette.text);
                    let fill = if waiting {
                        theme::mix(palette.surface, palette.error, 0.35)
                    } else {
                        palette.control
                    };
                    let button = egui::Button::new(label)
                        .fill(fill)
                        .corner_radius(10)
                        .min_size(egui::vec2(108.0, 78.0));
                    let key = choice.key().name();
                    if ui
                        .add(button)
                        .on_hover_text(format!("Key: {key}"))
                        .clicked()
                    {
                        chosen = Some(choice);
                    }
                }
            });
            ui.add_space(6.0);
            let hint = if problem.is_empty() {
                egui::RichText::new(hint()).color(palette.faint)
            } else {
                egui::RichText::new(problem).color(palette.error)
            };
            ui.label(hint.size(12.0));
        });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_ends_every_program_is_chosen_twice() {
        assert!(Choice::Lock.at_once());
        assert!(Choice::Suspend.at_once());
        for choice in [Choice::LogOut, Choice::Restart, Choice::ShutDown] {
            assert!(!choice.at_once(), "{} needs a second press", choice.words());
            assert_eq!(step(choice, None), Step::Arm(choice));
            assert_eq!(step(choice, Some(choice)), Step::CarryOut(choice));
            // Another one was waiting: this one waits now.
            assert_eq!(step(choice, Some(Choice::Lock)), Step::Arm(choice));
        }
        assert_eq!(
            step(Choice::Lock, Some(Choice::Restart)),
            Step::CarryOut(Choice::Lock)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_window_managers_choices_are_commands_it_reads() {
        for choice in Choice::ALL {
            match choice.command() {
                Some(command) => {
                    let read: gevrek_model::command::DispatchCommand =
                        serde_json::from_str(command).expect("a command");
                    assert!(
                        !matches!(read, gevrek_model::command::DispatchCommand::Unknown(_)),
                        "{command}"
                    );
                    assert!(choice.call().is_none());
                }
                None => assert!(
                    choice.call().is_some(),
                    "{} is carried out by someone",
                    choice.words()
                ),
            }
        }
    }

    #[test]
    fn every_choice_has_a_key_of_its_own_and_the_hint_names_them() {
        let mut keys: Vec<egui::Key> = Choice::ALL.iter().map(|choice| choice.key()).collect();
        keys.dedup();
        assert_eq!(keys.len(), Choice::ALL.len());
        let hint = hint();
        for choice in Choice::ALL {
            assert!(hint.contains(&choice.words().to_lowercase()), "{hint}");
        }
        assert!(hint.starts_with("L lock · O "), "{hint}");
        assert!(hint.ends_with("Esc closes"), "{hint}");
    }

    #[test]
    fn on_windows_each_choice_is_the_systems_call_for_it() {
        use gevrek_system::session::{call_for, Call};
        assert_eq!(call_for(Choice::Lock.end()), Call::Lock);
        assert_eq!(call_for(Choice::Suspend.end()), Call::Suspend);
        // Signing out needs no right; the machine's ends do.
        assert!(matches!(
            call_for(Choice::LogOut.end()),
            Call::Exit {
                flags: 0,
                privilege: false
            }
        ));
        for choice in [Choice::Restart, Choice::ShutDown] {
            assert!(matches!(
                call_for(choice.end()),
                Call::Exit {
                    privilege: true,
                    ..
                }
            ));
        }
        // No two choices do the same.
        for (index, one) in Choice::ALL.iter().enumerate() {
            for other in &Choice::ALL[index + 1..] {
                assert_ne!(call_for(one.end()), call_for(other.end()));
            }
        }
    }

    #[test]
    fn the_menu_is_drawn_and_chooses_nothing_by_itself() {
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        let frame = |events: Vec<egui::Event>, armed: Option<Choice>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(SIZE.0 as f32, SIZE.1 as f32),
                )),
                events,
                ..Default::default()
            };
            let mut chosen = None;
            let mut output = context.run_ui(input, |ui| {
                chosen = draw(ui, &palette, armed, "");
            });
            output.textures_delta.clear();
            chosen
        };
        assert_eq!(frame(Vec::new(), None), None);
        assert_eq!(frame(Vec::new(), Some(Choice::Restart)), None);
        // A choice's key chooses it; drawing carries nothing out.
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(frame(vec![key(egui::Key::R)], None), Some(Choice::Restart));
        assert_eq!(frame(vec![key(egui::Key::X)], None), None);
    }
}
