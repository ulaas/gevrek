//! The launcher: type a few letters, press Enter, the program starts.
//!
//! It finds the installed applications (`gevrek_system::apps`) and, after
//! a `>`, everything gevrek itself can be told to do (the commands of the
//! model, from its schema). What is chosen is handed to the window
//! manager to carry out, so a program started here is started the same
//! way as one started by a key. (On Windows an application is started as
//! the Start menu starts it, by the shell's half for that system; the
//! rest is the same.)
//!
//! What the launcher lists and how it is drawn ([`draw`]) is the same on
//! every system; each system's half of the shell opens the surface it is
//! drawn on, gives it the icons, and carries out what was chosen.

use gevrek_system::apps::App;
use gevrek_ui::theme;
use serde_json::Value;

/// One line of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub title: String,
    pub detail: String,
    /// The command the window manager is given when it is chosen.
    pub command: Value,
    /// The application it starts, to count how often it is started.
    pub app: Option<String>,
    /// The name (or file) of its icon; empty: none.
    pub icon: String,
    /// A text to put on the clipboard when it is chosen (instead of a
    /// command for the window manager).
    pub copy: Option<String>,
    /// The application has other things it can be started to do (the
    /// right arrow lists them).
    pub more: bool,
    /// The application is started by a message on the session's bus
    /// (`DBusActivatable`); the `command` is for where that fails.
    pub activate: Option<Activation>,
}

/// An application to start over the session's bus, or one of the things
/// it can be started to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    /// The application's name on the bus (the name of its entry).
    pub app: String,
    /// One of its actions; none: the application itself.
    pub action: Option<String>,
}

/// A window that is open, to switch to.
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub id: u64,
    pub title: String,
    pub class: String,
}

/// How often each application was started from the launcher, by its id
/// (what is started often comes first in the list), and which were
/// started last, the newest first (they stand in a row of their own).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    counts: std::collections::HashMap<String, u32>,
    recent: Vec<String>,
}

/// How many applications are remembered as started lately.
pub const REMEMBERED: usize = 12;

/// The file beside the counts' that keeps what was started last.
fn recent_file(path: &std::path::Path) -> std::path::PathBuf {
    path.with_file_name("launcher-recent.json")
}

impl Usage {
    pub fn load(path: &std::path::Path) -> Self {
        let read = |file: &std::path::Path| std::fs::read_to_string(file).ok();
        Self {
            counts: read(path)
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default(),
            recent: read(&recent_file(path))
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default(),
        }
    }

    pub fn count(&self, app: &str) -> u32 {
        self.counts.get(app).copied().unwrap_or(0)
    }

    /// The applications started last, the newest first.
    pub fn recent(&self) -> &[String] {
        &self.recent
    }

    /// One more start of `app`: counted, and the newest of those started
    /// last.
    pub fn started(&mut self, app: &str) {
        // (A count as large as numbers get, as a file from outside may
        // say, stays what it is.)
        let count = self.counts.entry(app.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        self.recent.retain(|known| known != app);
        self.recent.insert(0, app.to_string());
        self.recent.truncate(REMEMBERED);
    }

    /// One more start of `app`, written down at once.
    pub fn bump(&mut self, app: &str, path: &std::path::Path) {
        self.started(app);
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        if let Ok(text) = serde_json::to_string(&self.counts) {
            let _ = std::fs::write(path, text);
        }
        if let Ok(text) = serde_json::to_string(&self.recent) {
            let _ = std::fs::write(recent_file(path), text);
        }
    }
}

pub struct Launcher {
    pub typed: String,
    pub chosen: usize,
    apps: Vec<App>,
    windows: Vec<Open>,
    usage: Usage,
    /// What was copied lately, the newest first.
    copied: Vec<String>,
    commands: Vec<Entry>,
    /// The terminal applications that ask for one are started in.
    terminal: String,
    /// The application whose actions are listed in place of everything
    /// else (the right arrow on it; the left arrow or Escape goes back).
    actions_of: Option<String>,
    /// The button of the row under the field that Tab went to (the
    /// applications started last, then the ones that are always there);
    /// none: the keys are the list's.
    pub on_row: Option<usize>,
}

/// How many lines are shown.
pub const SHOWN: usize = 8;

/// How many applications started last stand in the row.
pub const RECENT: usize = 6;

/// How high the row under the field is, with the room around it.
const ROW: u32 = 52;

/// The size of the launcher's surface, in logical pixels: the field, the
/// row under it, and room for every line.
pub const SIZE: (u32, u32) = (640, 84 + ROW + SHOWN as u32 * 50);

/// Where Tab goes from a place in the row of `count` buttons (none: the
/// field and its list): to the next button, and after the last back to
/// the list; Shift+Tab the other way round.
pub fn tabbed(now: Option<usize>, count: usize, back: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match (now, back) {
        (None, false) => Some(0),
        (None, true) => Some(count - 1),
        (Some(at), false) => (at + 1 < count).then_some(at + 1),
        (Some(at), true) => at.checked_sub(1),
    }
}

/// What is always in the row, at its right: gevrek's own settings, the
/// control centre and the notifications, each with its sign.
pub fn always_there() -> Vec<(&'static str, Entry)> {
    use gevrek_ui::theme::icons;
    let own = |title: &str, detail: &str, command: Value| Entry {
        title: title.to_string(),
        detail: detail.to_string(),
        command,
        app: None,
        icon: String::new(),
        copy: None,
        more: false,
        activate: None,
    };
    // (The settings program beside this one, by its whole name: it need
    // not be on the path.)
    let studio = std::env::current_exe()
        .ok()
        .and_then(|own| {
            Some(
                own.parent()?
                    .join(format!("gevrek-studio{}", std::env::consts::EXE_SUFFIX)),
            )
        })
        .filter(|file| file.is_file())
        .map_or_else(
            || "gevrek-studio".to_string(),
            |file| file.display().to_string(),
        );
    let shell = |action: &str| serde_json::json!({"command": "shell", "action": action});
    vec![
        (
            icons::GEAR,
            own(
                "gevrek studio",
                "The settings of gevrek",
                serde_json::json!({"command": "exec", "program": studio, "args": []}),
            ),
        ),
        (
            icons::FADERS,
            own(
                "Control centre",
                "Sound, networks, the night light",
                shell("control_center"),
            ),
        ),
        (
            icons::BELL,
            own(
                "Notifications",
                "What was told lately",
                shell("notification_center"),
            ),
        ),
    ]
}

/// A text on one line, for a list.
pub fn one_line(text: &str, letters: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > letters {
        format!(
            "{}…",
            flat.chars()
                .take(letters.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        flat
    }
}

/// Where it is kept how often what was started from the launcher.
pub fn usage_file() -> std::path::PathBuf {
    gevrek_model::paths::state_dir().join("launcher-usage.json")
}

/// The windows that are open, as the window manager's state lists them:
/// to switch to by typing.
pub fn open_windows(wm: &Value) -> Vec<Open> {
    wm["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|monitor| {
            monitor["workspaces"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .flat_map(|workspace| {
            workspace["detailed_windows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|window| {
            Some(Open {
                id: window["window_id"].as_u64()?,
                title: window["title"].as_str()?.to_string(),
                class: window["class"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

impl Launcher {
    pub fn new(apps: Vec<App>, terminal: &str) -> Self {
        Self {
            typed: String::new(),
            chosen: 0,
            apps,
            windows: Vec::new(),
            usage: Usage::default(),
            copied: Vec::new(),
            commands: commands(),
            terminal: terminal.to_string(),
            actions_of: None,
            on_row: None,
        }
    }

    /// The line of an application.
    fn entry_of(&self, app: &App) -> Entry {
        Entry {
            title: app.name.clone(),
            detail: if app.generic_name.is_empty() {
                app.comment.clone()
            } else {
                app.generic_name.clone()
            },
            command: start_command(app, &self.terminal),
            app: Some(app.id.clone()),
            icon: app.icon.clone(),
            copy: None,
            more: !app.actions.is_empty(),
            activate: app.dbus_activatable.then(|| Activation {
                app: app.id.clone(),
                action: None,
            }),
        }
    }

    /// The applications started last that are still installed, the
    /// newest first: the row under the field.
    pub fn recent(&self) -> Vec<Entry> {
        self.usage
            .recent()
            .iter()
            .filter_map(|id| self.apps.iter().find(|app| app.id == *id))
            .take(RECENT)
            .map(|app| self.entry_of(app))
            .collect()
    }

    /// With the windows that are open and how often what was started.
    pub fn with(mut self, windows: Vec<Open>, usage: Usage) -> Self {
        self.windows = windows;
        self.usage = usage;
        self
    }

    /// The application of this id, among those the launcher lists.
    // For the half of the shell that starts applications itself: Windows'.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|app| app.id == id)
    }

    /// Other applications to list (they were read again).
    // Where reading them takes long and is done beside the drawing: Windows.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn set_apps(&mut self, apps: Vec<App>) {
        self.apps = apps;
    }

    /// With what was copied lately (listed after a `:`).
    pub fn with_copied(mut self, copied: Vec<String>) -> Self {
        self.copied = copied;
        self
    }

    /// List the actions of the application the choice is on, when it has
    /// any. Returns whether it has.
    pub fn open_actions(&mut self) -> bool {
        let chosen = self.entries().get(self.chosen).cloned();
        match chosen
            .filter(|entry| entry.more)
            .and_then(|entry| entry.app)
        {
            Some(app) => {
                self.actions_of = Some(app);
                self.chosen = 0;
                true
            }
            None => false,
        }
    }

    /// Back from an application's actions to the list. Returns whether
    /// actions were listed.
    pub fn close_actions(&mut self) -> bool {
        match self.actions_of.take() {
            Some(app) => {
                // The choice is on that application again.
                self.chosen = self
                    .entries()
                    .iter()
                    .position(|entry| entry.app.as_deref() == Some(app.as_str()))
                    .unwrap_or(0);
                true
            }
            None => false,
        }
    }

    /// The lines for what is typed now.
    pub fn entries(&self) -> Vec<Entry> {
        // An application's actions, in place of everything else.
        if let Some(app) = self
            .actions_of
            .as_ref()
            .and_then(|id| self.apps.iter().find(|app| app.id == *id))
        {
            return app
                .actions
                .iter()
                .take(SHOWN)
                .map(|action| Entry {
                    title: action.name.clone(),
                    detail: app.name.clone(),
                    command: start_command(
                        &App {
                            command: action.command.clone(),
                            ..app.clone()
                        },
                        &self.terminal,
                    ),
                    app: Some(app.id.clone()),
                    icon: app.icon.clone(),
                    copy: None,
                    more: false,
                    activate: app.dbus_activatable.then(|| Activation {
                        app: app.id.clone(),
                        action: Some(action.id.clone()),
                    }),
                })
                .collect();
        }
        let typed = self.typed.trim_start();
        if let Some(rest) = typed.strip_prefix(':') {
            let needle = rest.trim().to_lowercase();
            return self
                .copied
                .iter()
                .filter(|text| {
                    needle
                        .split_whitespace()
                        .all(|word| text.to_lowercase().contains(word))
                })
                .take(SHOWN)
                .map(|text| Entry {
                    title: one_line(text, 70),
                    detail: format!(
                        "Put back on the clipboard  ·  {} letters",
                        text.chars().count()
                    ),
                    command: Value::Null,
                    app: None,
                    icon: String::new(),
                    copy: Some(text.clone()),
                    more: false,
                    activate: None,
                })
                .collect();
        }
        if let Some(rest) = typed.strip_prefix('>') {
            let needle = rest.trim().to_lowercase();
            return self
                .commands
                .iter()
                .filter(|entry| {
                    needle.split_whitespace().all(|word| {
                        entry.title.to_lowercase().contains(word)
                            || entry.detail.to_lowercase().contains(word)
                    })
                })
                .take(SHOWN)
                .cloned()
                .collect();
        }
        // Applications: the best answer first, and among equals (all of
        // them, when nothing is typed) the one started most often.
        let mut apps: Vec<(u32, &App)> = self
            .apps
            .iter()
            .filter_map(|app| gevrek_system::apps::score(app, typed).map(|score| (score, app)))
            .collect();
        apps.sort_by(|one, other| {
            let with_use =
                |(score, app): &(u32, &App)| *score + self.usage.count(&app.id).min(20) * 6;
            with_use(other)
                .cmp(&with_use(one))
                .then_with(|| one.1.name.to_lowercase().cmp(&other.1.name.to_lowercase()))
        });
        let mut entries: Vec<Entry> = apps
            .into_iter()
            .map(|(_, app)| self.entry_of(app))
            .collect();
        // Open windows that answer what is typed: to switch to, after the
        // best application and before the rest.
        let needle = typed.trim().to_lowercase();
        if !needle.is_empty() {
            let switches: Vec<Entry> = self
                .windows
                .iter()
                .filter(|window| {
                    needle.split_whitespace().all(|word| {
                        window.title.to_lowercase().contains(word) || window.class.to_lowercase().contains(word)
                    })
                })
                .take(3)
                .map(|window| Entry {
                    title: window.title.clone(),
                    detail: format!("Switch to this window  ·  {}", window.class),
                    command: serde_json::json!({"command": "focus_window", "selector": {"kind": "handle", "value": window.id}}),
                    app: None,
                    // A window's class is usually its application's icon.
                    icon: window.class.clone(),
                    copy: None,
                    more: false,
                    activate: None,
                })
                .collect();
            let at = entries.len().min(1);
            entries.splice(at..at, switches);
        }
        entries.truncate(SHOWN);
        entries
    }

    /// Move the choice up or down, staying on the list.
    pub fn step(&mut self, by: i32, count: usize) {
        if count == 0 {
            self.chosen = 0;
            return;
        }
        let at = self.chosen as i32 + by;
        self.chosen = at.clamp(0, count as i32 - 1) as usize;
    }
}

/// What the launcher's drawing says was asked for.
#[derive(Debug, Default)]
pub struct Asked {
    /// Escape: put the launcher away.
    pub close: bool,
    /// This line was chosen (Enter, or a click on it).
    pub run: Option<Entry>,
    /// It was chosen with Shift+Enter: to be started inside gamescope
    /// (a game in a compositor of its own, with its own resolution).
    pub in_gamescope: bool,
    /// It was chosen with Ctrl+Enter, or with the other mouse button: to
    /// be started with an administrator's rights (Windows asks first).
    /// Only where that can be done: on Windows, for an application.
    pub elevated: bool,
}

/// The command that starts the same program inside gamescope, filling
/// the display: `gamescope -f -- <program> <arguments>`. Nothing for a
/// command that starts no program.
// (gamescope is a Wayland program: only the Linux shell offers it.)
#[cfg(any(target_os = "linux", test))]
pub fn in_gamescope(command: &Value) -> Option<Value> {
    if command["command"].as_str() != Some("exec") {
        return None;
    }
    let program = command["program"]
        .as_str()
        .filter(|program| !program.is_empty())?;
    let mut args = vec![Value::from("-f"), Value::from("--"), Value::from(program)];
    args.extend(command["args"].as_array().into_iter().flatten().cloned());
    Some(serde_json::json!({"command": "exec", "program": "gamescope", "args": args}))
}

/// Whether gamescope is installed (a program of that name on the path).
// (gamescope is a Wayland program: only the Linux shell offers it.)
#[cfg(target_os = "linux")]
pub fn gamescope_is_there() -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|folder| folder.join("gamescope").is_file())
    })
}

/// Draw the launcher and take in the keys: Escape, the arrows, Enter.
/// `hint` is what the empty field says can be typed; `icon` gives the
/// picture of a line that names an icon, when there is one by now.
pub fn draw(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    launcher: &mut Launcher,
    hint: &str,
    icon: &mut dyn FnMut(&egui::Context, &Entry) -> Option<egui::TextureHandle>,
) -> Asked {
    let context = ui.ctx().clone();
    let mut asked = Asked::default();
    let entries = launcher.entries();
    launcher.chosen = launcher.chosen.min(entries.len().saturating_sub(1));
    let pressed =
        |key: egui::Key| context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
    // The row under the field: the applications started last, and what
    // is always there. Tab goes along it and back to the list (taken
    // here, before the toolkit moves its own focus with it).
    let recent = launcher.recent();
    let fixed = always_there();
    let row: Vec<Entry> = recent
        .iter()
        .cloned()
        .chain(fixed.iter().map(|(_, entry)| entry.clone()))
        .collect();
    if context.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)) {
        launcher.on_row = tabbed(launcher.on_row, row.len(), true);
    } else if pressed(egui::Key::Tab) {
        launcher.on_row = tabbed(launcher.on_row, row.len(), false);
    }
    launcher.on_row = launcher.on_row.filter(|at| *at < row.len());
    if let Some(at) = launcher.on_row {
        // On the row: the arrows go along it, Enter presses the button,
        // Escape and the arrows up and down go back to the list.
        if pressed(egui::Key::ArrowRight) {
            launcher.on_row = Some((at + 1).min(row.len() - 1));
        }
        if pressed(egui::Key::ArrowLeft) {
            launcher.on_row = Some(at.saturating_sub(1));
        }
        if pressed(egui::Key::Enter) {
            asked.run = row.get(at).cloned();
        }
        if pressed(egui::Key::Escape)
            || context.input(|input| {
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowUp)
            })
        {
            launcher.on_row = None;
        }
    }
    // Escape from an application's actions goes back to the list first.
    if pressed(egui::Key::Escape) && !launcher.close_actions() {
        asked.close = true;
    }
    // The right arrow on an application that can be started to do other
    // things lists them; the left arrow goes back. (Otherwise the arrows
    // are the field's, to move in what is typed.)
    let has_more = entries.get(launcher.chosen).is_some_and(|entry| entry.more);
    if has_more && pressed(egui::Key::ArrowRight) {
        launcher.open_actions();
    }
    if launcher.actions_of.is_some() && pressed(egui::Key::ArrowLeft) {
        launcher.close_actions();
    }
    let entries = launcher.entries();
    launcher.chosen = launcher.chosen.min(entries.len().saturating_sub(1));
    if pressed(egui::Key::ArrowDown) {
        launcher.step(1, entries.len());
    }
    if pressed(egui::Key::ArrowUp) {
        launcher.step(-1, entries.len());
    }
    // (Shift+Enter first: Enter alone would take it otherwise.)
    if context.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter)) {
        asked.run = entries.get(launcher.chosen).cloned();
        asked.in_gamescope = true;
    }
    // (And Ctrl+Enter: with an administrator's rights.)
    if cfg!(target_os = "windows")
        && context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter))
    {
        asked.run = entries.get(launcher.chosen).cloned();
        asked.elevated = asked.run.is_some();
    }
    if pressed(egui::Key::Enter) {
        asked.run = entries.get(launcher.chosen).cloned();
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                .corner_radius(14)
                .inner_margin(14),
        )
        .show(ui, |ui| {
            let before = launcher.typed.clone();
            // The field has the keyboard from the first key on:
            // asked for before it is drawn, not after.
            let field_id = egui::Id::unique("gevrek launcher field");
            ui.memory_mut(|memory| memory.request_focus(field_id));
            ui.add(
                egui::TextEdit::singleline(&mut launcher.typed)
                    .id(field_id)
                    .hint_text(hint)
                    .font(egui::FontId::proportional(20.0))
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE),
            );
            if launcher.typed != before {
                launcher.chosen = 0;
                launcher.actions_of = None;
                // (Typing is the list's.)
                launcher.on_row = None;
            }
            ui.add_space(6.0);
            // The row: started last at the left, always there at the right.
            let side = 36.0;
            let mut over = None;
            let button = |ui: &mut egui::Ui,
                          index: usize,
                          entry: &Entry,
                          sign: Option<&str>,
                          picture: Option<egui::TextureHandle>|
             -> egui::Response {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                let on = launcher.on_row == Some(index);
                if on || response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        9.0,
                        if on {
                            theme::mix(palette.surface, palette.accent, 0.22)
                        } else {
                            palette.control_hover()
                        },
                    );
                }
                if on {
                    ui.painter().rect_stroke(
                        rect,
                        9.0,
                        egui::Stroke::new(1.5, palette.accent),
                        egui::StrokeKind::Inside,
                    );
                }
                let place = egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0));
                match (picture, sign) {
                    (Some(texture), _) => {
                        let whole =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter()
                            .image(texture.id(), place, whole, egui::Color32::WHITE);
                    }
                    (None, Some(sign)) => {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            sign,
                            egui::FontId::proportional(20.0),
                            if on { palette.text } else { palette.muted },
                        );
                    }
                    (None, None) => {
                        // Its first letter, where it has no icon.
                        ui.painter().rect_filled(place, 7.0, palette.control);
                        ui.painter().text(
                            place.center(),
                            egui::Align2::CENTER_CENTER,
                            entry
                                .title
                                .chars()
                                .take(1)
                                .collect::<String>()
                                .to_uppercase(),
                            egui::FontId::proportional(13.0),
                            palette.muted,
                        );
                    }
                }
                response.on_hover_text(&entry.title)
            };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.set_height(side);
                if recent.is_empty() {
                    ui.label(
                        egui::RichText::new("What you start from here stands in this row.")
                            .size(12.0)
                            .color(palette.faint),
                    );
                }
                for (index, entry) in recent.iter().enumerate() {
                    let picture = (!entry.icon.is_empty())
                        .then(|| icon(&context, entry))
                        .flatten();
                    let pressed = button(ui, index, entry, None, picture);
                    if pressed.clicked() {
                        over = Some(entry.clone());
                    }
                    // (The other button: with an administrator's rights.)
                    if cfg!(target_os = "windows") && pressed.secondary_clicked() {
                        over = Some(entry.clone());
                        asked.elevated = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // (Drawn from the right: the last first.)
                    for (offset, (sign, entry)) in fixed.iter().enumerate().rev() {
                        if button(ui, recent.len() + offset, entry, Some(sign), None).clicked() {
                            over = Some(entry.clone());
                        }
                    }
                });
            });
            if over.is_some() {
                asked.run = over;
            }
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
            for (index, entry) in entries.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 42.0),
                    egui::Sense::click(),
                );
                // (While Tab is on the row, the list's choice is not the
                // one Enter takes: it is shown fainter.)
                let chosen = index == launcher.chosen;
                if chosen && launcher.on_row.is_some() {
                    ui.painter().rect_filled(rect, 8.0, palette.control);
                } else if chosen {
                    ui.painter().rect_filled(
                        rect,
                        8.0,
                        theme::mix(palette.surface, palette.accent, 0.22),
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 8.0, palette.control_hover());
                }
                let picture = (!entry.icon.is_empty())
                    .then(|| icon(&context, entry))
                    .flatten();
                let icon_place = egui::Rect::from_center_size(
                    rect.left_center() + egui::vec2(26.0, 0.0),
                    egui::vec2(30.0, 30.0),
                );
                match &picture {
                    Some(texture) => {
                        let whole =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter()
                            .image(texture.id(), icon_place, whole, egui::Color32::WHITE);
                    }
                    None => {
                        // Its first letter, where it has no icon.
                        ui.painter().rect_filled(icon_place, 8.0, palette.control);
                        let letter: String = entry
                            .title
                            .chars()
                            .take(1)
                            .collect::<String>()
                            .to_uppercase();
                        ui.painter().text(
                            icon_place.center(),
                            egui::Align2::CENTER_CENTER,
                            letter,
                            egui::FontId::proportional(15.0),
                            palette.muted,
                        );
                    }
                }
                ui.painter().text(
                    rect.left_center()
                        + egui::vec2(52.0, if entry.detail.is_empty() { 0.0 } else { -8.0 }),
                    egui::Align2::LEFT_CENTER,
                    &entry.title,
                    egui::FontId::proportional(16.0),
                    palette.text,
                );
                if !entry.detail.is_empty() {
                    let detail: String = entry.detail.chars().take(84).collect();
                    ui.painter().text(
                        rect.left_center() + egui::vec2(52.0, 11.0),
                        egui::Align2::LEFT_CENTER,
                        detail,
                        egui::FontId::proportional(12.0),
                        palette.muted,
                    );
                }
                // More that it can be started to do: the right arrow.
                if entry.more {
                    ui.painter().text(
                        rect.right_center() - egui::vec2(14.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        gevrek_ui::theme::icons::CARET_RIGHT,
                        egui::FontId::proportional(14.0),
                        if chosen { palette.text } else { palette.faint },
                    );
                }
                if response.hovered()
                    && response
                        .ctx
                        .input(|input| input.pointer.delta() != egui::Vec2::ZERO)
                {
                    launcher.chosen = index;
                }
                if response.clicked() {
                    asked.run = Some(entry.clone());
                }
                if cfg!(target_os = "windows") && response.secondary_clicked() {
                    asked.run = Some(entry.clone());
                    asked.elevated = true;
                }
            }
            if entries.is_empty() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Nothing like that.").color(palette.muted));
            }
        });
    asked
}

/// The command that starts an application: its program and arguments, in
/// the terminal when it asks for one.
pub fn start_command(app: &App, terminal: &str) -> Value {
    let mut words = app.command.clone();
    if app.terminal && !words.is_empty() {
        let mut wrapped = vec![terminal.to_string(), "-e".to_string()];
        wrapped.append(&mut words);
        words = wrapped;
    }
    // An application that is only started over the bus has no command.
    let Some((program, args)) = words.split_first() else {
        return Value::Null;
    };
    serde_json::json!({"command": "exec", "program": program, "args": args})
}

/// Everything gevrek can be told to do that needs nothing said with it.
fn commands() -> Vec<Entry> {
    let Some(node) = gevrek_schema::schema().at("/desktop/bindings/0/command") else {
        return Vec::new();
    };
    let gevrek_schema::Kind::Union(variants) = node.kind() else {
        return Vec::new();
    };
    variants
        .iter()
        .filter(|variant| match &variant.shape {
            gevrek_schema::VariantShape::Word => true,
            gevrek_schema::VariantShape::Tagged { fields, .. } => {
                fields.iter().all(|field| !field.required)
            }
            gevrek_schema::VariantShape::Wrapped(_) => false,
        })
        .filter(|variant| variant.name != "exec")
        .map(|variant| Entry {
            title: gevrek_schema::title(variant.name),
            detail: variant.description.lines().next().unwrap_or("").to_string(),
            command: variant.blank(),
            app: None,
            icon: String::new(),
            copy: None,
            more: false,
            activate: None,
        })
        // The tutorial and the welcome: found by their names, whatever
        // keys a person has for them.
        .chain(
            [
                (
                    "Tutorial",
                    "Learn gevrek by doing: begin, or go on where it was left",
                    "tutorial",
                ),
                (
                    "Welcome",
                    "The window of the first start: the setup and the tour",
                    "welcome",
                ),
            ]
            .map(|(title, detail, action)| Entry {
                title: title.to_string(),
                detail: detail.to_string(),
                command: serde_json::json!({"command": "shell", "action": action}),
                app: None,
                icon: String::new(),
                copy: None,
                more: false,
                activate: None,
            }),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_started_inside_gamescope_by_its_own_command() {
        let command = serde_json::json!({"command": "exec", "program": "/usr/bin/supertux2", "args": ["--fullscreen"]});
        assert_eq!(
            in_gamescope(&command),
            Some(
                serde_json::json!({"command": "exec", "program": "gamescope",
                "args": ["-f", "--", "/usr/bin/supertux2", "--fullscreen"]})
            )
        );
        // What starts no program is left as it is.
        assert_eq!(
            in_gamescope(&serde_json::json!({"command": "focus_window"})),
            None
        );
        assert_eq!(
            in_gamescope(&serde_json::json!({"command": "exec", "program": ""})),
            None
        );
        assert_eq!(in_gamescope(&Value::Null), None);
    }
    use std::path::PathBuf;

    #[test]
    fn an_application_of_the_bus_is_started_there_and_keeps_its_command_for_else() {
        use gevrek_system::apps::AppAction;
        let mut calculator = app("Calculator", &["gnome-calculator"], false);
        calculator.id = "org.gnome.Calculator".to_string();
        calculator.dbus_activatable = true;
        calculator.actions = vec![AppAction {
            id: "new".to_string(),
            name: "New Window".to_string(),
            command: Vec::new(),
        }];
        let mut launcher = Launcher::new(vec![calculator, app("Files", &["files"], false)], "foot");
        launcher.typed = "calc".to_string();
        let entries = launcher.entries();
        assert_eq!(
            entries[0].activate,
            Some(Activation {
                app: "org.gnome.Calculator".to_string(),
                action: None
            })
        );
        assert_eq!(entries[0].command["program"], "gnome-calculator");
        launcher.typed = "files".to_string();
        assert_eq!(launcher.entries()[0].activate, None);
        // An action without a command of its own has only the bus.
        launcher.actions_of = Some("org.gnome.Calculator".to_string());
        let actions = launcher.entries();
        assert_eq!(
            actions[0]
                .activate
                .as_ref()
                .and_then(|activation| activation.action.as_deref()),
            Some("new")
        );
        assert!(actions[0].command.is_null());
    }

    fn app(name: &str, command: &[&str], terminal: bool) -> App {
        App {
            id: name.to_lowercase(),
            name: name.to_string(),
            generic_name: String::new(),
            comment: format!("{name} does things"),
            keywords: Vec::new(),
            icon: String::new(),
            command: command.iter().map(|word| word.to_string()).collect(),
            terminal,
            dbus_activatable: false,
            directory: None,
            entry: PathBuf::new(),
            actions: Vec::new(),
        }
    }

    #[test]
    fn an_applications_actions_are_listed_in_its_place_and_left_again() {
        use gevrek_system::apps::AppAction;
        let mut browser = app("Browser", &["browser"], false);
        browser.actions = vec![
            AppAction {
                id: "new".to_string(),
                name: "New Window".to_string(),
                command: vec!["browser".to_string(), "--new-window".to_string()],
            },
            AppAction {
                id: "private".to_string(),
                name: "New Private Window".to_string(),
                command: vec!["browser".to_string(), "--private".to_string()],
            },
        ];
        let mut launcher = Launcher::new(vec![app("Editor", &["editor"], false), browser], "term");
        launcher.typed = "brow".to_string();
        let entries = launcher.entries();
        assert_eq!(entries[0].title, "Browser");
        assert!(entries[0].more, "it has more it can be started to do");
        // An application without actions has no list to open.
        launcher.typed = "edit".to_string();
        assert!(!launcher.entries()[0].more && !launcher.open_actions());

        launcher.typed = "brow".to_string();
        assert!(launcher.open_actions());
        let actions = launcher.entries();
        let titles: Vec<&str> = actions.iter().map(|entry| entry.title.as_str()).collect();
        assert_eq!(titles, ["New Window", "New Private Window"]);
        assert_eq!(actions[1].detail, "Browser");
        assert_eq!(
            actions[1].command,
            serde_json::json!({"command": "exec", "program": "browser", "args": ["--private"]})
        );
        assert_eq!(
            actions[1].app.as_deref(),
            Some("browser"),
            "counted as a start of it"
        );
        // Back: the list again, the choice on the application.
        assert!(launcher.close_actions());
        assert_eq!(launcher.entries()[launcher.chosen].title, "Browser");
        assert!(!launcher.close_actions());
    }

    #[test]
    fn what_is_typed_finds_applications_and_after_a_mark_commands() {
        let mut launcher = Launcher::new(
            vec![
                app("Firefox", &["firefox"], false),
                app("Files", &["nautilus", "--new-window"], false),
            ],
            "foot",
        );
        launcher.typed = "fire".to_string();
        let entries = launcher.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Firefox");
        assert_eq!(
            entries[0].command,
            serde_json::json!({"command": "exec", "program": "firefox", "args": []})
        );
        launcher.typed = "> quit".to_string();
        let entries = launcher.entries();
        assert!(
            entries
                .iter()
                .any(|entry| entry.command == serde_json::json!({"command": "quit"})),
            "{entries:?}"
        );
        // A command that needs something said with it is not offered here.
        launcher.typed = ">".to_string();
        launcher.commands.iter().for_each(|entry| {
            let read: Result<gevrek_model::command::DispatchCommand, _> =
                serde_json::from_value(entry.command.clone());
            assert!(
                read.is_ok(),
                "{} is not a whole command: {}",
                entry.title,
                entry.command
            );
        });
        assert!(launcher.commands.len() > 20);
    }

    #[test]
    fn an_application_for_the_terminal_is_started_in_one() {
        let command = start_command(&app("htop", &["htop", "-d", "5"], true), "foot");
        assert_eq!(
            command,
            serde_json::json!({"command": "exec", "program": "foot", "args": ["-e", "htop", "-d", "5"]})
        );
        let plain = start_command(&app("Files", &["nautilus", "--new-window"], false), "foot");
        assert_eq!(plain["args"], serde_json::json!(["--new-window"]));
    }

    #[test]
    fn what_is_started_often_comes_first_and_open_windows_are_offered() {
        let apps = vec![
            app("Files", &["nautilus"], false),
            app("Firefox", &["firefox"], false),
            app("Figma", &["figma"], false),
        ];
        let folder = std::env::temp_dir().join(format!("gevrek-launcher-{}", std::process::id()));
        let path = folder.join("usage.json");
        let mut usage = Usage::load(&path);
        assert_eq!(usage.count("firefox"), 0);
        for _ in 0..3 {
            usage.bump("firefox", &path);
        }
        assert_eq!(Usage::load(&path).count("firefox"), 3, "it is written down");
        let windows = vec![
            Open {
                id: 7,
                title: "Budget 2026 - Files".to_string(),
                class: "org.gnome.Nautilus".to_string(),
            },
            Open {
                id: 9,
                title: "notes.txt".to_string(),
                class: "editor".to_string(),
            },
        ];
        let mut launcher = Launcher::new(apps, "foot").with(windows, usage);
        // Nothing typed: by use, then by name; no windows.
        let names: Vec<String> = launcher
            .entries()
            .into_iter()
            .map(|entry| entry.title)
            .collect();
        assert_eq!(names, ["Firefox", "Figma", "Files"]);
        // "fi" fits all three equally at the start: the one used most first.
        launcher.typed = "fi".to_string();
        let entries = launcher.entries();
        assert_eq!(entries[0].title, "Firefox");
        assert_eq!(entries[0].app.as_deref(), Some("firefox"));
        // The open window that answers "files" is offered right after the
        // best application.
        launcher.typed = "files".to_string();
        let entries = launcher.entries();
        assert_eq!(entries[0].title, "Files");
        assert_eq!(entries[1].title, "Budget 2026 - Files");
        assert_eq!(
            entries[1].command,
            serde_json::json!({"command": "focus_window", "selector": {"kind": "handle", "value": 7}})
        );
        let read: gevrek_model::command::DispatchCommand =
            serde_json::from_value(entries[1].command.clone()).expect("a command");
        assert!(!matches!(
            read,
            gevrek_model::command::DispatchCommand::Unknown(_)
        ));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn what_was_started_last_stands_in_the_row_and_tab_goes_along_it() {
        let apps = vec![
            app("Files", &["nautilus"], false),
            app("Firefox", &["firefox"], false),
            app("Figma", &["figma"], false),
        ];
        let folder =
            std::env::temp_dir().join(format!("gevrek-launcher-row-{}", std::process::id()));
        let path = folder.join("usage.json");
        let mut usage = Usage::load(&path);
        assert!(usage.recent().is_empty());
        for id in ["files", "firefox", "gone", "files"] {
            usage.bump(id, &path);
        }
        // The newest first, each once; written down beside the counts.
        assert_eq!(usage.recent(), ["files", "gone", "firefox"]);
        let read = Usage::load(&path);
        assert_eq!(read.recent(), ["files", "gone", "firefox"]);
        assert_eq!(read.count("files"), 2);
        // No more than are remembered.
        let mut many = Usage::default();
        for index in 0..REMEMBERED + 5 {
            many.started(&format!("app-{index}"));
        }
        assert_eq!(many.recent().len(), REMEMBERED);
        assert_eq!(many.recent()[0], format!("app-{}", REMEMBERED + 4));
        // In the row: those of them that are installed, as their lines.
        let launcher = Launcher::new(apps, "foot").with(Vec::new(), read);
        let row: Vec<String> = launcher
            .recent()
            .into_iter()
            .map(|entry| entry.title)
            .collect();
        assert_eq!(row, ["Files", "Firefox"]);
        assert_eq!(launcher.recent()[0].app.as_deref(), Some("files"));
        // Tab: from the list along the row and back to the list; Shift+Tab
        // the other way.
        let mut at = None;
        let mut seen = Vec::new();
        for _ in 0..6 {
            at = tabbed(at, 5, false);
            seen.push(at);
        }
        assert_eq!(seen, [Some(0), Some(1), Some(2), Some(3), Some(4), None]);
        assert_eq!(tabbed(None, 5, true), Some(4));
        assert_eq!(tabbed(Some(0), 5, true), None);
        assert_eq!(tabbed(None, 0, false), None);
        // What is always there: three, each a command the window manager
        // knows.
        let fixed = always_there();
        assert_eq!(fixed.len(), 3);
        for (_, entry) in &fixed {
            let read: gevrek_model::command::DispatchCommand =
                serde_json::from_value(entry.command.clone()).expect("a command");
            assert!(
                !matches!(read, gevrek_model::command::DispatchCommand::Unknown(_)),
                "{}",
                entry.title
            );
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The launcher drawn into a picture, to be looked at:
    ///
    ///     GEVREK_LOOK_OUT=launcher.png cargo test -p gevrek-shell \
    ///         launcher_is_drawn_to_be_looked_at -- --ignored
    ///
    /// `GEVREK_LOOK_TABS=4` presses Tab so many times before the picture.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_launcher_is_drawn_to_be_looked_at() {
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let out =
            std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let tabs: usize = said("GEVREK_LOOK_TABS")
            .and_then(|text| text.parse().ok())
            .unwrap_or(0);
        let names = [
            "Files",
            "Firefox",
            "Figma",
            "Terminal",
            "Calculator",
            "Music",
            "Mail",
            "Maps",
            "Notes",
        ];
        let apps: Vec<App> = names.iter().map(|name| app(name, &[name], false)).collect();
        let mut usage = Usage::default();
        for id in ["mail", "music", "terminal", "firefox"] {
            usage.started(id);
        }
        let mut launcher = Launcher::new(apps, "foot").with(Vec::new(), usage);
        let mut painter = gevrek_ui::picture::Painter::sized((SIZE.0, SIZE.1))
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut pixels = Vec::new();
        for step in 0..(4 + tabs) {
            let events = if step >= 2 && step < 2 + tabs {
                vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                Vec::new()
            };
            pixels = painter.frame_of(&context, 10.0 + step as f64 * 0.4, events, |ui| {
                draw(
                    ui,
                    &palette,
                    &mut launcher,
                    "An application · > a command",
                    &mut |_, _| None,
                );
            });
        }
        println!("on the row: {:?}", launcher.on_row);
        painter.save(&pixels, &out).expect("the picture is written");
    }

    #[test]
    fn what_was_copied_is_listed_after_a_colon() {
        let mut launcher = Launcher::new(Vec::new(), "foot").with_copied(vec![
            "the newest text".to_string(),
            "an address:\n12 Quince Lane".to_string(),
        ]);
        launcher.typed = ":".to_string();
        let entries = launcher.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].copy.as_deref(), Some("the newest text"));
        assert_eq!(entries[1].title, "an address: 12 Quince Lane");
        launcher.typed = ": quince".to_string();
        let entries = launcher.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].copy.as_deref(),
            Some("an address:\n12 Quince Lane")
        );
    }

    #[test]
    fn the_open_windows_are_read_from_the_window_managers_state() {
        let wm = serde_json::json!({"monitors": [
            {"workspaces": [
                {"detailed_windows": [
                    {"window_id": 7, "title": "notes.txt", "class": "editor"},
                    // No class: still a window to switch to.
                    {"window_id": 9, "title": "Budget"},
                    // No title, or no number: nothing to show or to switch to.
                    {"window_id": 11},
                    {"title": "lost"},
                ]},
                {"detailed_windows": []},
            ]},
            {"workspaces": [{"detailed_windows": [
                {"window_id": 12, "title": "on the other display", "class": "viewer"},
            ]}]},
        ]});
        let windows = open_windows(&wm);
        let ids: Vec<u64> = windows.iter().map(|window| window.id).collect();
        assert_eq!(ids, [7, 9, 12]);
        assert_eq!(windows[0].class, "editor");
        assert_eq!(windows[1].class, "");
        assert!(open_windows(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn a_line_names_its_application_and_the_list_can_be_read_again() {
        let mut launcher = Launcher::new(vec![app("Files", &["nautilus"], false)], "foot");
        let entries = launcher.entries();
        let id = entries[0].app.as_deref().expect("an application's line");
        assert_eq!(launcher.app(id).map(|app| app.name.as_str()), Some("Files"));
        assert!(launcher.app("nothing").is_none());
        // Read again while it is open: what is typed stays, the lines follow.
        launcher.typed = "fire".to_string();
        assert!(launcher.entries().is_empty());
        launcher.set_apps(vec![app("Firefox", &["firefox"], false)]);
        assert_eq!(launcher.entries()[0].title, "Firefox");
        assert!(launcher.app(id).is_none());
    }

    #[test]
    fn a_long_text_is_one_short_line_in_a_list() {
        assert_eq!(
            one_line("  several\n  lines\tof text ", 40),
            "several lines of text"
        );
        assert_eq!(one_line("abcdefghij", 6), "abcde…");
    }

    #[test]
    fn the_choice_stays_on_the_list() {
        let mut launcher = Launcher::new(Vec::new(), "foot");
        launcher.step(1, 3);
        launcher.step(1, 3);
        launcher.step(1, 3);
        assert_eq!(launcher.chosen, 2);
        launcher.step(-5, 3);
        assert_eq!(launcher.chosen, 0);
        launcher.step(1, 0);
        assert_eq!(launcher.chosen, 0);
    }

    /// The counts are a file anyone may have written: a count as large
    /// as numbers get is not counted past (it ended a debug build, and
    /// began again at nought otherwise), and what is no count is none.
    #[test]
    fn counts_from_outside_are_lived_with() {
        let folder =
            std::env::temp_dir().join(format!("gevrek-launcher-outside-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let path = folder.join("usage.json");
        std::fs::write(&path, r#"{"firefox": 4294967295}"#).expect("written");
        let mut usage = Usage::load(&path);
        usage.started("firefox");
        assert_eq!(usage.count("firefox"), u32::MAX);

        for nonsense in [
            r#"{"firefox": -1}"#,
            r#"{"firefox": 4294967296}"#,
            r#"{"firefox": "often"}"#,
            r#"["firefox"]"#,
            "null",
            "",
            r#"{"firefox": 3"#,
        ] {
            std::fs::write(&path, nonsense).expect("written");
            std::fs::write(recent_file(&path), nonsense).expect("written");
            let mut usage = Usage::load(&path);
            assert_eq!(usage.count("firefox"), 0, "{nonsense}");
            usage.started("firefox");
            assert_eq!(usage.recent(), ["firefox".to_string()]);
        }
        let _ = std::fs::remove_dir_all(folder);
    }
}
