//! The coach: the card of the tutorial that says the step.
//!
//! It lies over the desktop and never has the keyboard: what is pressed
//! goes to the window manager and the windows, which is what a lesson is
//! about. It says one thing to do, shows the keys this person's
//! configuration has for it, draws small what will come of it, and has
//! the ways on, back, past and out. What it shows is the tutorial's
//! (`gevrek_tutorial`); how it is drawn is the same on every system, and
//! each system's half of the shell puts it on a surface.

use gevrek_tutorial::{Demo, Help, Input, Lesson, Step, StepState};
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;

/// The card's size, in logical pixels.
pub const SIZE: (u32, u32) = (640, 196);

/// What the card shows now.
pub struct View<'a> {
    pub lesson: &'a Lesson,
    /// The step that is shown, counted from 0.
    pub at: usize,
    pub step: &'a Step,
    /// The keys the person's configuration has for the step's command.
    pub keys: Vec<String>,
    pub state: StepState,
    pub help: Help,
    pub paused: bool,
    /// Nothing moves by itself (the desktop's setting, or the system's).
    pub still: bool,
}

/// The windows of a small drawing: where each is before and after, in
/// parts of the drawing (left, top, width, height), which has the
/// keyboard before and after, and what else is drawn.
struct Scene {
    before: Vec<[f32; 4]>,
    after: Vec<[f32; 4]>,
    focus: (usize, usize),
    /// A bar comes up in the middle (the launcher).
    bar: bool,
}

/// The three windows as the dwindle layout puts them.
const TILED: [[f32; 4]; 3] = [
    [0.0, 0.0, 0.5, 1.0],
    [0.5, 0.0, 0.5, 0.5],
    [0.5, 0.5, 0.5, 0.5],
];

fn scene(demo: Demo) -> Option<Scene> {
    let tiled = TILED.to_vec();
    let plain = |before: Vec<[f32; 4]>, after: Vec<[f32; 4]>, focus| Scene {
        before,
        after,
        focus,
        bar: false,
    };
    Some(match demo {
        Demo::None => return None,
        Demo::Tile => plain(
            vec![
                [0.06, 0.10, 0.50, 0.55],
                [0.30, 0.32, 0.50, 0.50],
                [0.46, 0.14, 0.46, 0.50],
            ],
            tiled,
            (2, 2),
        ),
        // (The one that goes shrinks to nothing where it was.)
        Demo::Close => plain(
            tiled,
            vec![TILED[0], [0.5, 0.0, 0.5, 1.0], [0.75, 0.75, 0.0, 0.0]],
            (2, 1),
        ),
        Demo::Open => plain(
            vec![TILED[0], [0.5, 0.0, 0.5, 1.0], [0.75, 0.75, 0.0, 0.0]],
            tiled,
            (1, 2),
        ),
        Demo::Focus => plain(tiled.clone(), tiled, (0, 1)),
        Demo::Swap => plain(tiled, vec![TILED[1], TILED[0], TILED[2]], (0, 0)),
        Demo::Resize => plain(
            tiled,
            vec![
                [0.0, 0.0, 0.66, 1.0],
                [0.66, 0.0, 0.34, 0.5],
                [0.66, 0.5, 0.34, 0.5],
            ],
            (0, 0),
        ),
        Demo::Monocle => plain(tiled, vec![[0.0, 0.0, 1.0, 1.0], TILED[1], TILED[2]], (0, 0)),
        Demo::Float => plain(
            tiled,
            vec![TILED[0], [0.5, 0.0, 0.5, 1.0], [0.30, 0.22, 0.42, 0.52]],
            (2, 2),
        ),
        // (Out at the left; the workspace that comes is empty.)
        Demo::Workspace => plain(
            tiled,
            TILED.iter().map(|at| [at[0] - 1.1, at[1], at[2], at[3]]).collect(),
            (0, 0),
        ),
        // (The one that has the keyboard stays in sight, alone.)
        Demo::Carry => plain(
            tiled,
            vec![[-1.1, 0.0, 0.5, 1.0], [-0.6, 0.0, 0.5, 0.5], [0.0, 0.0, 1.0, 1.0]],
            (2, 2),
        ),
        Demo::Overview => plain(
            tiled,
            vec![
                [0.07, 0.30, 0.26, 0.40],
                [0.37, 0.30, 0.26, 0.40],
                [0.67, 0.30, 0.26, 0.40],
            ],
            (0, 0),
        ),
        Demo::Launcher => Scene {
            before: tiled.clone(),
            after: tiled,
            focus: (0, 0),
            bar: true,
        },
    })
}

/// How far a drawing is along at this moment: it rests, goes, rests at
/// its end and begins again, every three seconds. Still: at its end.
pub fn along(seconds: f64, still: bool) -> f32 {
    if still {
        return 1.0;
    }
    let at = (seconds.rem_euclid(3.0)) as f32;
    let part = ((at - 0.5) / 0.9).clamp(0.0, 1.0);
    // (Gently off and gently on.)
    let eased = part * part * (3.0 - 2.0 * part);
    if at > 2.7 {
        // And back, quickly, for the next round.
        eased * (1.0 - (at - 2.7) / 0.3)
    } else {
        eased
    }
}

/// Draw what a step will do, small: a display with its windows.
fn drawing(ui: &mut egui::Ui, palette: &theme::Palette, demo: Demo, t: f32, size: egui::Vec2) {
    let (frame, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(frame);
    painter.rect_filled(frame, 8.0, theme::mix(palette.surface, palette.bg, 0.6));
    let Some(scene) = scene(demo) else {
        return;
    };
    let room = frame.shrink(5.0);
    let place = |at: [f32; 4]| {
        egui::Rect::from_min_size(
            room.min + egui::vec2(at[0] * room.width(), at[1] * room.height()),
            egui::vec2(at[2] * room.width(), at[3] * room.height()),
        )
        .shrink(2.0)
    };
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let focused = if t < 0.5 { scene.focus.0 } else { scene.focus.1 };
    // (The one with the keyboard last: it lies over the others where
    // they overlap.)
    let mut order: Vec<usize> = (0..scene.before.len()).collect();
    order.sort_by_key(|index| *index == focused);
    for index in order {
        let (a, b) = (scene.before[index], scene.after[index]);
        let at = place([lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2]), lerp(a[3], b[3])]);
        if at.width() < 3.0 || at.height() < 3.0 {
            continue;
        }
        let mine = index == focused;
        painter.rect(
            at,
            4.0,
            if mine {
                theme::mix(palette.control, palette.accent, 0.18)
            } else {
                palette.control
            },
            egui::Stroke::new(
                if mine { 1.6 } else { 1.0 },
                if mine { palette.accent } else { palette.border },
            ),
            egui::StrokeKind::Inside,
        );
        // (A line or two, so that it reads as a window.)
        let line = |y: f32, part: f32| {
            let from = at.min + egui::vec2(6.0, y);
            if at.height() > y + 6.0 && at.width() > 20.0 {
                painter.line_segment(
                    [from, from + egui::vec2((at.width() - 12.0) * part, 0.0)],
                    egui::Stroke::new(1.5, theme::mix(palette.control, palette.muted, 0.5)),
                );
            }
        };
        line(8.0, 0.5);
        line(15.0, 0.8);
    }
    if scene.bar {
        let bar = egui::Rect::from_center_size(
            room.center(),
            egui::vec2(room.width() * 0.62, room.height() * 0.30 * t.max(0.05)),
        );
        let alpha = (t * 255.0) as u8;
        painter.rect(
            bar,
            5.0,
            palette.surface.gamma_multiply(t),
            egui::Stroke::new(1.2, palette.accent.gamma_multiply(t)),
            egui::StrokeKind::Inside,
        );
        if alpha > 120 {
            painter.text(
                bar.left_center() + egui::vec2(7.0, 0.0),
                egui::Align2::LEFT_CENTER,
                icons::MAGNIFYING_GLASS,
                egui::FontId::proportional(10.0),
                palette.muted,
            );
        }
    }
    painter.rect_stroke(
        frame,
        8.0,
        egui::Stroke::new(1.0, palette.border),
        egui::StrokeKind::Inside,
    );
}

/// Draw the card. What the person asked of it in this frame.
pub fn draw(ui: &mut egui::Ui, palette: &theme::Palette, view: &View<'_>) -> Vec<Input> {
    let mut asked = Vec::new();
    let seconds = ui.input(|input| input.time);
    let moving = view.step.demo != Demo::None && !view.still && !view.paused;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(
                    1.0,
                    if view.state == StepState::Done {
                        palette.ok
                    } else {
                        palette.border_strong()
                    },
                ))
                .corner_radius(16)
                .inner_margin(14),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // Which lesson, how far along it, and the ways out.
            ui.horizontal(|ui| {
                widgets::caption(ui, view.lesson.title);
                ui.add_space(4.0);
                let count = view.lesson.steps.len();
                let (dots, _) = ui.allocate_exact_size(
                    egui::vec2(count as f32 * 11.0, 10.0),
                    egui::Sense::hover(),
                );
                for index in 0..count {
                    let centre = dots.left_center() + egui::vec2(5.0 + index as f32 * 11.0, 0.0);
                    if index < view.at {
                        ui.painter().circle_filled(centre, 3.0, palette.accent);
                    } else if index == view.at {
                        ui.painter().circle_filled(centre, 3.5, palette.text);
                    } else {
                        ui.painter().circle_stroke(
                            centre,
                            3.0,
                            egui::Stroke::new(1.0, palette.faint),
                        );
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::small_icon_button(ui, icons::X, "Leave the tutorial. It can be taken up again where it was left.")
                        .clicked()
                    {
                        asked.push(Input::Leave);
                    }
                    let (sign, says) = if view.paused {
                        (icons::PLAY, "Go on")
                    } else {
                        (icons::PAUSE, "Pause: the keys are yours, nothing is counted")
                    };
                    if widgets::small_icon_button(ui, sign, says).clicked() {
                        asked.push(Input::Pause(!view.paused));
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                // What will come of it, small.
                if view.step.demo != Demo::None {
                    drawing(
                        ui,
                        palette,
                        view.step.demo,
                        along(seconds, view.still || view.paused),
                        egui::vec2(150.0, 96.0),
                    );
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(view.step.say).size(16.5).color(palette.text),
                        )
                        .wrap(),
                    );
                    // The keys, or the button that does it.
                    let waits = view.step.goal != gevrek_tutorial::Goal::Read;
                    if view.state == StepState::Done {
                        ui.label(
                            egui::RichText::new(format!("{}  That's it.", icons::CHECK_CIRCLE))
                                .size(14.0)
                                .color(palette.ok),
                        );
                    } else if view.paused {
                        ui.label(
                            egui::RichText::new("Paused.").size(13.5).color(palette.muted),
                        );
                    } else if !view.keys.is_empty() {
                        let (row, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 26.0),
                            egui::Sense::hover(),
                        );
                        let mut x = row.left();
                        for (index, keys) in view.keys.iter().take(2).enumerate() {
                            if index > 0 {
                                ui.painter().text(
                                    egui::pos2(x + 8.0, row.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    "or",
                                    egui::FontId::proportional(12.5),
                                    palette.faint,
                                );
                                x += 30.0;
                            }
                            x = widgets::keycaps(
                                ui,
                                egui::pos2(x, row.center().y),
                                keys,
                                palette.text,
                            );
                        }
                    } else if waits && view.step.act != gevrek_tutorial::lessons::Act::None {
                        // No key is bound to it: the card does it.
                        if widgets::primary_button(ui, true, "Do it").clicked() {
                            asked.push(Input::ShowMe);
                        }
                    }
                    if !view.step.why.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(view.step.why).size(12.5).color(palette.muted),
                            )
                            .wrap(),
                        );
                    }
                });
            });
            // The foot: back and past at the left, on at the right.
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if view.at > 0 && widgets::text_button(ui, icons::ARROW_LEFT, "Back").clicked() {
                        asked.push(Input::Back);
                    }
                    let waits = view.step.goal != gevrek_tutorial::Goal::Read
                        && view.state == StepState::Waiting;
                    if waits && widgets::text_button(ui, "", "Skip this step").clicked() {
                        asked.push(Input::Skip);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !waits {
                            let last = view.at + 1 == view.lesson.steps.len();
                            if widgets::primary_button(ui, true, if last { "Finish" } else { "Next" })
                                .clicked()
                            {
                                asked.push(Input::Next);
                            }
                        } else if view.step.act != gevrek_tutorial::lessons::Act::None
                            && !view.keys.is_empty()
                        {
                            // Shown how: offered plainly once the step
                            // has waited long, there quietly before.
                            let shown = if view.help == Help::Offer {
                                widgets::secondary_button(
                                    ui,
                                    format!("{}  Show me", icons::PLAY),
                                )
                            } else {
                                widgets::text_button(ui, icons::PLAY, "Show me")
                            };
                            if shown
                                .on_hover_text("gevrek does it once, to be watched; then it is yours to do.")
                                .clicked()
                            {
                                asked.push(Input::ShowMe);
                            }
                        }
                    });
                });
            });
        });
    if moving || view.state == StepState::Done {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    asked
}

/// The card's ground, as every state of it has it.
fn ground(palette: &theme::Palette, edge: egui::Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0, edge))
        .corner_radius(16)
        .inner_margin(14)
}

/// The card while a lesson's scene is set: its name, and that it comes.
pub fn draw_setting(ui: &mut egui::Ui, palette: &theme::Palette, lesson: &Lesson) -> Vec<Input> {
    let mut asked = Vec::new();
    egui::CentralPanel::default()
        .frame(ground(palette, palette.border_strong()))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::caption(ui, "The tutorial");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::small_icon_button(ui, icons::X, "Leave the tutorial").clicked() {
                        asked.push(Input::Leave);
                    }
                });
            });
            ui.add_space(14.0);
            ui.label(egui::RichText::new(lesson.title).size(20.0).strong().color(palette.text));
            ui.label(egui::RichText::new(lesson.summary).size(13.5).color(palette.muted));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                widgets::busy(ui);
                ui.label(
                    egui::RichText::new("Practice windows are opened on a workspace of the tutorial's own…")
                        .size(12.5)
                        .color(palette.faint),
                );
            });
        });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    asked
}

/// The card between two lessons: what was done, what would follow, and
/// whether to go on. `Next` goes on (or, after the last, ends it);
/// `Leave` ends it.
pub fn draw_between(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    done: &Lesson,
    next: Option<&Lesson>,
    count: (usize, usize),
) -> Vec<Input> {
    let mut asked = Vec::new();
    egui::CentralPanel::default()
        .frame(ground(palette, palette.ok))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                widgets::caption(ui, &format!("{} of {} lessons", count.0, count.1));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::small_icon_button(ui, icons::X, "Leave the tutorial").clicked() {
                        asked.push(Input::Leave);
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("{}  {}", icons::CHECK_CIRCLE, done.title))
                    .size(18.0)
                    .color(palette.ok),
            );
            match next {
                Some(next) => {
                    ui.label(
                        egui::RichText::new(format!(
                            "Next: {} (about {} min). {}",
                            next.title, next.minutes, next.summary
                        ))
                        .size(13.5)
                        .color(palette.muted),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new(
                            "That was the last one of the tour. The practice windows go, and your own workspace comes back.",
                        )
                        .size(13.5)
                        .color(palette.muted),
                    );
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    if next.is_some() && widgets::text_button(ui, "", "Enough for now").clicked() {
                        asked.push(Input::Leave);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let word = if next.is_some() { "Go on" } else { "Done" };
                        if widgets::primary_button(ui, true, word).clicked() {
                            asked.push(Input::Next);
                        }
                    });
                });
            });
        });
    asked
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_tutorial::lessons::{lesson, lessons};

    fn view<'a>(lesson: &'a Lesson, at: usize, keys: Vec<String>) -> View<'a> {
        View {
            lesson,
            at,
            step: &lesson.steps[at],
            keys,
            state: StepState::Waiting,
            help: Help::Quiet,
            paused: false,
            still: false,
        }
    }

    #[test]
    fn a_drawing_rests_goes_and_rests_at_its_end() {
        assert_eq!(along(0.0, false), 0.0);
        assert_eq!(along(0.4, false), 0.0);
        let half = along(0.95, false);
        assert!(half > 0.3 && half < 0.7, "{half}");
        assert_eq!(along(2.0, false), 1.0);
        assert!(along(2.9, false) < 0.5);
        // The next round is the same.
        assert_eq!(along(3.4, false), 0.0);
        // Where nothing moves by itself: what comes of it, still.
        assert_eq!(along(0.0, true), 1.0);
        // Every drawing has as many windows after as before.
        for demo in [
            Demo::Tile,
            Demo::Close,
            Demo::Open,
            Demo::Focus,
            Demo::Swap,
            Demo::Workspace,
            Demo::Carry,
            Demo::Overview,
            Demo::Launcher,
            Demo::Resize,
            Demo::Monocle,
            Demo::Float,
        ] {
            let scene = scene(demo).expect("a drawing");
            assert_eq!(scene.before.len(), scene.after.len(), "{demo:?}");
            assert!(scene.focus.0 < scene.before.len() && scene.focus.1 < scene.after.len());
        }
        assert!(scene(Demo::None).is_none());
    }

    #[test]
    fn every_step_of_every_lesson_draws_and_asks_nothing_by_itself() {
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        for lesson in lessons() {
            for at in 0..lesson.steps.len() {
                for (state, paused, keys) in [
                    (StepState::Waiting, false, vec!["Win+RIGHT".to_string(), "Win+L".to_string()]),
                    (StepState::Waiting, true, Vec::new()),
                    (StepState::Done, false, Vec::new()),
                ] {
                    let mut asked = Vec::new();
                    let mut drawn = context.run_ui(egui::RawInput::default(), |ui| {
                        let mut shown = view(lesson, at, keys.clone());
                        shown.state = state;
                        shown.paused = paused;
                        shown.help = Help::Offer;
                        asked = draw(ui, &palette, &shown);
                    });
                    drawn.textures_delta.clear();
                    assert!(asked.is_empty(), "{} step {}", lesson.id, at + 1);
                }
            }
        }
    }

    /// The card drawn into a picture, to be looked at:
    ///
    ///     GEVREK_LOOK_OUT=coach.png cargo test -p gevrek-shell \
    ///         coach_is_drawn_to_be_looked_at -- --ignored
    ///
    /// `GEVREK_LOOK=focus:1` for another lesson and step (counted from
    /// 1), `GEVREK_LOOK_AT=1.2` for the moment of the drawing in seconds,
    /// `GEVREK_LOOK_STATE=done|offer|nokeys|paused`.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_coach_is_drawn_to_be_looked_at() {
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let out = std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let which = said("GEVREK_LOOK").unwrap_or_else(|| "focus:1".to_string());
        let (id, step) = which.split_once(':').unwrap_or((which.as_str(), "1"));
        let shown = lesson(id).expect("a lesson of that name");
        let at = step.parse::<usize>().unwrap_or(1).clamp(1, shown.steps.len()) - 1;
        let moment: f64 = said("GEVREK_LOOK_AT").and_then(|text| text.parse().ok()).unwrap_or(2.0);
        let state = said("GEVREK_LOOK_STATE").unwrap_or_default();
        let config = gevrek_model::examples::starting_configuration();
        let keys = match shown.steps[at].act {
            gevrek_tutorial::lessons::Act::Command(command) if state != "nokeys" => {
                gevrek_tutorial::keys_for(&config, command)
            }
            _ => Vec::new(),
        };
        let mut painter = gevrek_ui::picture::Painter::sized((SIZE.0, SIZE.1))
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut pixels = Vec::new();
        for frame in 0..3 {
            pixels = painter.frame_of(&context, moment + f64::from(frame) * 0.001, Vec::new(), |ui| {
                let mut card = view(shown, at, keys.clone());
                match state.as_str() {
                    "done" => card.state = StepState::Done,
                    "offer" => card.help = Help::Offer,
                    "paused" => card.paused = true,
                    _ => {}
                }
                draw(ui, &palette, &card);
            });
        }
        painter.save(&pixels, &out).expect("the picture is written");
    }
}
