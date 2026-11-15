//! The tutorial while it is taken, as the shell carries it: the scene of
//! a lesson is set on the tutorial's own workspace, the lesson runs, and
//! between two lessons the person says whether to go on.
//!
//! This is the part that is the same on every system: it is told how the
//! desktop stands and what was pressed, and says what the shell is to do
//! (a command for the window manager, practice windows to open, the
//! card to draw again, the tutorial's end). Each system's half opens the
//! card's surface, starts and ends the practice windows, and hands the
//! commands on.

use gevrek_tutorial::lessons::{lessons, Chapter};
use gevrek_tutorial::{Effect, Facts, Help, Input, Lesson, Progress, Run, Step, StepState};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
    Setting { since: Instant, began: Instant, opened: bool, aimed: bool },
    Running(Run),
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
    facts: Facts,
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
        let (lesson, from) = match wanted.and_then(gevrek_tutorial::lessons::lesson) {
            Some(lesson) => (lesson, 0),
            None => progress.next().or_else(|| lessons().first().map(|first| (first, 0)))?,
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
                .filter(|name| name != gevrek_tutorial::WORKSPACE)
                .unwrap_or_default(),
            facts: now_shown.clone(),
        };
        let mut todo = vec![
            command(serde_json::json!({"command": "focus_monitor", "target": {"kind": "primary"}})),
            tutoring.workspace_laid(),
            command(serde_json::json!({
                "command": "switch_workspace_selector",
                "selector": {"kind": "name", "value": gevrek_tutorial::WORKSPACE},
            })),
        ];
        todo.extend(tutoring.set_scene(now));
        Some((tutoring, todo))
    }

    /// The tutorial's workspace, there and with the lesson's layout: said
    /// of the workspace by its name. (Seen on the desktop, with two
    /// displays: a layout asked of "the workspace" went to the one on
    /// the display the pointer was on, and that one of the person's
    /// floated.)
    fn workspace_laid(&self) -> Do {
        command(serde_json::json!({
            "command": "ensure_named_workspace",
            "target": {"kind": "primary"},
            "name": gevrek_tutorial::WORKSPACE,
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
        if self.facts.workspace == gevrek_tutorial::WORKSPACE {
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
        match &mut self.phase {
            Phase::Running(run) => {
                let effects = run.take(Input::Facts(self.facts.clone()), now);
                self.after(effects, now)
            }
            _ => self.tick(now),
        }
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
            Phase::Setting { since, began, opened, aimed } => {
                if !*opened {
                    let wanted = layout_named(self.lesson.scene.layout);
                    let on = self.facts.workspace == gevrek_tutorial::WORKSPACE;
                    let laid = wanted.as_str().is_some_and(|name| name == self.facts.layout);
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
                        if wants <= have {
                            return Vec::new();
                        }
                        // (New windows come where the keyboard is: on
                        // the tutorial's display.)
                        return vec![
                            command(serde_json::json!({"command": "focus_monitor", "target": {"kind": "primary"}})),
                            Do::OpenPractice(wants - have),
                        ];
                    }
                    if on && !laid && now.saturating_duration_since(*since) >= ASKED {
                        *since = now;
                        return vec![laid_again];
                    }
                    return Vec::new();
                }
                let there = self.facts.workspace == gevrek_tutorial::WORKSPACE
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
                            command(serde_json::json!({"command": "focus_direction", "direction": direction}))
                        };
                        return vec![go("up"), go("up"), go("left"), go("left")];
                    }
                    return Vec::new();
                }
                if now.saturating_duration_since(*since) >= AIMED {
                    let mut run = Run::begin(self.lesson, self.from, now);
                    let effects = run.take(Input::Facts(self.facts.clone()), now);
                    self.phase = Phase::Running(run);
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

/// The names the practice windows are opened by, in turn.
const PRACTICE: [&str; 6] = ["Notes", "Browser", "Music", "Mail", "Files", "Photos"];

/// Open this many practice windows more: the settings program beside
/// this one is a practice window when it is asked to be.
pub fn open_practice(windows: &mut Vec<std::process::Child>, count: usize) {
    let program = std::env::current_exe().ok().and_then(|own| {
        Some(own.parent()?.join(format!("gevrek-studio{}", std::env::consts::EXE_SUFFIX)))
    });
    let Some(program) = program.filter(|program| program.is_file()) else {
        log::warn!("no practice window: gevrek-studio is not beside the shell");
        return;
    };
    for _ in 0..count {
        let name = PRACTICE[windows.len() % PRACTICE.len()];
        // Held by a pipe: a practice window goes when whoever opened it
        // is gone, however that came (`HELD`).
        match std::process::Command::new(&program)
            .args(["--practice", name])
            .env(gevrek_tutorial::HELD, "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(window) => windows.push(window),
            Err(error) => log::warn!("a practice window was not opened: {error}"),
        }
    }
}

/// The tutorial has ended: its practice windows are closed, its
/// workspace goes once they are gone (a moment), and what was shown
/// before (`before`: a workspace's name) is shown again.
pub fn tidy(
    before: String,
    windows: Vec<std::process::Child>,
    tell: impl Fn(gevrek_link::Request) + Send + 'static,
) {
    for mut window in windows {
        let _ = window.kill();
        let _ = window.wait();
    }
    std::thread::spawn(move || {
        let dispatch =
            |command: serde_json::Value| tell(gevrek_link::Request::Dispatch(command.to_string()));
        std::thread::sleep(Duration::from_millis(600));
        dispatch(serde_json::json!({
            "command": "switch_workspace_selector",
            "selector": {"kind": "name", "value": gevrek_tutorial::WORKSPACE},
        }));
        dispatch(serde_json::json!({"command": "close_workspace"}));
        if !before.is_empty() {
            dispatch(serde_json::json!({
                "command": "switch_workspace_selector",
                "selector": {"kind": "name", "value": before},
            }));
        }
    });
}

/// A tutorial that did not end by itself (the shell was stopped, the
/// session ended) has left its workspace behind: it is closed, once,
/// when the window manager first says how the desktop stands. Its
/// practice windows went with the shell that held them.
pub fn tidy_after_a_stop(
    wm: &serde_json::Value,
    asked: &mut bool,
    tell: impl Fn(gevrek_link::Request) + Send + 'static,
) {
    if std::mem::replace(asked, true) {
        return;
    }
    let file = gevrek_tutorial::progress::file();
    let mut progress = Progress::load(&file);
    if !progress.running {
        return;
    }
    progress.running = false;
    progress.save(&file);
    if let Some(before) = left_behind(wm) {
        log::info!("a tutorial that was stopped is tidied up after");
        tidy(before, Vec::new(), tell);
    }
}

/// Whether the tutorial's workspace is there, and the workspace to show
/// after it is closed: the one that is shown, or, where that is the
/// tutorial's own, the first other one that has windows (else the
/// first).
fn left_behind(wm: &serde_json::Value) -> Option<String> {
    let monitors = wm["monitors"].as_array()?;
    let own = |workspace: &&serde_json::Value| workspace["name"] == gevrek_tutorial::WORKSPACE;
    let monitor = monitors.iter().find(|monitor| {
        monitor["workspaces"].as_array().into_iter().flatten().any(|workspace| own(&workspace))
    })?;
    let shown = Facts::of(wm).workspace;
    if shown != gevrek_tutorial::WORKSPACE {
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

/// Open the welcome where a person is to be welcomed (a first launch).
/// Asked once for each run of the shell, with the configuration that
/// runs.
pub fn welcome_if_due(
    running: &gevrek_model::Config,
    asked: &mut bool,
    tell: impl Fn(gevrek_link::Request) + Send + 'static,
) {
    if !std::mem::replace(asked, true) && gevrek_tutorial::welcome_due(running) {
        log::info!("the first launch: the welcome is opened");
        open_welcome(tell);
    }
}

/// Open the welcome: the settings program beside this one shows it. When
/// it ends with the tour asked for, the tour is begun: after the
/// welcome's program is gone, and its window with it.
pub fn open_welcome(tell: impl Fn(gevrek_link::Request) + Send + 'static) {
    let program = std::env::current_exe().ok().and_then(|own| {
        Some(own.parent()?.join(format!("gevrek-studio{}", std::env::consts::EXE_SUFFIX)))
    });
    let Some(program) = program.filter(|program| program.is_file()) else {
        log::warn!("no welcome: gevrek-studio is not beside the shell");
        return;
    };
    let welcome = std::process::Command::new(&program)
        .arg("--welcome")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut welcome = match welcome {
        Ok(welcome) => welcome,
        Err(error) => {
            log::warn!("the welcome was not opened: {error}");
            return;
        }
    };
    std::thread::spawn(move || {
        let asked = welcome
            .wait()
            .is_ok_and(|ended| ended.code() == Some(gevrek_tutorial::TOUR_ASKED));
        if asked {
            // (A moment: the desktop takes in that the window is gone.)
            std::thread::sleep(Duration::from_millis(600));
            tell(gevrek_link::Request::Dispatch(
                serde_json::json!({"command": "shell", "action": "tutorial"}).to_string(),
            ));
        }
    });
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
    use gevrek_tutorial::Window;

    fn desk(workspace: &str, windows: usize, focused: u64) -> Facts {
        Facts {
            workspace: workspace.to_string(),
            layout: "Dwindle".to_string(),
            windows: (1..=windows as u64)
                .map(|id| Window {
                    id,
                    at: (id as i32 * 100, 0, 100, 100),
                    focused: id == focused,
                    floating: false,
                    maximized: false,
                    app: gevrek_tutorial::PRACTICE_APP.to_string(),
                })
                .collect(),
            containers: windows,
            displays: 1,
            surfaces: Vec::new(),
        }
    }

    fn file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("gevrek-tutoring-{name}-{}", std::process::id()))
            .join("tutorial.json")
    }

    #[test]
    fn a_lesson_begins_on_its_own_workspace_when_its_scene_is_set() {
        let file = file("begins");
        let _ = std::fs::remove_file(&file);
        let start = Instant::now();
        let (mut tutoring, todo) =
            Tutoring::begin(Some("focus"), file.clone(), &desk("1", 2, 1), start).expect("a lesson");
        // Its workspace is made and shown; its layout and its windows are
        // not asked for yet.
        let said: Vec<String> = todo
            .iter()
            .map(|one| match one {
                Do::Dispatch(command) => command.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert!(said.iter().any(|one| one.contains("ensure_named_workspace") && one.contains("Tutorial")));
        assert!(said.iter().any(|one| one.contains("switch_workspace_selector")));
        assert!(said.iter().any(|one| one.contains("ensure_named_workspace") && one.contains("Dwindle")));
        assert!(said.iter().all(|one| !one.contains("set_layout") && !one.contains("OpenPractice")));
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
        assert!(tutoring.facts(desk("Tutorial", 0, 0), start + ASKED * 2).is_empty());
        assert_eq!(
            tutoring.facts(desk("Tutorial", 0, 0), start + ASKED + SHOWN).last(),
            Some(&Do::OpenPractice(3))
        );
        // They are not there yet: nothing begins.
        assert!(tutoring.facts(desk("Tutorial", 1, 1), start + SETTLE).is_empty());
        assert!(matches!(tutoring.shown(start + SETTLE), Shown::Setting(_)));
        // There, and settled: the keyboard is put on the first window.
        let then = start + ASKED + SHOWN + SETTLE + Duration::from_millis(100);
        let aimed = tutoring.facts(desk("Tutorial", 3, 3), then);
        assert_eq!(aimed.len(), 4);
        assert!(aimed.iter().all(|one| matches!(one, Do::Dispatch(command) if command.contains("focus_direction"))));
        // (Once: not again with the next look.)
        assert!(tutoring.tick(then + Duration::from_millis(100)).is_empty());
        // And a moment later the lesson's first step waits.
        let begun = then + AIMED;
        tutoring.facts(desk("Tutorial", 3, 1), begun - Duration::from_millis(10));
        assert!(tutoring.tick(begun).is_empty());
        assert!(matches!(tutoring.shown(begun), Shown::Step { at: 0, state: StepState::Waiting, .. }));
        // What happened while the scene was set is nothing done: the
        // step is measured from the desktop as the lesson began.
        tutoring.facts(desk("Tutorial", 3, 2), begun + Duration::from_secs(1));
        assert!(matches!(tutoring.shown(begun), Shown::Step { state: StepState::Done, .. }));
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
        assert_eq!((done.id, next.map(|next| next.id)), ("move", Some("workspaces")));
        assert_eq!(count.0, 1);
        assert!(Progress::load(&file).has_finished(done));
        // On: the next lesson's scene, with the windows that are there kept.
        tutoring.facts(desk("Tutorial", 3, 1), late + AIMED);
        let todo = tutoring.pressed(Input::Next, late + AIMED);
        assert_eq!(tutoring.lesson().id, "workspaces");
        assert!(todo.iter().all(|one| !matches!(one, Do::OpenPractice(_))), "{todo:?}");
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
        let (again, _) = Tutoring::begin(None, file.clone(), &desk("1", 0, 0), start).expect("a lesson");
        assert_eq!((again.lesson().id, again.from), ("workspaces", 1));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }
}
