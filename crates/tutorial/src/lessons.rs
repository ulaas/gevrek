//! The lessons, as data.
//!
//! A lesson sets a scene and has steps; a step says one thing, names the
//! command that does it (the keys shown are the person's own for that
//! command), and what it waits for. What a lesson says is written here
//! and nowhere else: the card that shows it, the list of lessons and
//! what is new after an update all read it from here.

use crate::facts::Goal;
use std::sync::OnceLock;

/// Where a lesson belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chapter {
    /// The tour a person is offered first: what everybody needs.
    Tour,
    /// A chapter of its own, taken when it is wanted.
    Chapter,
}

/// What a lesson needs of the machine to be done by hand; without it the
/// lesson is only shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// So many displays.
    Displays(usize),
    Touchpad,
    /// A system: `windows` or `linux`.
    System(&'static str),
}

/// What a lesson begins with, on the tutorial's workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scene {
    /// So many practice windows.
    pub windows: usize,
    /// The layout, by its name in a configuration (`dwindle`).
    pub layout: &'static str,
    /// A second workspace of the tutorial's own after the first
    /// (`WORKSPACE_BESIDE`): the lesson goes to the next workspace.
    pub beside: bool,
}

/// What the ring is drawn around while a step waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    None,
    /// The window that has the keyboard.
    Focused,
    /// Every window of the workspace.
    Windows,
    /// The panel.
    Panel,
    /// A surface of the shell, by its name.
    Surface(&'static str),
}

/// The small drawing that shows what a step does before it is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Demo {
    None,
    /// Windows that float fall into their places.
    Tile,
    /// A window goes; the others take its room.
    Close,
    /// A window comes; the others make room.
    Open,
    /// The keyboard goes from one window to the next.
    Focus,
    /// Two windows change places.
    Swap,
    /// One workspace goes out, another comes in.
    Workspace,
    /// A window goes to another workspace, and the view with it.
    Carry,
    /// Every window small, side by side.
    Overview,
    /// The launcher comes up.
    Launcher,
    /// A window grows and its neighbour shrinks.
    Resize,
    /// One window takes all the room.
    Monocle,
    /// A window lifts off the others.
    Float,
}

/// What a step asks to be done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Nothing: the step is read.
    None,
    /// A command, as the JSON a binding holds for it.
    Command(&'static str),
    /// One more practice window.
    Practice,
}

/// One thing to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// What to do, in one sentence.
    pub say: &'static str,
    /// What it is good for, or what to look at: a sentence more.
    pub why: &'static str,
    pub act: Act,
    /// The command has no key in gevrek's own bindings: the card's
    /// button does it (a person who has bound a key sees that key).
    pub by_button: bool,
    pub goal: Goal,
    pub spot: Spot,
    pub demo: Demo,
}

/// A lesson.
#[derive(Debug, Clone, PartialEq)]
pub struct Lesson {
    /// Its name for programs (`focus`): what the shell is told to begin
    /// with it, and what the tutorial's memory keeps it by.
    pub id: &'static str,
    pub title: &'static str,
    /// What a person can do after it.
    pub summary: &'static str,
    pub chapter: Chapter,
    /// About so many minutes.
    pub minutes: u32,
    /// The version it came with: what is new after an update.
    pub since: &'static str,
    /// Counted up when the lesson is changed so much that someone who
    /// finished it has not seen it.
    pub revision: u32,
    pub needs: Vec<Need>,
    pub scene: Scene,
    pub steps: Vec<Step>,
}

fn read(say: &'static str, why: &'static str, spot: Spot) -> Step {
    Step {
        say,
        why,
        act: Act::None,
        by_button: false,
        goal: Goal::Read,
        spot,
        demo: Demo::None,
    }
}

fn step(
    say: &'static str,
    why: &'static str,
    command: &'static str,
    goal: Goal,
    spot: Spot,
    demo: Demo,
) -> Step {
    Step {
        say,
        why,
        act: Act::Command(command),
        by_button: false,
        goal,
        spot,
        demo,
    }
}

const FOCUS_LEFT: &str = r#"{"command":"focus_direction","direction":"left"}"#;
const FOCUS_RIGHT: &str = r#"{"command":"focus_direction","direction":"right"}"#;
const FOCUS_DOWN: &str = r#"{"command":"focus_direction","direction":"down"}"#;
const SWAP_LEFT: &str = r#"{"command":"swap_direction","direction":"left"}"#;
const SWAP_RIGHT: &str = r#"{"command":"swap_direction","direction":"right"}"#;
const NEXT_WORKSPACE: &str =
    r#"{"command":"switch_workspace_selector","selector":{"kind":"next"}}"#;
const PREVIOUS_WORKSPACE: &str =
    r#"{"command":"switch_workspace_selector","selector":{"kind":"previous"}}"#;
const CARRY_NEXT: &str = r#"{"command":"move_to_workspace_and_follow","selector":{"kind":"next"}}"#;
const CARRY_PREVIOUS: &str =
    r#"{"command":"move_to_workspace_and_follow","selector":{"kind":"previous"}}"#;
const OVERVIEW: &str = r#"{"command":"toggle_expose"}"#;
const LAUNCHER: &str = r#"{"command":"shell","action":"launcher"}"#;
const CONTROL_CENTRE: &str = r#"{"command":"shell","action":"control_center"}"#;
const SHORTCUTS: &str = r#"{"command":"show_shortcuts"}"#;
const CLOSE: &str = r#"{"command":"close_active"}"#;
const WIDER: &str = r#"{"command":"resize_axis","axis":"horizontal","sizing":"increase"}"#;
const MONOCLE: &str = r#"{"command":"toggle_monocle"}"#;
const FLOAT: &str = r#"{"command":"toggle_floating"}"#;
const TILE: &str = r#"{"command":"set_layout","layout":"Dwindle"}"#;

fn tour() -> Vec<Lesson> {
    let lesson = |id, title, summary, minutes, scene, steps| Lesson {
        id,
        title,
        summary,
        chapter: Chapter::Tour,
        minutes,
        since: "0.7.0",
        revision: 1,
        needs: Vec::new(),
        scene,
        steps,
    };
    let tiled = Scene {
        windows: 3,
        layout: "dwindle",
        beside: false,
    };
    vec![
        lesson(
            "tiling",
            "Windows arrange themselves",
            "Know what gevrek does with windows, and why nothing overlaps.",
            2,
            Scene {
                windows: 3,
                layout: "floating",
                beside: false,
            },
            vec![
                read(
                    "These are practice windows, on a workspace of the tutorial's own.",
                    "Your own windows are where you left them, and stay there.",
                    Spot::Windows,
                ),
                Step {
                    by_button: true,
                    ..step(
                        "They float, as on any desktop: you would place each one by hand. Let gevrek place them.",
                        "Every window gets a share of the display. None lies over another.",
                        TILE,
                        Goal::Tiles,
                        Spot::Windows,
                        Demo::Tile,
                    )
                },
                step(
                    "Close the window that has the keyboard.",
                    "The others take its room at once: there is never a hole to fill.",
                    CLOSE,
                    Goal::WindowClosed,
                    Spot::Focused,
                    Demo::Close,
                ),
                Step {
                    act: Act::Practice,
                    by_button: true,
                    ..step(
                        "Open another window.",
                        "It gets a place, and the others make room for it.",
                        "",
                        Goal::WindowOpened,
                        Spot::Windows,
                        Demo::Open,
                    )
                },
                read(
                    "The coloured border is around the window that has the keyboard.",
                    "What you type goes there. The next lesson moves it.",
                    Spot::Focused,
                ),
            ],
        ),
        lesson(
            "focus",
            "Going from window to window",
            "Give any window the keyboard without reaching for the pointer.",
            1,
            tiled,
            vec![
                step(
                    "Go to the window on the right.",
                    "The border goes with the keyboard.",
                    FOCUS_RIGHT,
                    Goal::FocusChanged,
                    Spot::Focused,
                    Demo::Focus,
                ),
                step(
                    "Now to the one below.",
                    "Left, right, up and down: the window that lies that way.",
                    FOCUS_DOWN,
                    Goal::FocusChanged,
                    Spot::Focused,
                    Demo::Focus,
                ),
                step(
                    "And back to the left.",
                    "A press with the pointer on a window does the same. The keys are quicker once the hands know them.",
                    FOCUS_LEFT,
                    Goal::FocusChanged,
                    Spot::Focused,
                    Demo::Focus,
                ),
            ],
        ),
        lesson(
            "move",
            "Moving a window",
            "Put a window where you want it among the others.",
            1,
            tiled,
            vec![
                step(
                    "Move this window to the right.",
                    "It changes places with the window that was there.",
                    SWAP_RIGHT,
                    Goal::WindowMoved,
                    Spot::Focused,
                    Demo::Swap,
                ),
                step(
                    "And back to the left.",
                    "The keys to move are the keys to go, with one more held.",
                    SWAP_LEFT,
                    Goal::WindowMoved,
                    Spot::Focused,
                    Demo::Swap,
                ),
            ],
        ),
        lesson(
            "workspaces",
            "Workspaces",
            "Keep things apart on workspaces, and take a window along.",
            2,
            Scene {
                beside: true,
                ..tiled
            },
            vec![
                step(
                    "Show the next workspace.",
                    "A workspace is a desktop of its own. This one is empty; the practice windows wait on the one before.",
                    NEXT_WORKSPACE,
                    Goal::WorkspaceChanged,
                    Spot::Panel,
                    Demo::Workspace,
                ),
                step(
                    "Go back to the one before.",
                    "The panel shows the workspaces of this display; a press on one shows it.",
                    PREVIOUS_WORKSPACE,
                    Goal::WorkspaceChanged,
                    Spot::Panel,
                    Demo::Workspace,
                ),
                step(
                    "Take the window that has the keyboard to the next workspace.",
                    "The window goes, and the view goes with it.",
                    CARRY_NEXT,
                    Goal::WindowCarried,
                    Spot::Focused,
                    Demo::Carry,
                ),
                step(
                    "Bring it back.",
                    "Workspaces with a number have keys of their own: the number to show one, and one more key held to take a window there.",
                    CARRY_PREVIOUS,
                    Goal::WindowCarried,
                    Spot::Focused,
                    Demo::Carry,
                ),
                step(
                    "Look at every window of this workspace at once.",
                    "A press on a window there gives it the keyboard. The same key, or Escape, puts the overview away.",
                    OVERVIEW,
                    Goal::Done("toggle_expose"),
                    Spot::Windows,
                    Demo::Overview,
                ),
            ],
        ),
        lesson(
            "launcher",
            "Starting programs",
            "Start any program with a few letters.",
            1,
            tiled,
            vec![
                step(
                    "Open the launcher.",
                    "It lists the programs of this machine, the ones you start most first.",
                    LAUNCHER,
                    Goal::Shown("gevrek-launcher"),
                    Spot::Surface("gevrek-launcher"),
                    Demo::Launcher,
                ),
                read(
                    "Type a few letters of a program's name; Enter starts the one that is marked.",
                    "After a > it finds everything gevrek itself can do, after a : what you copied lately. Tab goes to the row of buttons under the field.",
                    Spot::Surface("gevrek-launcher"),
                ),
                Step {
                    act: Act::None,
                    ..step(
                        "Put it away with Escape.",
                        "Nothing is started.",
                        "",
                        Goal::Gone("gevrek-launcher"),
                        Spot::Surface("gevrek-launcher"),
                        Demo::None,
                    )
                },
            ],
        ),
        lesson(
            "shape",
            "Giving a window more room",
            "Make a window wider, give it the whole display, or lift it off the others.",
            2,
            tiled,
            vec![
                step(
                    "Make this window wider.",
                    "Its neighbour gives the room: the windows still fill the display.",
                    WIDER,
                    Goal::WindowResized,
                    Spot::Focused,
                    Demo::Resize,
                ),
                step(
                    "Give it all the room.",
                    "The other windows are behind it, where they were.",
                    MONOCLE,
                    Goal::Layout("monocle"),
                    Spot::Focused,
                    Demo::Monocle,
                ),
                step(
                    "And back among the others.",
                    "The same key again.",
                    MONOCLE,
                    Goal::LayoutChanged,
                    Spot::Windows,
                    Demo::Monocle,
                ),
                step(
                    "Let this window float.",
                    "A floating window lies over the others and keeps the size you give it: for a dialog, a calculator, a film in a corner.",
                    FLOAT,
                    Goal::Floats(true),
                    Spot::Focused,
                    Demo::Float,
                ),
                step(
                    "Put it back among the tiled ones.",
                    "The same key again.",
                    FLOAT,
                    Goal::Floats(false),
                    Spot::Focused,
                    Demo::Float,
                ),
            ],
        ),
        lesson(
            "studio",
            "The settings: gevrek studio",
            "Know where everything is changed, and how to open it: it does not open by itself.",
            2,
            tiled,
            vec![
                read(
                    "Everything about gevrek is set in one program: gevrek studio.",
                    "Keys, layouts, the look, the panel, rules for windows, displays, the wallpaper: each has its page there.",
                    Spot::None,
                ),
                read(
                    "It does not start with gevrek. You open it when you want to change something, and close it after.",
                    "gevrek runs without it: what you set is kept in your configuration.",
                    Spot::None,
                ),
                Step {
                    act: Act::None,
                    ..step(
                        "Open it: the launcher, then Tab to the row of buttons under the field. gevrek studio is the first of the three at its right.",
                        "In a terminal, `gevrek start editor` opens it too.",
                        "",
                        Goal::WindowOpened,
                        Spot::Surface("gevrek-launcher"),
                        Demo::Open,
                    )
                },
                read(
                    "This is gevrek studio. Its pages are on the left; the first, Overview, has this tutorial too.",
                    "A change is tried first, and kept only when you apply it. Undo takes it back.",
                    Spot::Focused,
                ),
                step(
                    "Close it again, as you close any window.",
                    "gevrek goes on as before. The studio is there the next time you want it.",
                    CLOSE,
                    Goal::WindowClosed,
                    Spot::Focused,
                    Demo::Close,
                ),
            ],
        ),
        lesson(
            "around",
            "Where everything is",
            "Find the panel, the control centre, the list of keys, and this tutorial again.",
            2,
            tiled,
            vec![
                read(
                    "The panel shows the workspaces, the windows that are open, and how the machine stands.",
                    "Every part of it can be pressed.",
                    Spot::Panel,
                ),
                step(
                    "Open the control centre.",
                    "Sound, networks, bluetooth, the night light: each shows what is in use, and opens its list when pressed.",
                    CONTROL_CENTRE,
                    Goal::Shown("gevrek-control-centre"),
                    Spot::Surface("gevrek-control-centre"),
                    Demo::None,
                ),
                Step {
                    act: Act::None,
                    ..step(
                        "Put it away with Escape.",
                        "",
                        "",
                        Goal::Gone("gevrek-control-centre"),
                        Spot::Surface("gevrek-control-centre"),
                        Demo::None,
                    )
                },
                step(
                    "Open the list of every key.",
                    "It is the list of your own configuration: change a key in the settings, and the list says the new one.",
                    SHORTCUTS,
                    Goal::Done("show_shortcuts"),
                    Spot::None,
                    Demo::None,
                ),
                read(
                    "That was the tour. Every lesson of it can be taken again, whenever you want.",
                    "The launcher finds the tutorial after a >, and so does `gevrek tutorial` in a terminal. gevrek studio's first page lists the lessons.",
                    Spot::None,
                ),
            ],
        ),
    ]
}

/// Every lesson, the tour first, in the order they are offered.
pub fn lessons() -> &'static [Lesson] {
    static ALL: OnceLock<Vec<Lesson>> = OnceLock::new();
    ALL.get_or_init(tour)
}

/// The lesson of this name.
pub fn lesson(id: &str) -> Option<&'static Lesson> {
    lessons().iter().find(|lesson| lesson.id == id)
}

/// The lessons of the tour, in their order.
pub fn the_tour() -> Vec<&'static Lesson> {
    lessons()
        .iter()
        .filter(|lesson| lesson.chapter == Chapter::Tour)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::command::DispatchCommand;

    #[test]
    fn every_lesson_can_be_taken() {
        let all = lessons();
        assert!(the_tour().len() >= 7);
        // What the welcome says of the tour's length holds ("about a
        // quarter of an hour").
        let minutes: u32 = the_tour().iter().map(|lesson| lesson.minutes).sum();
        assert!(
            (10..=17).contains(&minutes),
            "the tour takes {minutes} minutes"
        );
        // Nothing promises what is not there: only the tour is written.
        for lesson in all {
            for step in &lesson.steps {
                assert!(
                    !format!("{} {}", step.say, step.why)
                        .to_lowercase()
                        .contains("chapter"),
                    "{} speaks of chapters, and there are none",
                    lesson.id
                );
            }
        }
        let mut ids: Vec<&str> = all.iter().map(|lesson| lesson.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "a lesson's name is its own");
        let config = gevrek_model::examples::starting_configuration();
        for lesson in all {
            assert!(!lesson.steps.is_empty(), "{}", lesson.id);
            assert!(lesson.minutes >= 1 && !lesson.title.is_empty() && !lesson.summary.is_empty());
            assert!(
                gevrek_model::layout::LayoutType::all()
                    .iter()
                    .any(|layout| layout.config_name() == lesson.scene.layout),
                "{}: the scene's layout {}",
                lesson.id,
                lesson.scene.layout
            );
            for (index, step) in lesson.steps.iter().enumerate() {
                let at = format!("{} step {}", lesson.id, index + 1);
                assert!(!step.say.is_empty(), "{at}");
                // A step that is only read waits for nothing, and one
                // that waits has something to wait for.
                assert_eq!(
                    step.goal == Goal::Read,
                    step.act == Act::None
                        && step.demo == Demo::None
                        && !matches!(step.goal, Goal::Gone(_)),
                    "{at}"
                );
                let Act::Command(command) = step.act else {
                    continue;
                };
                // A command the window manager knows.
                let read: DispatchCommand = serde_json::from_str(command)
                    .unwrap_or_else(|error| panic!("{at}: {command}: {error}"));
                assert!(
                    !matches!(read, DispatchCommand::Unknown(_)),
                    "{at}: {command}"
                );
                // With a key in what gevrek starts with, unless the
                // card's button is how it is done.
                let keys = crate::keys_for(&config, command);
                assert_eq!(keys.is_empty(), step.by_button, "{at}: keys {keys:?}");
            }
        }
    }
}
