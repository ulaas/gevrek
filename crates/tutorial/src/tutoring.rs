//! The tutorial while it is taken, as whoever draws it carries it: the
//! scene of a lesson is set on the tutorial's own workspace, the lesson
//! runs, and between two lessons the person says whether to go on.
//!
//! This is the part that is the same on every system and needs no
//! window: it is told how the desktop stands and what was pressed, and
//! says what is to be done (a command for the window manager, practice
//! windows to open, the tutorial's end). The shell opens the card's
//! surface, starts and ends the practice windows, and hands the commands
//! on. Kept here, beside the lessons, so that every lesson can be played
//! against a real window manager without a desktop
//! (`tests/tutorial_played.rs`).

use crate::lessons::{lessons, Chapter};
use crate::{Effect, Facts, Help, Input, Lesson, Progress, Run, Step, StepState};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The card's size, in logical pixels (the shell draws it; its room is
/// kept free of tiles from here).
pub const CARD_SIZE: (u32, u32) = (640, 196);

/// How long the windows of a scene are given to settle before the
/// keyboard is put on the first of them, and after that before the
/// lesson's first step is measured.
const SETTLE: Duration = Duration::from_millis(1200);
const AIMED: Duration = Duration::from_millis(700);
/// A scene that does not come (a practice window that will not open) is
/// not waited for longer than this: the lesson begins with what is there.
const AT_MOST: Duration = Duration::from_secs(8);
/// How often the scene's layout is asked for while it is not there, and
/// how long it is waited for before the windows are opened without it.
const ASKED: Duration = Duration::from_millis(300);
const LAID_AT_MOST: Duration = Duration::from_secs(3);
/// How long the tutorial's workspace is shown with its layout before the
/// practice windows are opened on it.
const SHOWN: Duration = Duration::from_millis(900);

/// What the shell is to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Do {
    /// Hand this command (the JSON of a binding) to the window manager.
    Dispatch(String),
    /// Open so many practice windows.
    OpenPractice(usize),
    /// The tutorial is over: close the practice windows, take its
    /// workspace away, show what was shown before, close the card.
    End,
}

/// Where the tutorial is.
#[derive(Debug, Clone)]
enum Phase {
    /// The scene is set, in this order: the workspace shown, its layout
    /// asked for there, the practice windows opened (`opened`), the
    /// keyboard put on the first of them (`aimed`).
    Setting {
        since: Instant,
        began: Instant,
        opened: bool,
        aimed: bool,
    },
    Running(Box<Run>),
    /// A lesson was finished: the next of the tour, if there is one.
    Between(Option<&'static Lesson>),
}

/// What the card shows.
pub enum Shown<'a> {
    /// The scene is being set.
    Setting(&'a Lesson),
    Step {
        lesson: &'a Lesson,
        at: usize,
        step: &'a Step,
        state: StepState,
        help: Help,
        paused: bool,
    },
    /// A lesson is done; the one that would follow.
    Between {
        done: &'a Lesson,
        next: Option<&'a Lesson>,
        /// How many lessons are finished, of how many.
        count: (usize, usize),
    },
}

pub struct Tutoring {
    phase: Phase,
    lesson: &'static Lesson,
    /// The step the lesson is begun at (one that was left is taken up
    /// where it was).
    from: usize,
    progress: Progress,
    /// Where the tutorial's memory is kept.
    file: PathBuf,
    /// The workspace that was shown before the tutorial's own.
    pub before: String,
    /// The display the tutorial is on: the one the person was on when it
    /// began (empty where the desktop did not say: the main one then).
    display: String,
    facts: Facts,
    /// When the workspace beside the tutorial's was last asked for.
    beside_asked: Option<Instant>,
}

/// How thick the ring around what a step is about is, and how far
/// outside it the ring stands.
pub const RING: i32 = 4;
const RING_GAP: i32 = 3;

/// A rectangle of the desktop: its left, its top, how wide, how high.
pub type Around = (i32, i32, i32, i32);

/// What the ring is drawn around for a step that waits, as the desktop
/// stands: the window with the keyboard, all the windows of the
/// workspace together, or the panel (`panel`: where it is on the
/// tutorial's display). Nothing where the step is about nothing that has
/// a place (or about a surface of the shell's own, which the shell
/// shows where it likes), and where what it is about is not there.
pub fn ring_around(spot: crate::Spot, facts: &Facts, panel: Option<Around>) -> Option<Around> {
    use crate::Spot;
    let sized = |around: &Around| around.2 > 0 && around.3 > 0;
    match spot {
        Spot::None | Spot::Surface(_) => None,
        Spot::Focused => facts
            .windows
            .iter()
            .find(|window| window.focused)
            .map(|window| window.at)
            .filter(sized),
        Spot::Windows => {
            let all: Vec<Around> = facts
                .windows
                .iter()
                .map(|window| window.at)
                .filter(sized)
                .collect();
            let left = all.iter().map(|at| at.0).min()?;
            let top = all.iter().map(|at| at.1).min()?;
            let right = all.iter().map(|at| at.0 + at.2).max()?;
            let bottom = all.iter().map(|at| at.1 + at.3).max()?;
            Some((left, top, right - left, bottom - top))
        }
        Spot::Panel => panel.filter(sized),
    }
}

/// The four sides of the ring around a rectangle, each a rectangle of
/// its own (a surface as large as the display would be black on Windows
/// unless the graphics device draws with DirectX 12): above, below, at
/// the left, at the right. The corners belong to the two that lie.
pub fn ring_sides(around: Around) -> [Around; 4] {
    let (x, y, width, height) = (
        around.0 - RING_GAP - RING,
        around.1 - RING_GAP - RING,
        around.2 + 2 * (RING_GAP + RING),
        around.3 + 2 * (RING_GAP + RING),
    );
    [
        (x, y, width, RING),
        (x, y + height - RING, width, RING),
        (x, y + RING, RING, height - 2 * RING),
        (x + width - RING, y + RING, RING, height - 2 * RING),
    ]
}

/// How far the card stands from the display's edge, and how much room
/// is left between it and the tiles above it.
pub const CARD_MARGIN: u32 = 28;
const CARD_GAP: u32 = 12;

/// The room the card takes at the foot of the display is kept free of
/// tiles on the workspace that is shown (the tutorial's own): its tiles
/// end above the card. The room goes with the workspace when the
/// tutorial ends.
fn card_room() -> Do {
    command(serde_json::json!({
        "command": "set_workspace_work_area_offset",
        "left": 0,
        "top": 0,
        "right": 0,
        "bottom": CARD_SIZE.1 + CARD_MARGIN + CARD_GAP,
    }))
}

fn command(json: serde_json::Value) -> Do {
    Do::Dispatch(json.to_string())
}

/// The layout as the window manager is told it (`Dwindle`), from its
/// name in a lesson (`dwindle`).
fn layout_named(name: &str) -> serde_json::Value {
    name.parse::<gevrek_model::layout::LayoutType>()
        .ok()
        .and_then(|layout| serde_json::to_value(layout).ok())
        .unwrap_or_else(|| serde_json::Value::String("Dwindle".to_string()))
}

impl Tutoring {
    /// Begin the tutorial with this lesson (none: the one to take next
    /// by what is remembered), on the desktop as it stands. Nothing
    /// where there is no lesson to take.
    pub fn begin(
        wanted: Option<&str>,
        file: PathBuf,
        now_shown: &Facts,
        now: Instant,
    ) -> Option<(Self, Vec<Do>)> {
        let mut progress = Progress::load(&file);
        let (lesson, from) = match wanted.and_then(crate::lessons::lesson) {
            Some(lesson) => (lesson, 0),
            None => progress
                .next()
                .or_else(|| lessons().first().map(|first| (first, 0)))?,
        };
        progress.welcomed = true;
        progress.running = true;
        // (Someone who takes a lesson has seen this version: what is new
        // is counted from here.)
        if progress.version.is_empty() {
            progress.seen(env!("CARGO_PKG_VERSION"));
        }
        progress.save(&file);
        let mut tutoring = Self {
            phase: Phase::Setting {
                since: now,
                began: now,
                opened: false,
                aimed: false,
            },
            lesson,
            from,
            progress,
            file,
            // (Begun from the tutorial's own workspace, left behind by a
            // gevrek that stopped in the middle: there is none before.)
            before: Some(now_shown.workspace.clone())
                .filter(|name| name != crate::WORKSPACE)
                .unwrap_or_default(),
            display: now_shown.display.clone(),
            facts: now_shown.clone(),
            beside_asked: None,
        };
        let mut todo = vec![
            tutoring.display_focused(),
            tutoring.workspace_laid(),
            command(serde_json::json!({
                "command": "switch_workspace_selector",
                "selector": {"kind": "name", "value": crate::WORKSPACE},
            })),
        ];
        todo.extend(tutoring.set_scene(now));
        Some((tutoring, todo))
    }

    /// The display the tutorial is on, as a command names it: the one the
    /// person was on when it began. (It used to be the main display
    /// always: begun from another, the tutorial went away to where
    /// nobody was looking.)
    fn target(&self) -> serde_json::Value {
        if self.display.trim().is_empty() {
            serde_json::json!({"kind": "primary"})
        } else {
            serde_json::json!({"kind": "name", "value": self.display})
        }
    }

    /// The keyboard to the tutorial's display.
    fn display_focused(&self) -> Do {
        command(serde_json::json!({"command": "focus_monitor", "target": self.target()}))
    }

    /// What the ring is around now: what the step that waits is about.
    /// Nothing while a scene is set, between two lessons, and once the
    /// step is done.
    pub fn spot(&self) -> crate::Spot {
        match &self.phase {
            Phase::Running(run) if run.state() == StepState::Waiting && !run.paused() => {
                run.step().spot
            }
            _ => crate::Spot::None,
        }
    }

    /// The windows on the tutorial's workspace that are the person's own
    /// (no practice windows), as far as the desktop was last seen: they
    /// are taken along when the tutorial ends (`tidy`).
    pub fn own_windows(&self) -> Vec<u64> {
        own_windows(&self.facts)
    }

    /// The tutorial's workspace, there and with the lesson's layout: said
    /// of the workspace by its name. (Seen on the desktop, with two
    /// displays: a layout asked of "the workspace" went to the one on
    /// the display the pointer was on, and that one of the person's
    /// floated.)
    fn workspace_laid(&self) -> Do {
        command(serde_json::json!({
            "command": "ensure_named_workspace",
            "target": self.target(),
            "name": crate::WORKSPACE,
            "layout": layout_named(self.lesson.scene.layout),
        }))
    }

    /// Begin to set the scene of the lesson: its layout now, its practice
    /// windows with time (`tick`), once the tutorial's workspace is
    /// shown with that layout. (Seen: a window that came before the
    /// layout was tiled over the whole display and stayed that large.)
    fn set_scene(&mut self, now: Instant) -> Vec<Do> {
        self.phase = Phase::Setting {
            since: now,
            began: now,
            opened: false,
            aimed: false,
        };
        vec![self.workspace_laid()]
    }

    /// The workspace beside the tutorial's own, for a lesson that goes
    /// from one workspace to the next: made after it, with its layout.
    fn beside_laid(&self) -> Do {
        command(serde_json::json!({
            "command": "ensure_named_workspace",
            "target": self.target(),
            "name": crate::WORKSPACE_BESIDE,
            "layout": layout_named(self.lesson.scene.layout),
        }))
    }

    /// Whether the workspace beside the tutorial's own is there, as far
    /// as the desktop was last seen.
    pub fn beside_there(&self) -> bool {
        self.facts
            .workspaces
            .iter()
            .any(|name| name == crate::WORKSPACE_BESIDE)
    }

    /// A lesson that goes to the next workspace has one to go to that is
    /// the tutorial's own. (Played against the window manager: the next
    /// workspace after the tutorial's, the last of the display, was the
    /// person's first, and the window that was taken along was tiled
    /// among theirs.) A workspace of the moment goes when it is left
    /// empty, so it is asked for again whenever the tutorial's own is
    /// shown and it is not there.
    fn beside_due(&mut self, now: Instant) -> Option<Do> {
        if !self.lesson.scene.beside
            || self.facts.workspace != crate::WORKSPACE
            || self.beside_there()
        {
            return None;
        }
        if self
            .beside_asked
            .is_some_and(|asked| now.saturating_duration_since(asked) < ASKED)
        {
            return None;
        }
        self.beside_asked = Some(now);
        Some(self.beside_laid())
    }

    pub fn lesson(&self) -> &'static Lesson {
        self.lesson
    }

    /// Whether a lesson's step is waiting for something to be done (the
    /// card is drawn again and again meanwhile).
    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running(_))
    }

    /// How many practice windows the tutorial's workspace has.
    fn practice(&self) -> usize {
        if self.facts.workspace == crate::WORKSPACE {
            self.facts.windows.len()
        } else {
            0
        }
    }

    fn after(&mut self, effects: Vec<Effect>, now: Instant) -> Vec<Do> {
        let mut todo = Vec::new();
        for effect in effects {
            match effect {
                Effect::Dispatch(command) => todo.push(Do::Dispatch(command)),
                Effect::OpenPractice => todo.push(Do::OpenPractice(1)),
                Effect::Step => {}
                Effect::Finished => {
                    self.progress.finish(self.lesson);
                    self.progress.running = true;
                    self.progress.save(&self.file);
                    // The next of the tour, for a lesson of the tour.
                    let next = (self.lesson.chapter == Chapter::Tour)
                        .then(|| {
                            lessons()
                                .iter()
                                .skip_while(|lesson| lesson.id != self.lesson.id)
                                .skip(1)
                                .find(|lesson| lesson.chapter == Chapter::Tour)
                        })
                        .flatten();
                    self.phase = Phase::Between(next);
                }
                Effect::Left => todo.extend(self.leave(now)),
            }
        }
        todo
    }

    /// The tutorial is left: where it was is remembered.
    pub fn leave(&mut self, _now: Instant) -> Vec<Do> {
        if let Phase::Running(run) = &self.phase {
            self.progress.leave(self.lesson, run.at());
        }
        self.progress.running = false;
        self.progress.save(&self.file);
        vec![Do::End]
    }

    /// The desktop as it stands now.
    pub fn facts(&mut self, facts: Facts, now: Instant) -> Vec<Do> {
        self.facts = facts;
        let mut todo: Vec<Do> = self.beside_due(now).into_iter().collect();
        todo.extend(match &mut self.phase {
            Phase::Running(run) => {
                let effects = run.take(Input::Facts(self.facts.clone()), now);
                self.after(effects, now)
            }
            _ => self.tick(now),
        });
        todo
    }

    /// The window manager carried out a command of this name.
    pub fn carried_out(&mut self, name: &str, now: Instant) -> Vec<Do> {
        match &mut self.phase {
            Phase::Running(run) => {
                let effects = run.take(Input::CarriedOut(name.to_string()), now);
                self.after(effects, now)
            }
            _ => Vec::new(),
        }
    }

    /// Time passed.
    pub fn tick(&mut self, now: Instant) -> Vec<Do> {
        let have = self.practice();
        let laid_again = self.workspace_laid();
        match &mut self.phase {
            Phase::Setting {
                since,
                began,
                opened,
                aimed,
            } => {
                if !*opened {
                    let wanted = layout_named(self.lesson.scene.layout);
                    let on = self.facts.workspace == crate::WORKSPACE;
                    let laid = wanted
                        .as_str()
                        .is_some_and(|name| name == self.facts.layout);
                    // (Not waited for longer than this: the lesson is
                    // not held up by a layout that does not come.)
                    let waited = now.saturating_duration_since(*began) >= LAID_AT_MOST;
                    // (Shown and laid, and a moment more: seen on the
                    // desktop with two displays, the person's window
                    // that the switch hid gave the keyboard to a window
                    // on the other display a moment later, and the
                    // first practice window went there after it. The
                    // keyboard is taken to the tutorial's display after
                    // that moment, just before the windows come.)
                    let settled = now.saturating_duration_since(*since) >= SHOWN;
                    if (on && laid && settled) || waited {
                        *opened = true;
                        *since = now;
                        let wants = self.lesson.scene.windows;
                        // The card's room is kept free of tiles on the
                        // tutorial's workspace (which is the one shown
                        // now): the card lay over the foot of the window
                        // at the lower right.
                        if wants <= have {
                            return vec![card_room()];
                        }
                        // (New windows come where the keyboard is: on
                        // the tutorial's display.)
                        return vec![
                            card_room(),
                            self.display_focused(),
                            Do::OpenPractice(wants - have),
                        ];
                    }
                    if on && !laid && now.saturating_duration_since(*since) >= ASKED {
                        *since = now;
                        return vec![laid_again];
                    }
                    return Vec::new();
                }
                let there = self.facts.workspace == crate::WORKSPACE
                    && self.facts.windows.len() >= self.lesson.scene.windows;
                let late = now.saturating_duration_since(*began) >= AT_MOST;
                if !*aimed {
                    if (there && now.saturating_duration_since(*since) >= SETTLE) || late {
                        // The keyboard on the first window: up, then as
                        // far left as there is one. (Left last: seen on
                        // the desktop, "up" from the window at the left
                        // went to another, and the lesson began there.)
                        *aimed = true;
                        *since = now;
                        let go = |direction: &str| {
                            command(
                                serde_json::json!({"command": "focus_direction", "direction": direction}),
                            )
                        };
                        return vec![go("up"), go("up"), go("left"), go("left")];
                    }
                    return Vec::new();
                }
                if now.saturating_duration_since(*since) >= AIMED {
                    let mut run = Run::begin(self.lesson, self.from, now);
                    let effects = run.take(Input::Facts(self.facts.clone()), now);
                    self.phase = Phase::Running(Box::new(run));
                    return self.after(effects, now);
                }
                Vec::new()
            }
            Phase::Running(run) => {
                let effects = run.take(Input::Tick, now);
                self.after(effects, now)
            }
            Phase::Between(_) => Vec::new(),
        }
    }

    /// Something was pressed on the card.
    pub fn pressed(&mut self, input: Input, now: Instant) -> Vec<Do> {
        match (&mut self.phase, input) {
            (_, Input::Leave) => self.leave(now),
            (Phase::Running(run), input) => {
                let effects = run.take(input, now);
                self.after(effects, now)
            }
            // Between two lessons: on to the next, or the end.
            (Phase::Between(next), Input::Next) => match *next {
                Some(next) => {
                    self.lesson = next;
                    self.from = 0;
                    let mut todo = self.set_scene(now);
                    todo.extend(self.tick(now));
                    todo
                }
                None => self.leave(now),
            },
            _ => Vec::new(),
        }
    }

    /// What the card shows now.
    pub fn shown(&self, now: Instant) -> Shown<'_> {
        match &self.phase {
            Phase::Setting { .. } => Shown::Setting(self.lesson),
            Phase::Running(run) => Shown::Step {
                lesson: self.lesson,
                at: run.at(),
                step: run.step(),
                state: run.state(),
                help: run.help(now),
                paused: run.paused(),
            },
            Phase::Between(next) => Shown::Between {
                done: self.lesson,
                next: *next,
                count: self.progress.count(),
            },
        }
    }
}

/// The windows of the tutorial's workspace that are the person's own:
/// those that are no practice windows, where the workspace that is shown
/// is the tutorial's.
pub fn own_windows(facts: &Facts) -> Vec<u64> {
    if !crate::is_own_workspace(&facts.workspace) {
        return Vec::new();
    }
    facts
        .windows
        .iter()
        .filter(|window| !window.app.eq_ignore_ascii_case(crate::PRACTICE_APP))
        .map(|window| window.id)
        .collect()
}

/// What the window manager is asked to do when the tutorial has ended,
/// in this order: the person's own windows that are on the tutorial's
/// workspace go to the workspace that was shown before (a workspace with
/// a window on it is not closed: one window of the person's that had
/// come there kept the tutorial's workspace for good); the tutorial's
/// workspace is shown and closed; what was shown before is shown again.
/// `beside`: the workspace beside the tutorial's own is there, and is
/// closed first, the same way.
pub fn tidy_commands(before: &str, own: &[u64], beside: bool) -> Vec<serde_json::Value> {
    let mut commands = Vec::new();
    if !before.is_empty() {
        for window in own {
            commands.push(serde_json::json!({
                "command": "move_window_to_workspace",
                "window": {"kind": "handle", "value": window},
                "workspace": {"kind": "name", "value": before},
            }));
        }
    }
    let mut closed = |name: &str| {
        commands.push(serde_json::json!({
            "command": "switch_workspace_selector",
            "selector": {"kind": "name", "value": name},
        }));
        commands.push(serde_json::json!({"command": "close_workspace"}));
    };
    if beside {
        closed(crate::WORKSPACE_BESIDE);
    }
    closed(crate::WORKSPACE);
    if !before.is_empty() {
        commands.push(serde_json::json!({
            "command": "switch_workspace_selector",
            "selector": {"kind": "name", "value": before},
        }));
    }
    commands
}

/// Whether the tutorial's workspace is there, and the workspace to show
/// after it is closed: the one that is shown, or, where that is the
/// tutorial's own, the first other one that has windows (else the
/// first).
pub fn left_behind(wm: &serde_json::Value) -> Option<String> {
    let monitors = wm["monitors"].as_array()?;
    let own = |workspace: &&serde_json::Value| {
        workspace["name"]
            .as_str()
            .is_some_and(crate::is_own_workspace)
    };
    let monitor = monitors.iter().find(|monitor| {
        monitor["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|workspace| own(&workspace))
    })?;
    let shown = Facts::of(wm).workspace;
    if !crate::is_own_workspace(&shown) {
        return Some(shown);
    }
    let others: Vec<&serde_json::Value> = monitor["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|workspace| !own(workspace))
        .collect();
    let name = |workspace: &&serde_json::Value| workspace["name"].as_str().map(str::to_string);
    others
        .iter()
        .find(|workspace| workspace["container_count"].as_u64().unwrap_or(0) > 0)
        .and_then(name)
        .or_else(|| others.first().and_then(name))
        .or(Some(String::new()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_workspace_left_behind_is_found_with_the_one_to_go_back_to() {
        let state = |shown: &str, names: &[&str]| {
            serde_json::json!({"monitors": [{
                "is_focused": true,
                "workspace": {"name": shown, "layout_type": "Dwindle", "detailed_windows": []},
                "workspaces": names.iter().map(|name| serde_json::json!({"name": name})).collect::<Vec<_>>(),
            }]})
        };
        assert_eq!(super::left_behind(&state("1", &["1", "2"])), None);
        assert_eq!(
            super::left_behind(&state("2", &["1", "2", "Tutorial"])),
            Some("2".to_string())
        );
        // The tutorial's own is shown: back to the first other one.
        assert_eq!(
            super::left_behind(&state("Tutorial", &["1", "2", "Tutorial"])),
            Some("1".to_string())
        );
    }

    use super::*;
    use crate::Window;

    fn desk(workspace: &str, windows: usize, focused: u64) -> Facts {
        Facts {
            display: String::new(),
            workspace: workspace.to_string(),
            layout: "Dwindle".to_string(),
            windows: (1..=windows as u64)
                .map(|id| Window {
                    id,
                    at: (id as i32 * 100, 0, 100, 100),
                    focused: id == focused,
                    floating: false,
                    maximized: false,
                    app: crate::PRACTICE_APP.to_string(),
                })
                .collect(),
            containers: windows,
            displays: 1,
            surfaces: Vec::new(),
            workspaces: Vec::new(),
        }
    }

    fn file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("gevrek-tutoring-{name}-{}", std::process::id()))
            .join("tutorial.json")
    }

    #[test]
    fn the_ring_is_around_what_a_step_is_about() {
        use crate::Spot;
        // Two windows side by side; the second has the keyboard.
        let mut facts = desk(crate::WORKSPACE, 2, 2);
        facts.windows[0].at = (0, 40, 900, 1000);
        facts.windows[1].at = (900, 40, 1020, 1000);
        let panel = Some((0, 0, 1920, 36));
        assert_eq!(
            ring_around(Spot::Focused, &facts, panel),
            Some((900, 40, 1020, 1000))
        );
        assert_eq!(
            ring_around(Spot::Windows, &facts, panel),
            Some((0, 40, 1920, 1000))
        );
        assert_eq!(ring_around(Spot::Panel, &facts, panel), panel);
        // About nothing with a place, or about what is not there: none.
        assert_eq!(ring_around(Spot::None, &facts, panel), None);
        assert_eq!(
            ring_around(Spot::Surface("gevrek-launcher"), &facts, panel),
            None
        );
        assert_eq!(ring_around(Spot::Panel, &facts, None), None);
        let empty = desk(crate::WORKSPACE, 0, 0);
        assert_eq!(ring_around(Spot::Focused, &empty, panel), None);
        assert_eq!(ring_around(Spot::Windows, &empty, panel), None);

        // Its four sides lie around the rectangle, outside it, and meet
        // at the corners without lying over each other.
        let around = (100, 200, 600, 400);
        let sides = ring_sides(around);
        let outer = (
            around.0 - RING_GAP - RING,
            around.1 - RING_GAP - RING,
            around.2 + 2 * (RING_GAP + RING),
            around.3 + 2 * (RING_GAP + RING),
        );
        for side in &sides {
            assert!(side.2 > 0 && side.3 > 0);
            // Within the outer rectangle, and nowhere on what it is around.
            assert!(side.0 >= outer.0 && side.0 + side.2 <= outer.0 + outer.2);
            assert!(side.1 >= outer.1 && side.1 + side.3 <= outer.1 + outer.3);
            let apart = side.0 + side.2 <= around.0
                || around.0 + around.2 <= side.0
                || side.1 + side.3 <= around.1
                || around.1 + around.3 <= side.1;
            assert!(apart, "{side:?} lies on {around:?}");
        }
        let area: i32 = sides.iter().map(|side| side.2 * side.3).sum();
        assert_eq!(
            area,
            outer.2 * outer.3 - (outer.2 - 2 * RING) * (outer.3 - 2 * RING)
        );
    }

    #[test]
    fn the_cards_room_is_kept_free_of_tiles() {
        use gevrek_model::command::DispatchCommand;
        let Do::Dispatch(json) = card_room() else {
            panic!("a command");
        };
        let command: DispatchCommand = serde_json::from_str(&json).expect("a command");
        // All of the card, its distance from the edge, and a little more.
        let wanted = (CARD_SIZE.1 + CARD_MARGIN) as i32;
        assert!(matches!(
            command,
            DispatchCommand::SetWorkspaceWorkAreaOffset { left: 0, top: 0, right: 0, bottom }
                if bottom > wanted && bottom < wanted + 40
        ));
    }

    #[test]
    fn the_tutorial_is_on_the_display_the_person_is_on() {
        use gevrek_model::command::{DispatchCommand, MonitorTarget};
        let targets = |todo: &[Do]| -> Vec<MonitorTarget> {
            todo.iter()
                .filter_map(|one| match one {
                    Do::Dispatch(json) => serde_json::from_str::<DispatchCommand>(json).ok(),
                    _ => None,
                })
                .filter_map(|command| match command {
                    DispatchCommand::FocusMonitor { target }
                    | DispatchCommand::EnsureNamedWorkspace { target, .. } => Some(target),
                    _ => None,
                })
                .collect()
        };
        // Begun on the second display: everything of it is asked there.
        let mut second = desk("mail", 0, 0);
        second.display = "DELL (2)".to_string();
        let (_, todo) =
            Tutoring::begin(None, file("display-second"), &second, Instant::now()).expect("begun");
        let asked = targets(&todo);
        assert!(!asked.is_empty());
        assert!(asked
            .iter()
            .all(|target| *target == MonitorTarget::Name("DELL (2)".to_string())));
        // Where the desktop does not say which display that is: the main one.
        let (_, todo) = Tutoring::begin(
            None,
            file("display-unknown"),
            &desk("mail", 0, 0),
            Instant::now(),
        )
        .expect("begun");
        let asked = targets(&todo);
        assert!(!asked.is_empty());
        assert!(asked.iter().all(|target| *target == MonitorTarget::Primary));
    }

    #[test]
    fn the_persons_own_windows_leave_the_tutorials_workspace_with_it() {
        use gevrek_model::command::{DispatchCommand, WindowSelector, WorkspaceSelector};
        // On the tutorial's workspace: two practice windows and one of
        // the person's own.
        let mut facts = desk(crate::WORKSPACE, 2, 1);
        let mut own = facts.windows[0].clone();
        own.id = 77;
        own.app = "org.mozilla.firefox".to_string();
        facts.windows.push(own);
        assert_eq!(own_windows(&facts), [77]);
        // On any other workspace nothing is the tutorial's to move.
        assert!(own_windows(&desk("mail", 2, 1)).is_empty());

        // It goes to the workspace of before, then the tutorial's is
        // closed, then that one is shown: each a command the model reads.
        let commands: Vec<DispatchCommand> = tidy_commands("mail", &[77], false)
            .iter()
            .map(|json| serde_json::from_value(json.clone()).expect("a command"))
            .collect();
        let name = |name: &str| WorkspaceSelector::Name(name.to_string());
        assert_eq!(
            commands,
            [
                DispatchCommand::MoveWindowToWorkspace {
                    window: WindowSelector::Handle(77),
                    workspace: name("mail"),
                },
                DispatchCommand::SwitchWorkspaceSelector {
                    selector: name(crate::WORKSPACE),
                },
                DispatchCommand::CloseWorkspace,
                DispatchCommand::SwitchWorkspaceSelector {
                    selector: name("mail"),
                },
            ]
        );
        // With no workspace of before there is nowhere to take them, and
        // nothing to show after: the tutorial's is only tried to close.
        assert_eq!(tidy_commands("", &[77], false).len(), 2);
    }

    #[test]
    fn a_lesson_begins_on_its_own_workspace_when_its_scene_is_set() {
        let file = file("begins");
        let _ = std::fs::remove_file(&file);
        let start = Instant::now();
        let (mut tutoring, todo) =
            Tutoring::begin(Some("focus"), file.clone(), &desk("1", 2, 1), start)
                .expect("a lesson");
        // Its workspace is made and shown; its layout and its windows are
        // not asked for yet.
        let said: Vec<String> = todo
            .iter()
            .map(|one| match one {
                Do::Dispatch(command) => command.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert!(said
            .iter()
            .any(|one| one.contains("ensure_named_workspace") && one.contains("Tutorial")));
        assert!(said
            .iter()
            .any(|one| one.contains("switch_workspace_selector")));
        assert!(said
            .iter()
            .any(|one| one.contains("ensure_named_workspace") && one.contains("Dwindle")));
        assert!(said
            .iter()
            .all(|one| !one.contains("set_layout") && !one.contains("OpenPractice")));
        assert_eq!(tutoring.before, "1");
        assert!(matches!(tutoring.shown(start), Shown::Setting(_)));
        assert!(Progress::load(&file).running);
        // Still on the workspace of before: nothing is asked of it.
        assert!(tutoring.facts(desk("1", 2, 1), start + ASKED).is_empty());
        // The tutorial's workspace is shown, with another layout: the
        // scene's is asked for there, and no window is opened before it.
        let mut bare = desk("Tutorial", 0, 0);
        bare.layout = "BSP".to_string();
        let asked = tutoring.facts(bare, start + ASKED);
        assert!(
            matches!(&asked[..], [Do::Dispatch(command)] if command.contains("ensure_named_workspace") && command.contains("Dwindle")),
            "{asked:?}"
        );
        // The layout is there: a moment, and the windows are opened.
        assert!(tutoring
            .facts(desk("Tutorial", 0, 0), start + ASKED * 2)
            .is_empty());
        assert_eq!(
            tutoring
                .facts(desk("Tutorial", 0, 0), start + ASKED + SHOWN)
                .last(),
            Some(&Do::OpenPractice(3))
        );
        // They are not there yet: nothing begins.
        assert!(tutoring
            .facts(desk("Tutorial", 1, 1), start + SETTLE)
            .is_empty());
        assert!(matches!(tutoring.shown(start + SETTLE), Shown::Setting(_)));
        // There, and settled: the keyboard is put on the first window.
        let then = start + ASKED + SHOWN + SETTLE + Duration::from_millis(100);
        let aimed = tutoring.facts(desk("Tutorial", 3, 3), then);
        assert_eq!(aimed.len(), 4);
        assert!(aimed.iter().all(
            |one| matches!(one, Do::Dispatch(command) if command.contains("focus_direction"))
        ));
        // (Once: not again with the next look.)
        assert!(tutoring.tick(then + Duration::from_millis(100)).is_empty());
        // And a moment later the lesson's first step waits.
        let begun = then + AIMED;
        tutoring.facts(desk("Tutorial", 3, 1), begun - Duration::from_millis(10));
        assert!(tutoring.tick(begun).is_empty());
        assert!(matches!(
            tutoring.shown(begun),
            Shown::Step {
                at: 0,
                state: StepState::Waiting,
                ..
            }
        ));
        // What happened while the scene was set is nothing done: the
        // step is measured from the desktop as the lesson began.
        tutoring.facts(desk("Tutorial", 3, 2), begun + Duration::from_secs(1));
        assert!(matches!(
            tutoring.shown(begun),
            Shown::Step {
                state: StepState::Done,
                ..
            }
        ));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn between_lessons_the_person_goes_on_or_leaves_and_it_is_remembered() {
        let file = file("between");
        let _ = std::fs::remove_file(&file);
        let start = Instant::now();
        let (mut tutoring, _) =
            Tutoring::begin(Some("move"), file.clone(), &desk("1", 0, 0), start).expect("a lesson");
        // The scene never comes: the lesson begins all the same.
        let late = start + AT_MOST;
        assert_eq!(tutoring.tick(late).last(), Some(&Do::OpenPractice(3)));
        assert_eq!(tutoring.tick(late).len(), 4);
        tutoring.tick(late + AIMED);
        assert!(tutoring.is_running());
        // Past both of its steps: finished, and the next of the tour offered.
        tutoring.pressed(Input::Skip, late + AIMED);
        tutoring.pressed(Input::Skip, late + AIMED);
        let Shown::Between { done, next, count } = tutoring.shown(late + AIMED) else {
            panic!("between two lessons");
        };
        assert_eq!(
            (done.id, next.map(|next| next.id)),
            ("move", Some("workspaces"))
        );
        assert_eq!(count.0, 1);
        assert!(Progress::load(&file).has_finished(done));
        // On: the next lesson's scene, with the windows that are there kept.
        tutoring.facts(desk("Tutorial", 3, 1), late + AIMED);
        let todo = tutoring.pressed(Input::Next, late + AIMED);
        assert_eq!(tutoring.lesson().id, "workspaces");
        assert!(
            todo.iter().all(|one| !matches!(one, Do::OpenPractice(_))),
            "{todo:?}"
        );
        assert!(matches!(tutoring.shown(late + AIMED), Shown::Setting(_)));
        // Left in the middle of a lesson: remembered where.
        tutoring.tick(late + AIMED + SHOWN);
        tutoring.tick(late + AIMED + SHOWN + SETTLE);
        tutoring.tick(late + AIMED + SHOWN + SETTLE + AIMED);
        tutoring.pressed(Input::Skip, late + AT_MOST);
        assert_eq!(tutoring.pressed(Input::Leave, late + AT_MOST), [Do::End]);
        let kept = Progress::load(&file);
        assert_eq!(kept.left_at, Some(("workspaces".to_string(), 1)));
        assert!(!kept.running);
        // Taken up again: that lesson, at that step.
        let (again, _) =
            Tutoring::begin(None, file.clone(), &desk("1", 0, 0), start).expect("a lesson");
        assert_eq!((again.lesson().id, again.from), ("workspaces", 1));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }
}
