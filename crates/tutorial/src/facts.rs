//! What a step can know of the desktop, and what it waits for.

use serde_json::Value;

/// A window of the workspace that has the keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub id: u64,
    /// Its place and size, in pixels.
    pub at: (i32, i32, i32, i32),
    pub focused: bool,
    pub floating: bool,
    pub maximized: bool,
    /// Its application (class or app id).
    pub app: String,
}

/// How one window stands, for it to say of itself: a practice window
/// shows it, so that what a step did is read on the window it was done
/// to (it has the keyboard now, it floats, it is one of three tabs).
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub keyboard: bool,
    pub floating: bool,
    pub maximized: bool,
    /// It is one of so many windows that share a place as tabs.
    pub tab_of: Option<usize>,
}

impl Standing {
    /// How the practice window with this title stands, by the window
    /// manager's state: looked for on every workspace of every display
    /// (a practice window may have been sent to another). `None` where
    /// the state has no such window.
    pub fn of(state: &Value, title: &str) -> Option<Standing> {
        let whole = |value: &Value| value.as_f64().unwrap_or(0.0).round() as i64;
        let place = |window: &Value| {
            (
                whole(&window["position"]["x"]),
                whole(&window["position"]["y"]),
                whole(&window["size"]["width"]),
                whole(&window["size"]["height"]),
            )
        };
        let is = |window: &Value, key: &str| window[key].as_bool().unwrap_or(false);
        for monitor in state["monitors"].as_array().into_iter().flatten() {
            let all = monitor["workspaces"].as_array().into_iter().flatten();
            for workspace in all.chain(std::iter::once(&monitor["workspace"])) {
                let windows: Vec<&Value> = workspace["detailed_windows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|window| !is(window, "is_border_window"))
                    .collect();
                let Some(own) = windows.iter().find(|window| {
                    window["title"].as_str() == Some(title)
                        && [&window["app_id"], &window["class"]].iter().any(|app| {
                            app.as_str()
                                .is_some_and(|app| app.eq_ignore_ascii_case(crate::PRACTICE_APP))
                        })
                }) else {
                    continue;
                };
                let floating = is(own, "is_floating");
                // Tabs share one place: the tiled windows that are where
                // this one is. (Only where the workspace has fewer places
                // than tiled windows: else two windows of one size are
                // two windows.)
                let tiled: Vec<&&Value> = windows
                    .iter()
                    .filter(|window| !is(window, "is_floating") && !is(window, "is_minimized"))
                    .collect();
                let places = workspace["container_count"].as_u64().unwrap_or(u64::MAX);
                let sharing = tiled
                    .iter()
                    .filter(|window| place(window) == place(own))
                    .count();
                let tab_of = (!floating && places < tiled.len() as u64 && sharing > 1)
                    .then_some(sharing);
                return Some(Standing {
                    keyboard: is(own, "is_focused"),
                    floating,
                    maximized: is(own, "is_maximized"),
                    tab_of,
                });
            }
        }
        None
    }

    /// It in words, as a window says it of itself.
    pub fn words(&self) -> String {
        let keyboard = if self.keyboard {
            "Has the keyboard"
        } else {
            "Does not have the keyboard"
        };
        let how = match (self.maximized, self.floating, self.tab_of) {
            (true, _, _) => "maximized".to_string(),
            (_, true, _) => "floating".to_string(),
            (_, _, Some(count)) => format!("one of {count} tabs"),
            _ => "tiled".to_string(),
        };
        format!("{keyboard}  ·  {how}")
    }
}

/// The desktop as far as a step looks at it: the workspace shown on the
/// display that has the keyboard, and the shell's surfaces that are up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    /// The display that has the keyboard, by its name (empty where the
    /// desktop does not say).
    pub display: String,
    /// The workspace's name.
    pub workspace: String,
    /// Its layout, as the window manager names it (`Dwindle`).
    pub layout: String,
    pub windows: Vec<Window>,
    /// How many places the tiled windows take: fewer than there are tiled
    /// windows where some are joined as tabs.
    pub containers: usize,
    /// How many displays the desktop has.
    pub displays: usize,
    /// The shell's surfaces that are shown (`gevrek-launcher`), told by
    /// the shell itself.
    pub surfaces: Vec<String>,
}

impl Facts {
    /// The facts in the window manager's state (`gevrek state --json`):
    /// of the display that has the keyboard, or the first.
    pub fn of(state: &Value) -> Facts {
        let monitors: Vec<&Value> = state["monitors"].as_array().into_iter().flatten().collect();
        let Some(monitor) = monitors
            .iter()
            .find(|monitor| monitor["is_focused"].as_bool() == Some(true))
            .or(monitors.first())
        else {
            return Facts::default();
        };
        let workspace = &monitor["workspace"];
        let number = |value: &Value| value.as_f64().unwrap_or(0.0).round() as i32;
        let windows = workspace["detailed_windows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|window| window["is_border_window"].as_bool() != Some(true))
            .filter_map(|window| {
                let app = window["app_id"]
                    .as_str()
                    .filter(|app| !app.is_empty())
                    .or(window["class"].as_str())
                    .unwrap_or_default();
                Some(Window {
                    id: window["window_id"].as_u64()?,
                    at: (
                        number(&window["position"]["x"]),
                        number(&window["position"]["y"]),
                        number(&window["size"]["width"]),
                        number(&window["size"]["height"]),
                    ),
                    focused: window["is_focused"].as_bool().unwrap_or(false),
                    floating: window["is_floating"].as_bool().unwrap_or(false),
                    maximized: window["is_maximized"].as_bool().unwrap_or(false),
                    app: app.to_string(),
                })
            })
            .collect();
        Facts {
            display: monitor["name"].as_str().unwrap_or_default().to_string(),
            workspace: workspace["name"].as_str().unwrap_or_default().to_string(),
            layout: workspace["layout_type"].as_str().unwrap_or_default().to_string(),
            windows,
            containers: workspace["container_count"].as_u64().unwrap_or(0) as usize,
            displays: monitors.len(),
            surfaces: Vec::new(),
        }
    }

    /// The window that has the keyboard.
    pub fn focused(&self) -> Option<&Window> {
        self.windows.iter().find(|window| window.focused)
    }

    fn window(&self, id: u64) -> Option<&Window> {
        self.windows.iter().find(|window| window.id == id)
    }

    /// Whether the workspace arranges its windows (any layout but the
    /// floating one).
    pub fn tiles(&self) -> bool {
        !self.layout.is_empty() && !self.layout.eq_ignore_ascii_case("floating")
    }

    fn tiled(&self) -> usize {
        self.windows.iter().filter(|window| !window.floating).count()
    }
}

/// What a step waits for. Each is said of the desktop as it is now
/// against how it was when the step began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// Nothing to do: the step is read, and gone on from by hand.
    Read,
    /// The workspace arranges its windows.
    Tiles,
    /// The workspace has another layout than it had.
    LayoutChanged,
    /// The workspace has this layout.
    Layout(&'static str),
    /// Another window has the keyboard.
    FocusChanged,
    /// The window that had the keyboard is at another place, on the same
    /// workspace.
    WindowMoved,
    /// The window that had the keyboard has another size.
    WindowResized,
    /// Another workspace is shown.
    WorkspaceChanged,
    /// Another workspace is shown, and the window that had the keyboard
    /// came along.
    WindowCarried,
    /// There are more windows than there were.
    WindowOpened,
    /// There are fewer windows than there were.
    WindowClosed,
    /// The window that has the keyboard floats, or (false) is tiled.
    Floats(bool),
    /// Some windows are joined as tabs, or (false) none is any more.
    Tabs(bool),
    /// The window that has the keyboard is maximised, or (false) not.
    Maximized(bool),
    /// A surface of the shell is shown, by its name.
    Shown(&'static str),
    /// A surface of the shell that was shown is gone.
    Gone(&'static str),
    /// A command of this name was carried out (for what leaves nothing to
    /// look at): the name as the window manager's events have it.
    Done(&'static str),
}

/// Whether a goal is met: `before` is the desktop when the step began,
/// `now` as it is, `carried_out` the commands' names the window manager
/// said it carried out since.
pub fn met(goal: Goal, before: &Facts, now: &Facts, carried_out: &[String]) -> bool {
    let had = before.focused();
    let has = now.focused();
    let same_place = before.workspace == now.workspace;
    match goal {
        Goal::Read => false,
        Goal::Tiles => now.tiles(),
        Goal::LayoutChanged => same_place && !now.layout.is_empty() && now.layout != before.layout,
        Goal::Layout(name) => now.layout.eq_ignore_ascii_case(name),
        Goal::FocusChanged => {
            same_place
                && matches!((had, has), (Some(had), Some(has)) if had.id != has.id)
        }
        Goal::WindowMoved => same_place
            && had.is_some_and(|had| {
                now.window(had.id)
                    .is_some_and(|window| (window.at.0, window.at.1) != (had.at.0, had.at.1))
            }),
        Goal::WindowResized => same_place
            && had.is_some_and(|had| {
                now.window(had.id)
                    .is_some_and(|window| (window.at.2, window.at.3) != (had.at.2, had.at.3))
            }),
        Goal::WorkspaceChanged => !now.workspace.is_empty() && !same_place,
        Goal::WindowCarried => {
            !now.workspace.is_empty()
                && !same_place
                && had.is_some_and(|had| now.window(had.id).is_some())
        }
        Goal::WindowOpened => same_place && now.windows.len() > before.windows.len(),
        Goal::WindowClosed => same_place && now.windows.len() < before.windows.len(),
        Goal::Floats(wanted) => has.is_some_and(|window| window.floating == wanted),
        Goal::Tabs(wanted) => (now.containers < now.tiled() && now.tiled() > 1) == wanted,
        Goal::Maximized(wanted) => has.is_some_and(|window| window.maximized == wanted),
        Goal::Shown(name) => now.surfaces.iter().any(|shown| shown == name),
        Goal::Gone(name) => {
            before.surfaces.iter().any(|shown| shown == name)
                && !now.surfaces.iter().any(|shown| shown == name)
        }
        Goal::Done(name) => carried_out.iter().any(|done| done == name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn window(id: u64, at: (i32, i32, i32, i32), focused: bool) -> Window {
        Window {
            id,
            at,
            focused,
            floating: false,
            maximized: false,
            app: "gevrek-practice".to_string(),
        }
    }

    fn desk() -> Facts {
        Facts {
            display: String::new(),
            workspace: "Tutorial".to_string(),
            layout: "Dwindle".to_string(),
            windows: vec![
                window(1, (0, 0, 800, 900), true),
                window(2, (800, 0, 800, 450), false),
                window(3, (800, 450, 800, 450), false),
            ],
            containers: 3,
            displays: 1,
            surfaces: Vec::new(),
        }
    }

    #[test]
    fn a_practice_window_is_told_how_it_stands() {
        let window = |title: &str, at: i32, focused: bool, floating: bool| {
            json!({"window_id": at, "title": title, "app_id": "gevrek-practice", "class": "",
                   "position": {"x": at, "y": 0}, "size": {"width": 600, "height": 400},
                   "is_focused": focused, "is_floating": floating, "is_maximized": false})
        };
        // Three places: two tiles beside each other and a floating one.
        let state = json!({"monitors": [{"is_focused": true,
            "workspace": {"name": "Tutorial", "container_count": 2, "detailed_windows": []},
            "workspaces": [{"name": "Tutorial", "container_count": 2, "detailed_windows": [
                window("Notes", 0, true, false),
                window("Browser", 600, false, false),
                window("Music", 100, false, true),
            ]}]}]});
        let notes = Standing::of(&state, "Notes").unwrap();
        assert_eq!(
            notes,
            Standing { keyboard: true, floating: false, maximized: false, tab_of: None }
        );
        assert_eq!(notes.words(), "Has the keyboard  ·  tiled");
        assert_eq!(
            Standing::of(&state, "Music").unwrap().words(),
            "Does not have the keyboard  ·  floating"
        );
        assert_eq!(Standing::of(&state, "Mail"), None);

        // Two of them joined as tabs: one place for both, and each says so.
        let tabs = json!({"monitors": [{"is_focused": true,
            "workspace": {"name": "Tutorial", "container_count": 1, "detailed_windows": [
                window("Notes", 0, false, false),
                window("Browser", 0, true, false),
            ]}}]});
        assert_eq!(
            Standing::of(&tabs, "Browser").unwrap().words(),
            "Has the keyboard  ·  one of 2 tabs"
        );
        assert_eq!(Standing::of(&tabs, "Notes").unwrap().tab_of, Some(2));
        // A window of the person's own with the same title is not it.
        let other = json!({"monitors": [{"workspace": {"detailed_windows": [
            {"title": "Notes", "app_id": "org.gnome.Notes", "class": "notes"}]}}]});
        assert_eq!(Standing::of(&other, "Notes"), None);
    }

    #[test]
    fn the_facts_are_read_from_the_window_managers_state() {
        let state = json!({"monitors": [
            {"is_focused": false, "workspace": {"name": "1", "layout_type": "Dwindle", "detailed_windows": []}},
            {"is_focused": true, "workspace": {
                "name": "Tutorial", "layout_type": "Floating", "container_count": 2,
                "detailed_windows": [
                    {"window_id": 7, "position": {"x": 14.0, "y": 65.4}, "size": {"width": 800, "height": 600},
                     "is_focused": true, "is_floating": true, "is_maximized": false, "app_id": "gevrek-practice", "class": "x"},
                    {"window_id": 8, "position": {"x": 0, "y": 0}, "size": {"width": 1, "height": 1},
                     "is_border_window": true},
                    {"window_id": 9, "position": {"x": 900.0, "y": 65.0}, "size": {"width": 700, "height": 600},
                     "is_focused": false, "is_floating": false, "class": "Notes", "app_id": ""},
                ]}},
        ]});
        let facts = Facts::of(&state);
        assert_eq!((facts.workspace.as_str(), facts.layout.as_str()), ("Tutorial", "Floating"));
        assert_eq!((facts.displays, facts.containers, facts.windows.len()), (2, 2, 2));
        assert_eq!(facts.focused().map(|window| window.id), Some(7));
        assert_eq!(facts.windows[0].at, (14, 65, 800, 600));
        assert_eq!(facts.windows[1].app, "Notes");
        assert!(!facts.tiles());
        // A state without displays: nothing, and nothing breaks.
        assert_eq!(Facts::of(&json!({})), Facts::default());
    }

    #[test]
    fn a_goal_is_about_what_came_of_it() {
        let before = desk();
        let none: [String; 0] = [];
        let check = |goal, now: &Facts| met(goal, &before, now, &none);
        // Nothing has happened: nothing is met but what already holds.
        for goal in [
            Goal::Read,
            Goal::LayoutChanged,
            Goal::FocusChanged,
            Goal::WindowMoved,
            Goal::WindowResized,
            Goal::WorkspaceChanged,
            Goal::WindowCarried,
            Goal::WindowOpened,
            Goal::WindowClosed,
            Goal::Floats(true),
            Goal::Tabs(true),
            Goal::Shown("gevrek-launcher"),
        ] {
            assert!(!check(goal, &before), "{goal:?}");
        }
        assert!(check(Goal::Tiles, &before) && check(Goal::Layout("dwindle"), &before));

        // The keyboard goes to the window beside it.
        let mut now = desk();
        now.windows[0].focused = false;
        now.windows[1].focused = true;
        assert!(check(Goal::FocusChanged, &now) && !check(Goal::WindowMoved, &now));

        // The window that had the keyboard changes places with another.
        let mut now = desk();
        now.windows[0].at = (800, 0, 800, 450);
        now.windows[1].at = (0, 0, 800, 900);
        assert!(check(Goal::WindowMoved, &now) && check(Goal::WindowResized, &now));
        assert!(!check(Goal::FocusChanged, &now));

        // Another workspace: alone, and with the window carried along.
        let mut now = desk();
        now.workspace = "2".to_string();
        now.windows.clear();
        assert!(check(Goal::WorkspaceChanged, &now) && !check(Goal::WindowCarried, &now));
        now.windows.push(window(1, (0, 0, 1600, 900), true));
        assert!(check(Goal::WindowCarried, &now));
        // (Fewer windows there is no window closed here.)
        assert!(!check(Goal::WindowClosed, &now));

        // One window fewer, one more.
        let mut now = desk();
        now.windows.pop();
        assert!(check(Goal::WindowClosed, &now));
        now.windows.push(window(3, (0, 0, 1, 1), false));
        now.windows.push(window(4, (0, 0, 1, 1), false));
        assert!(check(Goal::WindowOpened, &now));

        // Floating, tabs, the layout, a surface of the shell, a command.
        let mut now = desk();
        now.windows[0].floating = true;
        assert!(check(Goal::Floats(true), &now) && !check(Goal::Floats(false), &now));
        let mut now = desk();
        now.containers = 2;
        assert!(check(Goal::Tabs(true), &now) && check(Goal::Tabs(false), &before));
        let mut now = desk();
        now.layout = "Monocle".to_string();
        assert!(check(Goal::LayoutChanged, &now) && check(Goal::Layout("monocle"), &now));
        now.layout = "Floating".to_string();
        assert!(!check(Goal::Tiles, &now));
        let mut now = desk();
        now.surfaces.push("gevrek-launcher".to_string());
        assert!(check(Goal::Shown("gevrek-launcher"), &now));
        assert!(met(Goal::Gone("gevrek-launcher"), &now, &before, &none));
        assert!(met(Goal::Done("toggle_expose"), &before, &before, &["toggle_expose".to_string()]));
        assert!(!met(Goal::Done("toggle_expose"), &before, &before, &["quit".to_string()]));
    }
}
