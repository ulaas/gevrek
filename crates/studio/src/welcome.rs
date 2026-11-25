//! The welcome: the window a person sees the first time gevrek runs.
//!
//! Three pages. Hello: what gevrek is, in two sentences, over a small
//! drawing of windows that fall into their places. Setup: the layout to
//! start with, the look, and where the panel is, each shown as it is
//! chosen. Start: the tour, on one's own, or later.
//!
//! What is chosen is shown here, in drawings, and given to the window
//! manager once, when the person says so: a configuration handed to the
//! window manager is written to its file, and the first file of a person
//! is theirs to begin, not something a look at the choices leaves behind.
//! Whoever says "later" has changed nothing.

use eframe::egui;
use gevrek_link::{Event, Link, Request};
use gevrek_model::examples::BuiltInExample;
use gevrek_model::layout::LayoutType;
use gevrek_model::settings::PanelEdge;
use gevrek_model::Config;
use gevrek_ui::theme::{self, icons};
use gevrek_ui::widgets;

/// The window's size.
pub const SIZE: [f32; 2] = [900.0, 640.0];

/// The layouts offered to start with: what most people want first, and
/// the one that leaves everything as it was.
const LAYOUTS: [(LayoutType, &str, &str); 5] = [
    (
        LayoutType::Dwindle,
        "Dwindle",
        "Each new window takes half of the room that is left.",
    ),
    (
        LayoutType::MasterStack,
        "Main and stack",
        "One large window, the others beside it under each other.",
    ),
    (
        LayoutType::Columns,
        "Columns",
        "Side by side, each as high as the display.",
    ),
    (
        LayoutType::Scrolling,
        "Scrolling",
        "Columns on a strip that is as long as it has to be.",
    ),
    (
        LayoutType::Floating,
        "Floating",
        "Nothing is arranged: windows lie where you put them.",
    ),
];

/// How many of the built-in examples are offered as looks.
const LOOKS: usize = 4;

/// For checks: the answer the welcome gives itself (`tour`, `own`,
/// `later`), where no person is there to press.
pub const ANSWER: &str = "GEVREK_WELCOME_ANSWER";

/// How it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// What was chosen is kept, and the tour begins.
    Tour,
    /// What was chosen is kept; no tour.
    Own,
    /// Nothing is changed.
    Later,
}

/// The welcome's pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Hello,
    Setup,
    Start,
}

/// What is shown and chosen.
pub struct Welcome {
    pub page: Page,
    looks: Vec<BuiltInExample>,
    pub look: usize,
    pub layout: LayoutType,
    /// Where the panel is; none: no panel.
    pub edge: Option<PanelEdge>,
    /// The key gevrek's bindings are held with.
    pub main_key: gevrek_model::settings::MainKey,
    /// Something was chosen that is not what gevrek starts with anyway.
    /// Which of the four was chosen here: the layout, the look, the
    /// panel, the main key.
    touched: [bool; 4],
}

impl Default for Welcome {
    fn default() -> Self {
        let looks: Vec<BuiltInExample> = gevrek_model::examples::built_in_examples()
            .into_iter()
            .take(LOOKS)
            .collect();
        let start = &gevrek_model::examples::starting_configuration().desktop;
        Self {
            page: Page::Hello,
            look: 0,
            layout: start
                .workspaces
                .first()
                .map_or(LayoutType::Dwindle, |workspace| workspace.layout),
            edge: start.shell.panel.enabled.then_some(start.shell.panel.edge),
            main_key: start.main_key(),
            touched: [false; 4],
            looks,
        }
    }
}

/// How far a drawing that goes there and rests is along, every four
/// seconds; still: at its end.
fn along(seconds: f64, still: bool) -> f32 {
    if still {
        return 1.0;
    }
    let at = seconds.rem_euclid(4.0) as f32;
    let part = ((at - 0.6) / 1.1).clamp(0.0, 1.0);
    let eased = part * part * (3.0 - 2.0 * part);
    if at > 3.6 {
        eased * (1.0 - (at - 3.6) / 0.4)
    } else {
        eased
    }
}

/// The windows of a layout on a small display, as gevrek's own engine
/// places them: parts of the display (left, top, width, height).
pub fn placed(layout: LayoutType, windows: usize) -> Vec<[f32; 4]> {
    let profile = gevrek_model::settings::WorkspaceProfile {
        layout,
        gaps_in: 6,
        gaps_out: 8,
        ..Default::default()
    };
    let area = (640, 400);
    crate::preview::layout_rects(
        &profile,
        &gevrek_model::settings::AppearanceProfile::default(),
        windows,
        area,
    )
    .into_iter()
    .map(|rect| {
        [
            rect.x as f32 / area.0 as f32,
            rect.y as f32 / area.1 as f32,
            rect.width as f32 / area.0 as f32,
            rect.height as f32 / area.1 as f32,
        ]
    })
    .collect()
}

/// Where three windows lie before gevrek arranges them.
const LOOSE: [[f32; 4]; 4] = [
    [0.06, 0.10, 0.46, 0.52],
    [0.30, 0.34, 0.46, 0.50],
    [0.48, 0.12, 0.44, 0.48],
    [0.16, 0.44, 0.40, 0.46],
];

/// Draw a small display with windows: each at `from`, going to `to` as
/// far as `t` says, in these colours.
#[allow(clippy::too_many_arguments)]
fn display(
    painter: &egui::Painter,
    frame: egui::Rect,
    from: &[[f32; 4]],
    to: &[[f32; 4]],
    t: f32,
    ground: egui::Color32,
    window: egui::Color32,
    border: egui::Color32,
    focused: egui::Color32,
) {
    let painter = painter.with_clip_rect(frame);
    painter.rect_filled(frame, 10.0, ground);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    for (index, target) in to.iter().enumerate() {
        let start = from.get(index).copied().unwrap_or(*target);
        let at = egui::Rect::from_min_size(
            frame.min
                + egui::vec2(
                    lerp(start[0], target[0]) * frame.width(),
                    lerp(start[1], target[1]) * frame.height(),
                ),
            egui::vec2(
                lerp(start[2], target[2]) * frame.width(),
                lerp(start[3], target[3]) * frame.height(),
            ),
        );
        let first = index == 0;
        painter.rect(
            at,
            (frame.width() / 60.0).clamp(2.0, 8.0),
            window,
            egui::Stroke::new(
                if first { 2.0 } else { 1.0 },
                if first { focused } else { border },
            ),
            egui::StrokeKind::Inside,
        );
        if at.width() > 40.0 && at.height() > 30.0 {
            for (line, part) in [(0.0, 0.45), (1.0, 0.75), (2.0, 0.6)] {
                let y = at.top() + 10.0 + line * 7.0;
                if y + 6.0 < at.bottom() {
                    painter.line_segment(
                        [
                            egui::pos2(at.left() + 8.0, y),
                            egui::pos2(at.left() + 8.0 + (at.width() - 16.0) * part, y),
                        ],
                        egui::Stroke::new(1.5, theme::mix(window, focused, 0.25)),
                    );
                }
            }
        }
    }
}

impl Welcome {
    /// The configuration that is chosen: the look's, with the layout on
    /// every workspace and the panel where it is wanted.
    pub fn chosen(&self) -> Config {
        let mut config = self
            .looks
            .get(self.look)
            .map(|look| look.config.clone())
            .unwrap_or_else(gevrek_model::examples::starting_configuration);
        for workspace in &mut config.desktop.workspaces {
            workspace.layout = self.layout;
        }
        let panel = &mut config.desktop.shell.panel;
        match self.edge {
            Some(edge) => {
                panel.enabled = true;
                panel.edge = edge;
            }
            None => panel.enabled = false,
        }
        config.desktop.set_main_key(self.main_key);
        config
    }

    /// What lies on keys the system or most programs use themselves
    /// when the bindings are held with the main key that is chosen: said
    /// under the choice, so that it is made knowing it.
    pub fn taken_keys(&self) -> Vec<(String, String)> {
        self.chosen().desktop.set_main_key(self.main_key)
    }

    /// A configuration that runs, with what was chosen here laid over
    /// it and nothing else of it changed: for a person who has a
    /// configuration of their own and opens the welcome again.
    pub fn laid_over(&self, running: &Config) -> Config {
        let mut config = running.clone();
        if self.touched[0] {
            for workspace in &mut config.desktop.workspaces {
                workspace.layout = self.layout;
            }
        }
        if let (true, Some(look)) = (self.touched[1], self.looks.get(self.look)) {
            config.desktop.appearance = look.config.desktop.appearance.clone();
            config.desktop.shell.panel.background = look.config.desktop.shell.panel.background;
        }
        if self.touched[2] {
            let panel = &mut config.desktop.shell.panel;
            match self.edge {
                Some(edge) => {
                    panel.enabled = true;
                    panel.edge = edge;
                }
                None => panel.enabled = false,
            }
        }
        if self.touched[3] {
            config.desktop.set_main_key(self.main_key);
        }
        config
    }

    /// Whether anything was chosen that gevrek does not start with
    /// anyway (only then is a configuration handed over).
    pub fn has_chosen(&self) -> bool {
        self.touched.contains(&true)
    }

    fn hello(&mut self, ui: &mut egui::Ui, palette: &theme::Palette, still: bool) {
        let seconds = ui.input(|input| input.time);
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            gevrek_ui::brand::show_mark(ui, 56.0, palette.bg);
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Welcome to gevrek").size(30.0).strong().color(palette.text));
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(
                    "gevrek arranges your windows for you: each gets its share of the display, and none lies over another.\n\
                     You go between them, move them and gather them on workspaces with a few keys.",
                )
                .size(14.5)
                .color(palette.muted),
            );
            ui.add_space(14.0);
            let (frame, _) = ui.allocate_exact_size(egui::vec2(470.0, 280.0), egui::Sense::hover());
            display(
                ui.painter(),
                frame,
                &LOOSE,
                &placed(self.layout, 4),
                along(seconds, still),
                theme::mix(palette.bg, palette.surface, 0.5),
                palette.control,
                palette.border,
                palette.accent,
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "Nothing of yours has been moved: for now your windows float, as they always did.",
                )
                .size(12.5)
                .color(palette.faint),
            );
        });
        if !still {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    fn setup(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) {
        ui.label(egui::RichText::new("Make it yours").size(22.0).strong().color(palette.text));
        ui.label(
            egui::RichText::new(
                "A few choices to begin with. Each can be changed at any time in gevrek studio, and there is much more there.",
            )
            .size(13.0)
            .color(palette.muted),
        );
        ui.add_space(10.0);

        // The layout: each as the engine places four windows.
        widgets::caption(ui, "How windows are arranged");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (layout, name, says) in LAYOUTS {
                let chosen = self.layout == layout;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(160.0, 128.0), egui::Sense::click());
                ui.painter().rect(
                    rect,
                    10.0,
                    if response.hovered() { palette.control_hover() } else { palette.control },
                    egui::Stroke::new(
                        if chosen { 2.0 } else { 1.0 },
                        if chosen { palette.accent } else { palette.border },
                    ),
                    egui::StrokeKind::Inside,
                );
                let small = egui::Rect::from_min_size(
                    rect.min + egui::vec2(10.0, 10.0),
                    egui::vec2(rect.width() - 20.0, 82.0),
                );
                let rects = placed(layout, 4);
                display(
                    ui.painter(),
                    small,
                    &rects,
                    &rects,
                    1.0,
                    theme::mix(palette.bg, palette.surface, 0.4),
                    theme::mix(palette.control, palette.text, 0.06),
                    palette.border,
                    if chosen { palette.accent } else { palette.muted },
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 10.0, rect.bottom() - 12.0),
                    egui::Align2::LEFT_BOTTOM,
                    name,
                    egui::FontId::proportional(13.5),
                    if chosen { palette.text } else { palette.muted },
                );
                if response.on_hover_text(says).clicked() && !chosen {
                    self.layout = layout;
                    self.touched[0] = true;
                }
            }
        });
        if let Some((_, _, says)) = LAYOUTS.iter().find(|(layout, ..)| *layout == self.layout) {
            ui.label(egui::RichText::new(*says).size(12.5).color(palette.faint));
        }
        ui.add_space(10.0);

        // The look: each built-in example by its own colours.
        widgets::caption(ui, "The look");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
            let colour = |(red, green, blue): (u8, u8, u8)| egui::Color32::from_rgb(red, green, blue);
            for index in 0..self.looks.len() {
                let chosen = self.look == index;
                let look = &self.looks[index];
                let appearance = &look.config.desktop.appearance;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(204.0, 100.0), egui::Sense::click());
                let ground = colour(look.config.desktop.shell.panel.background);
                ui.painter().rect(
                    rect,
                    10.0,
                    ground,
                    egui::Stroke::new(
                        if chosen { 2.0 } else { 1.0 },
                        if chosen { palette.accent } else { palette.border },
                    ),
                    egui::StrokeKind::Inside,
                );
                let rects = placed(LayoutType::Dwindle, 3);
                let small = egui::Rect::from_min_size(
                    rect.min + egui::vec2(8.0, 8.0),
                    egui::vec2(rect.width() - 16.0, 62.0),
                );
                display(
                    ui.painter(),
                    small,
                    &rects,
                    &rects,
                    1.0,
                    ground,
                    theme::mix(ground, egui::Color32::WHITE, 0.07),
                    colour(appearance.inactive_border_color),
                    colour(appearance.active_border_color),
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 10.0, rect.bottom() - 9.0),
                    egui::Align2::LEFT_BOTTOM,
                    look.name,
                    egui::FontId::proportional(13.0),
                    if theme::luminance(ground) > 0.5 {
                        egui::Color32::from_rgb(20, 20, 24)
                    } else {
                        egui::Color32::from_rgb(236, 236, 240)
                    },
                );
                if response.on_hover_text(look.description).clicked() && !chosen {
                    self.look = index;
                    self.touched[1] = true;
                }
            }
        });
        ui.add_space(10.0);

        // The panel.
        widgets::caption(ui, "The panel");
        let options = [
            (Some(PanelEdge::Top), "At the top"),
            (Some(PanelEdge::Bottom), "At the bottom"),
            (None, "None"),
        ];
        ui.horizontal(|ui| {
            if let Some(edge) = widgets::segmented(
                ui,
                ui.scope_id().with("the panel's edge"),
                Some(&self.edge),
                &options,
                false,
            ) {
                if edge != self.edge {
                    self.edge = edge;
                    self.touched[2] = true;
                }
            }
            ui.label(
                egui::RichText::new(match self.edge {
                    Some(_) => "It shows the workspaces, the open windows, the clock and how the machine stands.",
                    None => "No panel: the launcher and the keys do everything it would.",
                })
                .size(12.5)
                .color(palette.faint),
            );
        });
        ui.add_space(10.0);

        // The main key: every binding is held with it.
        widgets::caption(ui, "The key gevrek's shortcuts are held with");
        use gevrek_model::settings::MainKey;
        let keys = [(MainKey::Super, "Super (the Windows key)"), (MainKey::Alt, "Alt")];
        ui.horizontal(|ui| {
            if let Some(key) = widgets::segmented(
                ui,
                ui.scope_id().with("the main key"),
                Some(&self.main_key),
                &keys,
                false,
            ) {
                if key != self.main_key {
                    self.main_key = key;
                    self.touched[3] = true;
                }
            }
            let taken = self.taken_keys();
            let says = match (self.main_key, taken.is_empty()) {
                (MainKey::Super, true) => {
                    "Programs do not use it for themselves, so nothing of theirs is taken.".to_string()
                }
                (MainKey::Alt, true) => {
                    "Programs use Alt for their menus: a shortcut of gevrek's comes first.".to_string()
                }
                (_, false) => format!(
                    "These would take keys you may know otherwise: {}.",
                    taken
                        .iter()
                        .map(|(name, keys)| format!("{keys} ({name})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            ui.label(egui::RichText::new(says).size(12.5).color(palette.faint));
        });
    }

    fn start(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) -> Option<Outcome> {
        let mut outcome = None;
        ui.vertical_centered(|ui| {
            ui.add_space(78.0);
            ui.label(egui::RichText::new("Ready").size(26.0).strong().color(palette.text));
            ui.add_space(4.0);
            let look = self.looks.get(self.look).map_or("Aurora", |look| look.name);
            let layout = LAYOUTS
                .iter()
                .find(|(layout, ..)| *layout == self.layout)
                .map_or("Dwindle", |(_, name, _)| *name);
            ui.label(
                egui::RichText::new(format!(
                    "{layout}, the look {look}, {}.",
                    match self.edge {
                        Some(PanelEdge::Bottom) => "the panel at the bottom",
                        Some(_) => "the panel at the top",
                        None => "no panel",
                    }
                ))
                .size(14.0)
                .color(palette.muted),
            );
            ui.add_space(26.0);
            let card = |ui: &mut egui::Ui, sign: &str, title: &str, says: &str, main: bool| -> bool {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(560.0, 74.0), egui::Sense::click());
                ui.painter().rect(
                    rect,
                    12.0,
                    if main {
                        theme::mix(palette.control, palette.accent, if response.hovered() { 0.30 } else { 0.20 })
                    } else if response.hovered() {
                        palette.control_hover()
                    } else {
                        palette.control
                    },
                    egui::Stroke::new(
                        if main { 1.5 } else { 1.0 },
                        if main { palette.accent } else { palette.border },
                    ),
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    rect.left_center() + egui::vec2(28.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    sign,
                    egui::FontId::proportional(24.0),
                    if main { palette.accent_fg() } else { palette.muted },
                );
                ui.painter().text(
                    rect.left_center() + egui::vec2(58.0, -10.0),
                    egui::Align2::LEFT_CENTER,
                    title,
                    egui::FontId::proportional(16.5),
                    palette.text,
                );
                ui.painter().text(
                    rect.left_center() + egui::vec2(58.0, 12.0),
                    egui::Align2::LEFT_CENTER,
                    says,
                    egui::FontId::proportional(12.5),
                    palette.muted,
                );
                response.clicked()
            };
            if card(
                ui,
                icons::COMPASS,
                "Take the tour",
                "About a quarter of an hour, on practice windows of its own. It can be left at any step.",
                true,
            ) {
                outcome = Some(Outcome::Tour);
            }
            ui.add_space(8.0);
            if card(
                ui,
                icons::ROCKET_LAUNCH,
                "Start on my own",
                "The list of every key is one key away, and the tour stays in the launcher.",
                false,
            ) {
                outcome = Some(Outcome::Own);
            }
        });
        outcome
    }

    /// Draw the welcome. How it ended, once it has.
    pub fn draw(&mut self, ui: &mut egui::Ui, palette: &theme::Palette, still: bool) -> Option<Outcome> {
        let mut outcome = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg).inner_margin(26))
            .show(ui, |ui| {
                // The foot first, so that it keeps its place whatever a
                // page holds: later at the left, the dots, on at the right.
                egui::Panel::bottom("the welcome's foot")
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if widgets::text_button(ui, "", "Later")
                                .on_hover_text("Nothing is changed. The welcome is in the launcher, after a >.")
                                .clicked()
                            {
                                outcome = Some(Outcome::Later);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                match self.page {
                                    Page::Hello => {
                                        if widgets::primary_button(ui, true, "Set it up").clicked() {
                                            self.page = Page::Setup;
                                        }
                                        if widgets::secondary_button(ui, "Skip the setup").clicked() {
                                            self.page = Page::Start;
                                        }
                                    }
                                    Page::Setup => {
                                        if widgets::primary_button(ui, true, "Next").clicked() {
                                            self.page = Page::Start;
                                        }
                                        if widgets::secondary_button(ui, "Back").clicked() {
                                            self.page = Page::Hello;
                                        }
                                    }
                                    Page::Start => {
                                        if widgets::secondary_button(ui, "Back").clicked() {
                                            self.page = Page::Setup;
                                        }
                                    }
                                }
                                // Where it is among its pages.
                                let (dots, _) =
                                    ui.allocate_exact_size(egui::vec2(46.0, 10.0), egui::Sense::hover());
                                for (index, page) in [Page::Hello, Page::Setup, Page::Start].iter().enumerate() {
                                    let centre = dots.left_center() + egui::vec2(8.0 + index as f32 * 14.0, 0.0);
                                    if *page == self.page {
                                        ui.painter().circle_filled(centre, 4.0, palette.accent);
                                    } else {
                                        ui.painter().circle_stroke(
                                            centre,
                                            3.5,
                                            egui::Stroke::new(1.0, palette.faint),
                                        );
                                    }
                                }
                            });
                        });
                    });
                match self.page {
                    Page::Hello => self.hello(ui, palette, still),
                    Page::Setup => self.setup(ui, palette),
                    Page::Start => outcome = outcome.or(self.start(ui, palette)),
                }
            });
        outcome
    }
}

/// The welcome as a window: connected to the window manager, to hand it
/// what was chosen.
struct Window {
    welcome: Welcome,
    link: Link,
    /// The configuration that runs and its revision, once it is known.
    running: Option<(Config, u64)>,
    /// This is a first launch: the desktop floats, and no configuration
    /// of the person's own is there.
    first: bool,
    settings: gevrek_ui::Settings,
    fonts: bool,
    /// It ended: the window closes when what was to be said is said.
    over: Option<std::time::Instant>,
    /// When the window was opened (a check's answer comes a moment
    /// after, `ANSWER`).
    opened: std::time::Instant,
    /// The tour waits for the configuration that was handed over.
    tour: Option<Tour>,
    /// When the running configuration was last asked for.
    asked: std::time::Instant,
    /// Set where the tour is to begin after this window.
    wants_tour: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// What was chosen was handed over and refused, because another
    /// configuration ran by then: it is handed over once more, on the
    /// configuration that is asked for now.
    again: Again,
}

/// Whether what was chosen is handed over once more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Again {
    /// Nothing was refused.
    No,
    /// Refused: with the next configuration that is heard of.
    Due,
    /// Handed over a second time: not a third.
    Done,
}

impl Window {
    /// What is handed to the window manager, and the revision it is laid
    /// on: a first launch gets its first configuration, whatever was
    /// chosen; a person who has one gets only what they chose here laid
    /// over theirs. Nothing where nothing was chosen, and where no
    /// window manager was heard of.
    fn wanted(&self) -> Option<(Config, u64)> {
        match &self.running {
            Some((_, revision)) if self.first => Some((self.welcome.chosen(), *revision)),
            Some((running, revision)) if self.welcome.has_chosen() => {
                Some((self.welcome.laid_over(running), *revision))
            }
            _ => None,
        }
    }

    fn end(&mut self, outcome: Outcome) {
        // "Later" on a first launch is later: nothing is written, the
        // desktop keeps floating, and the welcome is there at the next
        // start. Every other answer is an answer.
        // Nor is an answer one that could not be kept: with no window
        // manager heard of, nothing of what was chosen can be handed
        // over, and the welcome is there again at the next start. (It
        // was marked as answered, and what was chosen was lost for
        // good.)
        let kept = match outcome {
            Outcome::Later => !self.first,
            _ => self.running.is_some(),
        };
        if kept {
            let mut progress = gevrek_tutorial::Progress::load(&gevrek_tutorial::progress::file());
            progress.welcomed = true;
            if progress.version.is_empty() {
                progress.seen(env!("CARGO_PKG_VERSION"));
            }
            progress.save(&gevrek_tutorial::progress::file());
        }
        if outcome != Outcome::Later {
            // A first launch gets its first configuration, whatever was
            // chosen (with it the desktop stops floating, unless floating
            // was chosen); a person who has one gets only what they
            // chose here laid over theirs.
            if self.running.is_none() {
                log::warn!("what was chosen is not kept: no window manager answers");
            }
            let wanted = self.wanted();
            if let Some((config, base_revision)) = wanted {
                log::debug!(
                    "the welcome sends {} on revision {base_revision}",
                    if self.first { "a first configuration" } else { "what was chosen" }
                );
                self.link.send(Request::Apply {
                    config: Box::new(config),
                    base_revision,
                    client_id: "gevrek-welcome".to_string(),
                });
                if outcome == Outcome::Tour {
                    // The tour begins on a desktop that has settled: once
                    // the window manager runs what was handed over (or
                    // has not said so for a while).
                    self.tour = Some(Tour(
                        base_revision,
                        std::time::Instant::now() + std::time::Duration::from_secs(6),
                    ));
                    return;
                }
            }
        }
        if outcome == Outcome::Tour {
            self.begin_tour();
        }
        // (A moment for it to be sent before the window goes.)
        self.over = Some(std::time::Instant::now() + std::time::Duration::from_millis(600));
    }
}

impl Window {
    /// The tour is to begin: told to whoever opened the welcome by how
    /// this program ends, and begun by them after that. (Seen on the
    /// desktop: asked for from here, the window's end, a moment later,
    /// took the keyboard to another display in the middle of the
    /// lesson's beginning, and a practice window went there. A window
    /// that is closed is not gone before its program is.)
    fn begin_tour(&mut self) {
        self.wants_tour.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The tour that was asked for with a configuration to run first:
/// until the window manager runs another revision than this one, or
/// this moment at the latest.
struct Tour(u64, std::time::Instant);

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        for event in self.link.poll() {
            match &event {
                Event::Applied(revision) => log::debug!("the window manager applied revision {revision}"),
                Event::ApplyFailed(why) => {
                    log::warn!("what the welcome sent was not applied: {why}")
                }
                // Another configuration ran by the time it came (it
                // was changed elsewhere meanwhile): the one that runs
                // now is asked for, and what was chosen is laid on it.
                Event::Conflict(why) => {
                    log::warn!("what the welcome sent was not applied: {why}");
                    if self.again == Again::No {
                        self.again = Again::Due;
                        self.link.send(Request::Fetch);
                        // (The window stays for the answer.)
                        let longer = std::time::Instant::now() + std::time::Duration::from_secs(3);
                        if let Some(over) = &mut self.over {
                            *over = longer;
                        }
                    }
                }
                Event::Disconnected(why) => log::debug!("the line to the window manager is down: {why}"),
                _ => {}
            }
            if let Event::Configuration(config, revision) = event {
                if self.running.is_none() {
                    self.first = gevrek_tutorial::welcome_due(&config);
                }
                self.running = Some((*config, revision));
                if self.again == Again::Due {
                    self.again = Again::Done;
                    if let Some((config, base_revision)) = self.wanted() {
                        log::debug!("the welcome sends what was chosen again, on revision {base_revision}");
                        self.link.send(Request::Apply {
                            config: Box::new(config),
                            base_revision,
                            client_id: "gevrek-welcome".to_string(),
                        });
                        // The tour waits for this one to run.
                        if let Some(Tour(before, _)) = &mut self.tour {
                            *before = base_revision;
                        }
                    }
                }
            }
        }
        if !self.fonts {
            theme::install_fonts_for(&context, &self.settings, gevrek_ui::fonts::Area::Studio);
            self.fonts = true;
        }
        let system = context.system_theme().unwrap_or(egui::Theme::Dark);
        let palette = self
            .settings
            .resolve(system, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &self.settings);
        if let Some(Tour(before, latest)) = self.tour {
            let now = std::time::Instant::now();
            let runs = self
                .running
                .as_ref()
                .is_some_and(|(_, revision)| *revision != before);
            // (Asked, a few times a second: nobody tells of it by
            // themselves.)
            if !runs && self.asked.elapsed() >= std::time::Duration::from_millis(400) {
                self.asked = now;
                self.link.send(Request::Fetch);
            }
            if runs || now >= latest {
                self.tour = None;
                self.begin_tour();
                self.over = Some(now);
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(palette.bg))
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Setting up your desktop…")
                                .size(18.0)
                                .color(palette.muted),
                        );
                    });
                });
            context.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if let Some(over) = self.over {
            if std::time::Instant::now() >= over {
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                context.request_repaint_after(std::time::Duration::from_millis(100));
            }
            return;
        }
        let drawn = self.welcome.draw(ui, &palette, self.settings.reduce_motion);
        // A check answers without a pointer: by this word, a moment after
        // the window manager was heard.
        let answered = std::env::var(ANSWER).ok().and_then(|answer| {
            let due = self.running.is_some()
                && self.opened.elapsed() >= std::time::Duration::from_millis(2500);
            if !due {
                context.request_repaint_after(std::time::Duration::from_millis(200));
                return None;
            }
            match answer.as_str() {
                "tour" => Some(Outcome::Tour),
                "own" => Some(Outcome::Own),
                "later" => Some(Outcome::Later),
                _ => None,
            }
        });
        if let Some(outcome) = drawn.or(answered) {
            self.end(outcome);
            context.request_repaint();
        }
    }
}

/// Open the welcome and stay until it is answered. Whether the tour was
/// asked for: whoever opened the welcome begins it, once this program
/// has ended (`gevrek_tutorial::TOUR_ASKED`).
pub fn run(address: String, appearance: &std::path::Path) -> anyhow::Result<bool> {
    let settings = gevrek_ui::Settings::load(appearance);
    let wants_tour = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wanted = std::sync::Arc::clone(&wants_tour);
    eframe::run_native(
        "Welcome to gevrek",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("gevrek-welcome")
                .with_title("Welcome to gevrek")
                .with_inner_size(SIZE)
                .with_resizable(false),
            centered: true,
            ..Default::default()
        },
        Box::new(move |creation| {
            let wake = creation.egui_ctx.clone();
            let link = Link::start(address, move || wake.request_repaint());
            link.send(Request::Fetch);
            Ok(Box::new(Window {
                welcome: Welcome::default(),
                link,
                running: None,
                first: false,
                settings,
                fonts: false,
                over: None,
                opened: std::time::Instant::now(),
                tour: None,
                asked: std::time::Instant::now(),
                wants_tour: wanted,
                again: Again::No,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("the welcome could not be opened: {error}"))?;
    Ok(wants_tour.load(std::sync::atomic::Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_chosen_is_the_looks_configuration_with_the_layout_and_the_panel() {
        let mut welcome = Welcome::default();
        // As gevrek starts: nothing chosen, nothing to hand over.
        assert!(!welcome.has_chosen());
        let start = gevrek_model::examples::starting_configuration();
        assert_eq!(
            serde_json::to_value(welcome.chosen()).unwrap(),
            serde_json::to_value(&start).unwrap(),
            "what is offered first is what gevrek starts with"
        );
        welcome.layout = LayoutType::Columns;
        welcome.edge = Some(PanelEdge::Bottom);
        welcome.look = 1;
        let chosen = welcome.chosen();
        assert!(chosen.desktop.workspaces.iter().all(|workspace| workspace.layout == LayoutType::Columns));
        assert!(chosen.desktop.shell.panel.enabled);
        assert_eq!(chosen.desktop.shell.panel.edge, PanelEdge::Bottom);
        assert_eq!(
            chosen.desktop.appearance.active_border_color,
            welcome.looks[1].config.desktop.appearance.active_border_color
        );
        // It is a configuration gevrek takes.
        let errors: Vec<_> = chosen
            .validate()
            .into_iter()
            .filter(|issue| issue.severity == gevrek_model::settings::ConfigIssueSeverity::Error)
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        welcome.edge = None;
        assert!(!welcome.chosen().desktop.shell.panel.enabled);
        // Over a configuration of the person's own, only what was chosen
        // here changes.
        let mut theirs = start.clone();
        theirs.desktop.focus.history_size += 3;
        assert_eq!(
            serde_json::to_value(welcome.laid_over(&theirs)).unwrap(),
            serde_json::to_value(&theirs).unwrap()
        );
        welcome.touched[0] = true;
        // The main key: chosen, every shortcut that was held with Super
        // is held with Alt, in what is chosen and in what is laid over a
        // configuration that runs; not chosen, a running one keeps its own.
        {
            use gevrek_model::settings::MainKey;
            let mut keys = Welcome::default();
            assert_eq!(keys.main_key, MainKey::Super);
            assert_eq!(keys.chosen().desktop.main_key(), MainKey::Super);
            let running = keys.chosen();
            keys.main_key = MainKey::Alt;
            assert_eq!(keys.laid_over(&running).desktop.main_key(), MainKey::Super);
            keys.touched[3] = true;
            assert!(keys.has_chosen());
            assert_eq!(keys.chosen().desktop.main_key(), MainKey::Alt);
            let laid = keys.laid_over(&running);
            assert_eq!(laid.desktop.main_key(), MainKey::Alt);
            assert_eq!(laid.desktop.bindings.len(), running.desktop.bindings.len());
        }
        let over = welcome.laid_over(&theirs);
        assert!(over.desktop.workspaces.iter().all(|workspace| workspace.layout == LayoutType::Columns));
        assert_eq!(over.desktop.focus.history_size, theirs.desktop.focus.history_size);
        assert_eq!(over.desktop.shell.panel, theirs.desktop.shell.panel);
        // Every layout offered is placed by the engine, inside the display.
        for (layout, name, _) in LAYOUTS {
            let rects = placed(layout, 4);
            assert!(!rects.is_empty(), "{name}");
            for rect in rects {
                assert!(rect[2] > 0.0 && rect[3] > 0.0 && rect[0] >= 0.0 && rect[1] >= 0.0, "{name}: {rect:?}");
            }
        }
    }

    #[test]
    fn every_page_draws_and_ends_only_when_it_is_answered() {
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        let mut welcome = Welcome::default();
        for page in [Page::Hello, Page::Setup, Page::Start] {
            welcome.page = page;
            for still in [false, true] {
                let mut outcome = None;
                let mut drawn = context.run_ui(egui::RawInput::default(), |ui| {
                    outcome = welcome.draw(ui, &palette, still);
                });
                drawn.textures_delta.clear();
                assert_eq!(outcome, None, "{page:?}");
            }
        }
        assert!(!welcome.has_chosen());
        assert_eq!(along(0.0, false), 0.0);
        assert_eq!(along(2.5, false), 1.0);
        assert_eq!(along(0.0, true), 1.0);
    }

    /// The welcome drawn into a picture, to be looked at:
    ///
    ///     GEVREK_LOOK_OUT=welcome.png GEVREK_LOOK=setup cargo test -p gevrek-studio \
    ///         welcome_is_drawn_to_be_looked_at -- --ignored
    ///
    /// `GEVREK_LOOK`: `hello`, `setup` or `start`; `GEVREK_LOOK_AT`: the
    /// moment of the drawing, in seconds.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_welcome_is_drawn_to_be_looked_at() {
        use gevrek_ui::picture::Painter;
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let out = std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let moment: f64 = said("GEVREK_LOOK_AT").and_then(|text| text.parse().ok()).unwrap_or(2.5);
        let mut welcome = Welcome {
            page: match said("GEVREK_LOOK").as_deref() {
                Some("setup") => Page::Setup,
                Some("start") => Page::Start,
                _ => Page::Hello,
            },
            ..Welcome::default()
        };
        let mut painter = Painter::sized((SIZE[0] as u32, SIZE[1] as u32))
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut pixels = Vec::new();
        for frame in 0..3 {
            pixels = painter.frame_of(&context, moment + f64::from(frame) * 0.001, Vec::new(), |ui| {
                welcome.draw(ui, &palette, false);
            });
        }
        painter.save(&pixels, &out).expect("the picture is written");
    }
}
