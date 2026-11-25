//! Every lesson of the tutorial, played from its first step to its last
//! against the real window manager: the engine's core and the shared host
//! on a desktop in memory, with the state read the way the shell reads it
//! (what the host publishes, made into the protocol's message, read as
//! JSON). Each step is done the way a person does it (the step's own
//! command, as its key would run it; the card's button where that is the
//! way; Escape for what is put away) and has to be seen done.
//!
//! What is not here: the card's drawing, the practice windows as programs
//! and the shell's own surfaces. They are stood in for: a practice window
//! is a window of the practice application, a surface of the shell is up
//! when its command was carried out.

use gevrek_engine::core::WmCore;
use gevrek_engine::events::WindowEvent;
use gevrek_engine::executor::{Executor, NodeExecutionEvent};
use gevrek_engine::focus_manager::FocusManager;
use gevrek_engine::input::Hotkey;
use gevrek_engine::outputs::{MonitorHandle, Outputs};
use gevrek_engine::rules::RuleEngine;
use gevrek_engine::snap_zones::SnapZoneManager;
use gevrek_engine::testing::FakeDesktop;
use gevrek_engine::WindowHandle;
use gevrek_host::wm::commands::Commands;
use gevrek_host::wm::published::{Published, SystemState};
use gevrek_host::wm::{HostState, Parts, WmHost};
use gevrek_model::command::{DispatchCommand, ShellAction};
use gevrek_model::config::Config;
use gevrek_tutorial::lessons::{Act, Lesson};
use gevrek_tutorial::tutoring::{tidy_commands, Do, Shown, Tutoring};
use gevrek_tutorial::{Facts, Goal, Input, StepState};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OUTPUT: MonitorHandle = MonitorHandle(1);

/// The window manager: the real core and the shared host, on a desktop in
/// memory, with the configuration gevrek starts with.
struct Wm {
    config: Config,
    core: WmCore,
    desktop: FakeDesktop,
    host: HostState,
    rules: Arc<Mutex<RuleEngine>>,
    /// The commands it said it carried out, by their names, since they
    /// were last taken.
    carried_out: Arc<Mutex<Vec<String>>>,
    /// The windows it knows of (to tell it of those that went).
    known: BTreeSet<WindowHandle>,
}

impl WmHost for Wm {
    type Desktop = FakeDesktop;

    fn parts(&mut self) -> Parts<'_, FakeDesktop> {
        Parts {
            config: &mut self.config,
            core: &mut self.core,
            desktop: &mut self.desktop,
            host: &mut self.host,
        }
    }
    fn host(&self) -> &HostState {
        &self.host
    }
    fn config(&self) -> &Config {
        &self.config
    }
    fn settle(&mut self) {
        self.core
            .flush_pending_layouts(&mut self.desktop)
            .expect("laid out");
    }
    fn install_bindings(&mut self, _bindings: Vec<(i32, Hotkey)>) -> anyhow::Result<()> {
        Ok(())
    }
    fn apply_appearance(&mut self) {
        self.core
            .apply_workspace_appearance(&self.config.desktop.appearance);
    }
    fn set_animations(&mut self, _animations: &gevrek_model::settings::AnimationProfile) {}
    fn configure_monitors(&mut self) {
        gevrek_engine::config_apply::configure_monitor_profiles(
            &mut self.core.monitor_manager,
            &self.config.desktop,
        );
    }
    fn rules(&self) -> &Arc<Mutex<RuleEngine>> {
        &self.rules
    }
    fn system_state(&self) -> SystemState {
        SystemState::default()
    }
    // (The list of shortcuts is a window of gevrek's own: not opened here.)
    fn show_shortcuts(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn workspace_overview(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl Wm {
    fn new() -> Self {
        let config = gevrek_model::examples::starting_configuration();
        let rules = Arc::new(Mutex::new(gevrek_engine::config_apply::build_rule_engine(
            &config.desktop,
        )));
        let mut executor = Executor::new(
            config.node_graph.clone(),
            Arc::new(Mutex::new(Vec::new())),
            rules.clone(),
        );
        executor.set_scratchpad_profiles(config.desktop.scratchpads.clone());
        executor.set_workspace_profiles(
            config.desktop.workspaces.clone(),
            config.desktop.appearance.clone(),
        );
        let mut outputs = Outputs::with_workspace_count(0);
        outputs.add_test_monitor(OUTPUT, "Primary");
        let core = WmCore::new(
            executor,
            outputs,
            FocusManager::with_config(config.desktop.focus.clone()),
            SnapZoneManager::new(Default::default()),
        );
        let carried_out = Arc::new(Mutex::new(Vec::new()));
        let told = carried_out.clone();
        let host = HostState {
            events: Some(Arc::new(move |event| {
                let NodeExecutionEvent::Runtime(event) = event else {
                    return;
                };
                if event.kind.as_str() != "dispatch.succeeded" {
                    return;
                }
                // (As the shell reads it: the command's name in its JSON.)
                let name = event
                    .fields
                    .iter()
                    .find(|(name, _)| name.as_str() == "payload")
                    .and_then(|(_, payload)| {
                        serde_json::from_str::<serde_json::Value>(payload).ok()
                    })
                    .and_then(|payload| payload["command"].as_str().map(str::to_string));
                if let Some(name) = name {
                    told.lock().unwrap().push(name);
                }
            })),
            overlay: Some(Arc::new(Mutex::new(None))),
            ..HostState::default()
        };
        let mut wm = Self {
            config,
            core,
            desktop: FakeDesktop::new(OUTPUT),
            host,
            rules,
            carried_out,
            known: BTreeSet::new(),
        };
        wm.configure_monitors();
        wm.apply_appearance();
        // The first workspace is shown, as when gevrek starts.
        wm.dispatch(r#"{"command":"switch_workspace_selector","selector":{"kind":"name","value":"1"}}"#)
            .expect("the first workspace");
        wm.take_carried_out();
        wm
    }

    /// A window opens, of this application.
    fn open(&mut self, title: &str, app: &str) -> WindowHandle {
        let id = self.desktop.open(title, app);
        if let Some(window) = self.desktop.windows.get_mut(&id) {
            window.class = app.to_string();
        }
        self.core
            .handle_window_event(&mut self.desktop, WindowEvent::Created(id))
            .expect("a window that opens is taken in");
        self.known.insert(id);
        self.settle();
        id
    }

    /// The window manager hears of the windows that went.
    fn hear_of_closed(&mut self) {
        let gone: Vec<WindowHandle> = self
            .known
            .iter()
            .copied()
            .filter(|id| !self.desktop.exists_window(*id))
            .collect();
        for id in gone {
            self.known.remove(&id);
            self.core
                .handle_window_event(&mut self.desktop, WindowEvent::Destroyed(id))
                .expect("a window that went is let go");
        }
        self.settle();
    }

    /// A command, as a key, the card or the shell hands it over. Whether
    /// it was carried out.
    fn dispatch(&mut self, json: &str) -> Result<(), String> {
        let command: DispatchCommand =
            serde_json::from_str(json).map_err(|error| format!("{json}: {error}"))?;
        let result = self.run_command(command).map_err(|error| format!("{json}: {error}"));
        self.hear_of_closed();
        result
    }

    /// The state as the shell reads it.
    fn state(&mut self) -> serde_json::Value {
        self.publish_state(None);
        let data = self
            .host
            .overlay
            .as_ref()
            .and_then(|slot| slot.lock().unwrap().clone())
            .expect("published");
        let response = gevrek_server::overlay_response(
            &data,
            &self.config.desktop.appearance,
            self.config.desktop.platform.windows.shadow,
        );
        serde_json::to_value(&response).expect("the state as JSON")
    }

    fn take_carried_out(&self) -> Vec<String> {
        std::mem::take(&mut *self.carried_out.lock().unwrap())
    }
}

/// What a person's desktop looked like: each window with its workspace
/// and its place.
fn standing(state: &serde_json::Value, apps: &[&str]) -> Vec<(String, String, String)> {
    let mut all = Vec::new();
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
            for window in workspace["detailed_windows"].as_array().into_iter().flatten() {
                let app = window["class"].as_str().unwrap_or_default();
                if apps.contains(&app) {
                    all.push((
                        window["title"].as_str().unwrap_or_default().to_string(),
                        workspace["name"].as_str().unwrap_or_default().to_string(),
                        format!("{} {}", window["position"], window["size"]),
                    ));
                }
            }
        }
    }
    all.sort();
    all
}

/// The names of the workspaces with a practice window on them.
fn practice_on(state: &serde_json::Value) -> BTreeSet<String> {
    standing(state, &[gevrek_tutorial::PRACTICE_APP])
        .into_iter()
        .map(|(_, workspace, _)| workspace)
        .collect()
}

fn names(state: &serde_json::Value) -> Vec<String> {
    state["monitors"][0]["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|workspace| workspace["name"].as_str().map(str::to_string))
        .collect()
}

/// The desk a tutorial is played on.
struct Desk {
    wm: Wm,
    /// The person puts a surface of the shell away as soon as they have
    /// read about it, before the card asks them to.
    hasty: bool,
    /// The shell's own surfaces that are up.
    surfaces: BTreeSet<&'static str>,
    now: Instant,
    practice: usize,
    /// Everything that was asked and went wrong.
    refused: Vec<String>,
}

impl Desk {
    fn new() -> Self {
        Self {
            wm: Wm::new(),
            hasty: false,
            surfaces: BTreeSet::new(),
            now: Instant::now(),
            practice: 0,
            refused: Vec::new(),
        }
    }

    fn facts(&mut self) -> Facts {
        let mut facts = Facts::of(&self.wm.state());
        facts.surfaces = self.surfaces.iter().map(|name| name.to_string()).collect();
        facts
    }

    /// A command: the window manager's, and what the shell does on being
    /// told (its launcher and its control centre come up and go).
    fn command(&mut self, json: &str) {
        let asked: Option<DispatchCommand> = serde_json::from_str(json).ok();
        match self.wm.dispatch(json) {
            Ok(()) => {
                let surface = match asked {
                    Some(DispatchCommand::Shell { action: ShellAction::Launcher }) => {
                        Some("gevrek-launcher")
                    }
                    Some(DispatchCommand::Shell { action: ShellAction::ControlCenter }) => {
                        Some("gevrek-control-centre")
                    }
                    _ => None,
                };
                if let Some(surface) = surface {
                    if !self.surfaces.remove(surface) {
                        self.surfaces.insert(surface);
                    }
                }
            }
            Err(error) => self.refused.push(error),
        }
    }

    /// Carry out what the tutorial asks for. Whether it ended.
    fn carry_out(&mut self, todo: Vec<Do>) -> bool {
        let mut ended = false;
        for one in todo {
            match one {
                // (What the tutorial asks for itself may be refused:
                // the keyboard is sent up and left until there is no
                // window further that way.)
                Do::Dispatch(json) => {
                    let before = self.refused.len();
                    self.command(&json);
                    if json.contains("focus_direction") {
                        self.refused.truncate(before);
                    }
                }
                Do::OpenPractice(count) => {
                    for _ in 0..count {
                        self.practice += 1;
                        let title = format!("Practice {}", self.practice);
                        self.wm.open(&title, gevrek_tutorial::PRACTICE_APP);
                    }
                }
                Do::End => ended = true,
            }
        }
        ended
    }

    /// A moment passes: the tutorial is told how the desktop stands, what
    /// was carried out, and the time. Whether it ended.
    fn moment(&mut self, tutoring: &mut Tutoring) -> bool {
        self.now += Duration::from_millis(100);
        let mut todo = Vec::new();
        for name in self.wm.take_carried_out() {
            todo.extend(tutoring.carried_out(&name, self.now));
        }
        let facts = self.facts();
        todo.extend(tutoring.facts(facts, self.now));
        todo.extend(tutoring.tick(self.now));
        self.carry_out(todo)
    }

    /// The card's button.
    fn press(&mut self, tutoring: &mut Tutoring, input: Input) -> bool {
        let todo = tutoring.pressed(input, self.now);
        self.carry_out(todo)
    }
}

fn memory(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "gevrek-tutorial-played-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    folder.join("tutorial.json")
}

/// Play the tutorial from where its memory says, to its end, each step
/// done as a person does it. The lessons that were finished, in order.
/// `check`: looked at after every moment.
fn play(
    desk: &mut Desk,
    wanted: Option<&str>,
    file: &std::path::Path,
    mut check: impl FnMut(&mut Desk, &str),
) -> Vec<&'static str> {
    let facts = desk.facts();
    let (mut tutoring, todo) =
        Tutoring::begin(wanted, file.to_path_buf(), &facts, desk.now).expect("a lesson to take");
    let before = tutoring.before.clone();
    assert!(!desk.carry_out(todo));
    let mut finished = Vec::new();
    // The step that was done by hand already (a lesson, a step).
    let mut acted: Option<(&'static str, usize)> = None;
    let mut waited = 0;
    let mut ended = false;
    for _ in 0..20_000 {
        if desk.moment(&mut tutoring) {
            ended = true;
            break;
        }
        let lesson: &'static Lesson = tutoring.lesson();
        let shown = match tutoring.shown(desk.now) {
            Shown::Setting(_) => None,
            Shown::Step { at, step, state, .. } => Some(Ok((at, step, state))),
            Shown::Between { done, next, .. } => Some(Err((done.id, next.map(|next| next.id)))),
        };
        check(desk, lesson.id);
        match shown {
            // The scene is being set.
            None => {
                waited += 1;
                assert!(waited < 200, "the scene of {} never came", lesson.id);
            }
            Some(Err((done, next))) => {
                waited = 0;
                acted = None;
                if finished.last() != Some(&done) {
                    finished.push(done);
                }
                // On to the next, or (after the last) out.
                let _ = next;
                if desk.press(&mut tutoring, Input::Next) {
                    ended = true;
                    break;
                }
            }
            Some(Ok((_, _, StepState::Done))) => waited = 0,
            Some(Ok((at, step, StepState::Waiting))) => {
                if acted == Some((lesson.id, at)) {
                    waited += 1;
                    let stands = desk.facts();
                    assert!(
                        waited < 50,
                        "{} step {} was done and is not seen done: \"{}\" ({:?}); refused: {:?}; the desktop: {:?}",
                        lesson.id,
                        at + 1,
                        step.say,
                        step.goal,
                        desk.refused,
                        stands,
                    );
                    continue;
                }
                acted = Some((lesson.id, at));
                waited = 0;
                let ended_here = match (step.goal, step.act) {
                    (Goal::Read, _) => {
                        if desk.hasty {
                            desk.surfaces.clear();
                        }
                        desk.press(&mut tutoring, Input::Next)
                    }
                    // The card's button is the way.
                    (_, Act::Practice) => desk.press(&mut tutoring, Input::ShowMe),
                    (_, Act::Command(_)) if step.by_button => {
                        desk.press(&mut tutoring, Input::ShowMe)
                    }
                    // The key the card shows runs the step's command.
                    (_, Act::Command(command)) => {
                        desk.command(command);
                        false
                    }
                    // Escape puts a surface of the shell away.
                    (Goal::Gone(surface), Act::None) => {
                        assert!(
                            desk.surfaces.remove(surface),
                            "{} step {} asks for {surface} to be put away, and it is not up",
                            lesson.id,
                            at + 1
                        );
                        false
                    }
                    // The settings program is opened from the launcher.
                    (Goal::WindowOpened, Act::None) => {
                        desk.wm.open("gevrek studio", "gevrek-studio");
                        false
                    }
                    (goal, Act::None) => panic!(
                        "{} step {}: nothing here knows how {goal:?} is done by hand",
                        lesson.id,
                        at + 1
                    ),
                };
                assert!(!ended_here);
            }
        }
    }
    assert!(ended, "the tutorial never ended; it finished {finished:?}");
    // Its end, as the shell tidies: the practice windows closed, its
    // workspace gone, what was shown before shown again.
    let own = tutoring.own_windows();
    let beside = tutoring.beside_there();
    let practice: Vec<WindowHandle> = desk
        .wm
        .desktop
        .windows
        .iter()
        .filter(|(_, window)| window.class == gevrek_tutorial::PRACTICE_APP)
        .map(|(id, _)| *id)
        .collect();
    for id in practice {
        desk.wm.desktop.close(id);
    }
    desk.wm.hear_of_closed();
    for command in tidy_commands(&before, &own, beside) {
        desk.command(&command.to_string());
    }
    finished
}

#[test]
fn the_tour_is_played_from_its_first_step_to_its_last() {
    let mut desk = Desk::new();
    // The person's own desktop: two windows on the first workspace, one
    // on the second, and the first shown.
    desk.command(r#"{"command":"switch_workspace_selector","selector":{"kind":"name","value":"2"}}"#);
    desk.wm.open("Mail", "mail");
    desk.command(r#"{"command":"switch_workspace_selector","selector":{"kind":"name","value":"1"}}"#);
    desk.wm.open("Editor", "editor");
    desk.wm.open("Browser", "browser");
    assert!(desk.refused.is_empty(), "{:?}", desk.refused);
    let own = ["mail", "editor", "browser"];
    let at_first = standing(&desk.wm.state(), &own);
    assert_eq!(at_first.len(), 3);
    let workspaces_at_first = names(&desk.wm.state());

    let file = memory("tour");
    let tour: Vec<&str> = gevrek_tutorial::lessons::the_tour()
        .iter()
        .map(|lesson| lesson.id)
        .collect();
    let finished = play(&mut desk, None, &file, |desk, lesson| {
        let state = desk.wm.state();
        // Nothing of the person's own is moved by the tutorial, at any
        // moment of it, and no practice window is ever among them.
        assert_eq!(
            standing(&state, &own),
            at_first,
            "the person's windows were moved during {lesson}"
        );
        let on = practice_on(&state);
        assert!(
            on.iter().all(|name| name.starts_with(gevrek_tutorial::WORKSPACE)),
            "a practice window is on {on:?} during {lesson}"
        );
    });
    assert_eq!(finished, tour, "every lesson of the tour, in its order");
    assert!(desk.refused.is_empty(), "refused: {:?}", desk.refused);

    // After it: the desktop as it was.
    let state = desk.wm.state();
    assert_eq!(standing(&state, &own), at_first);
    assert_eq!(names(&state), workspaces_at_first);
    assert_eq!(Facts::of(&state).workspace, "1");
    assert!(desk.surfaces.is_empty(), "{:?} is still up", desk.surfaces);
    let kept = gevrek_tutorial::Progress::load(&file);
    assert!(!kept.running);
    assert_eq!(kept.count(), (tour.len(), gevrek_tutorial::lessons().len()));
    let _ = std::fs::remove_dir_all(file.parent().unwrap());
}

#[test]
fn every_lesson_is_played_by_itself() {
    for lesson in gevrek_tutorial::lessons() {
        let mut desk = Desk::new();
        desk.wm.open("Editor", "editor");
        let file = memory(&format!("alone-{}", lesson.id));
        let at_first = standing(&desk.wm.state(), &["editor"]);
        let finished = play(&mut desk, Some(lesson.id), &file, |_, _| {});
        // (Taken by itself, a lesson of the tour goes on to the next: the
        // one asked for is the first that was finished.)
        assert_eq!(finished.first(), Some(&lesson.id));
        assert!(desk.refused.is_empty(), "{}: refused: {:?}", lesson.id, desk.refused);
        assert_eq!(standing(&desk.wm.state(), &["editor"]), at_first, "{}", lesson.id);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }
}

#[test]
fn a_surface_put_away_early_does_not_hold_a_lesson_up() {
    // The launcher is put away while the card still tells of it: the step
    // that asks for it to be put away is done already, and the lesson
    // goes on by itself.
    for lesson in ["launcher", "around"] {
        let mut desk = Desk::new();
        desk.hasty = true;
        desk.wm.open("Editor", "editor");
        let file = memory(&format!("hasty-{lesson}"));
        let finished = play(&mut desk, Some(lesson), &file, |_, _| {});
        assert_eq!(finished.first(), Some(&lesson));
        assert!(desk.refused.is_empty(), "{lesson}: refused: {:?}", desk.refused);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }
}
