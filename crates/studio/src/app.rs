//! The window: the bar at the top, the list of pages, the page.

use crate::document::Document;
use crate::form::{Form, Loose};
use crate::inspect::{self, clock};
use crate::link::{Event, Level, Link, Peer, Request};
use crate::pages::{Page, Show};
use crate::session::{Fetched, RemoteNotice, Session};
use crate::tryout::TryOut;
use egui::{Color32, RichText, Ui};
use gevrek_model::settings::ConfigIssueSeverity;
use gevrek_model::Config;
use gevrek_ui::theme::{self, icons, Palette};
use gevrek_ui::widgets;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long after the last change a live edit is sent.
const LIVE_PAUSE: Duration = Duration::from_millis(350);
const TOAST: Duration = Duration::from_secs(5);
const KEPT_EVENTS: usize = 500;
const KEPT_LOG: usize = 1000;

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Good,
    Note,
    Warn,
    Bad,
}

struct Toast {
    tone: Tone,
    text: String,
    since: Instant,
}

struct EventRow {
    time_ms: i64,
    kind: String,
    window: Option<u64>,
    fields: String,
}

struct LogRow {
    time_ms: i64,
    level: Level,
    message: String,
}

/// Where the configuration comes from and goes to.
pub enum Source {
    /// The running window manager at this address.
    WindowManager(String),
    /// A file, with no window manager involved.
    File(PathBuf),
}

pub struct Options {
    pub source: Source,
    pub page: Page,
    /// Where the appearance of this window is kept.
    pub appearance: PathBuf,
    /// Steps the window takes by itself (see [`Step`]): for checks.
    pub script: Vec<Step>,
}

/// What the window can be told to do without a person, one after the
/// other: `--script "set /desktop/shell/panel/height 44; apply; quit"`. Each step
/// waits until the window manager's configuration is there and nothing is
/// on its way to it, so a script reads like what a person would do.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Put a value (JSON) at a place of the configuration.
    Set(String, Value),
    Apply,
    /// Apply on trial (`tryout`); `Keep` and `Back` answer it.
    Try,
    Keep,
    Back,
    Undo,
    /// Run a command (JSON), as the command centre's button does.
    Run(Value),
    Live(bool),
    /// Start or end the sandbox.
    Sandbox(bool),
    Page(Page),
    /// Wait for the keys of a binding, as its button does.
    Record(usize),
    /// Wait for the keys of a node of the graph that starts at a key, as
    /// its button does.
    RecordNode(usize),
    /// Do nothing for so many milliseconds.
    Wait(u64),
    Quit,
}

impl Step {
    /// Read `set /a/b 3; apply; quit`.
    pub fn parse_all(text: &str) -> Result<Vec<Step>, String> {
        let mut steps = Vec::new();
        for part in text
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let (word, rest) = part.split_once(char::is_whitespace).unwrap_or((part, ""));
            let rest = rest.trim();
            steps.push(match word {
                "set" => {
                    let (pointer, json) = rest
                        .split_once(char::is_whitespace)
                        .ok_or_else(|| format!("{part}: set PLACE VALUE"))?;
                    let value = serde_json::from_str(json.trim())
                        .map_err(|error| format!("{part}: {error}"))?;
                    Step::Set(pointer.to_string(), value)
                }
                "run" => Step::Run(
                    serde_json::from_str(rest).map_err(|error| format!("{part}: {error}"))?,
                ),
                "apply" => Step::Apply,
                "try" => Step::Try,
                "keep" => Step::Keep,
                "back" => Step::Back,
                "undo" => Step::Undo,
                "live" => Step::Live(rest != "off"),
                "sandbox" => Step::Sandbox(rest != "off"),
                "page" => {
                    Step::Page(Page::from_key(rest).ok_or_else(|| format!("{part}: no such page"))?)
                }
                "record" => Step::Record(
                    rest.parse()
                        .map_err(|_| format!("{part}: record BINDING (its place in the list)"))?,
                ),
                "record-node" => Step::RecordNode(
                    rest.parse()
                        .map_err(|_| format!("{part}: record-node NODE (its number)"))?,
                ),
                "wait" => Step::Wait(
                    rest.parse()
                        .map_err(|_| format!("{part}: wait MILLISECONDS"))?,
                ),
                "quit" => Step::Quit,
                _ => {
                    return Err(format!(
                        "{part}: not a step (set, apply, try, keep, back, undo, run, live, page, record, record-node, wait, quit)"
                    ))
                }
            });
        }
        Ok(steps)
    }
}

pub struct Studio {
    link: Link,
    file: Option<PathBuf>,
    address: String,
    peer: Option<Peer>,
    /// Why there is no line, while there is none.
    line_down: String,
    session: Option<Session>,
    client_id: String,
    document: Document,
    /// The document's generation the session and the file know.
    seen_generation: u64,
    /// The generation that was sent last, and when the document last changed.
    sent_generation: u64,
    changed_at: Instant,
    applying: bool,
    live: bool,
    page: Page,
    appearance: gevrek_ui::Settings,
    appearance_path: PathBuf,
    appearance_open: bool,
    fonts_ready: bool,
    toasts: Vec<Toast>,
    search: Option<String>,
    /// A setting to show a ring around, until when.
    highlight: Option<(String, Instant)>,
    events: VecDeque<EventRow>,
    log: VecDeque<LogRow>,
    state: Option<Value>,
    state_asked: Option<Instant>,
    raw: String,
    raw_generation: u64,
    raw_error: String,
    script: VecDeque<Step>,
    /// The script waits until then.
    script_wait: Option<Instant>,
    /// The command being put together in the command centre.
    command: Loose,
    command_filter: String,
    /// What is typed to search the events, the log and the windows.
    events_filter: String,
    log_filter: String,
    windows_filter: String,
    /// The least serious level of the log that is shown.
    log_lowest: Level,
    /// New display settings are on trial until then.
    trial: Option<Instant>,
    /// The whole configuration is on trial (`tryout`).
    try_out: Option<TryOut>,
    bindings: crate::bindings::BindingsPage,
    /// Every key for the studio, while a binding's keys are recorded.
    #[cfg(target_os = "linux")]
    all_keys: Option<crate::all_keys::AllKeys>,
    /// The compositor does not give them: not asked again.
    #[cfg(target_os = "linux")]
    all_keys_refused: bool,
    /// The touchpad's pinches, which the toolkit does not hear on
    /// Wayland (`pinch.rs`): asked for at the first frame.
    #[cfg(target_os = "linux")]
    pinch: Option<crate::pinch::Pinch>,
    #[cfg(target_os = "linux")]
    pinch_asked: bool,
    /// The same on Windows: the keyboard is listened to before the window
    /// manager hears of a key (`gevrek_system::every_key`).
    #[cfg(windows)]
    every_key: Option<gevrek_system::every_key::EveryKey>,
    /// Windows does not let it: not asked again.
    #[cfg(windows)]
    every_key_refused: bool,
    appearance_preview: crate::preview::AppearancePreview,
    layout_preview: crate::preview::LayoutPreview,
    /// The workspaces of each display.
    workspaces: crate::workspaces::Page,
    zones_edit: crate::zones_edit::ZonesEdit,
    /// This studio asked the window manager to have a window picked,
    /// and has not heard which yet.
    picking: bool,
    arrangement: crate::displays::Arrangement,
    night_light: crate::night_light::Card,
    wallpaper_preview: crate::wallpaper_preview::WallpaperPreview,
    history: crate::history::HistoryPage,
    sandbox: Option<crate::sandbox::Sandbox>,
    /// What runs (or what the file holds), as JSON, and the revision it
    /// was made from.
    base: Option<(u64, Value)>,
    graph: crate::graph::GraphPage,
    /// The file that was to be opened is there and could not be read (it
    /// does not parse, or is not valid): what is shown is gevrek's
    /// defaults, and saving them over the file would lose it. While this
    /// is set nothing is saved.
    file_problem: Option<String>,
    /// The window manager answers, and its configuration could not be
    /// fetched: what is shown is gevrek's defaults, and nothing can be
    /// applied.
    fetch_problem: Option<String>,
}

/// Where the schema describes a command.
const COMMAND: &str = "/desktop/bindings/0/command";

impl Studio {
    pub fn new(context: &egui::Context, options: Options) -> Self {
        let wake = context.clone();
        let mut file_problem = None;
        let (link, file, address, document) = match options.source {
            Source::WindowManager(address) => (
                Link::start(address.clone(), move || wake.request_repaint()),
                None,
                address,
                Document::new(&Config::default()),
            ),
            Source::File(path) => {
                let config = Config::load(&path).unwrap_or_else(|error| {
                    log::warn!(
                        "{}: {error}; starting from gevrek's defaults",
                        path.display()
                    );
                    // (A file that is not there yet is made by the first
                    // save; one that is there is not written over.)
                    if path.exists() {
                        file_problem = Some(format!("{}: {error}", path.display()));
                    }
                    Config::default()
                });
                (
                    Link::none(),
                    Some(path),
                    String::new(),
                    Document::new(&config),
                )
            }
        };
        let generation = document.generation();
        Self {
            file_problem,
            fetch_problem: None,
            link,
            file,
            address,
            peer: None,
            line_down: "connecting".to_string(),
            session: None,
            client_id: format!("studio-{}", std::process::id()),
            document,
            seen_generation: generation,
            sent_generation: generation,
            changed_at: Instant::now(),
            applying: false,
            live: false,
            page: options.page,
            appearance: gevrek_ui::Settings::load(&options.appearance),
            appearance_path: options.appearance,
            appearance_open: false,
            fonts_ready: false,
            toasts: Vec::new(),
            search: None,
            highlight: None,
            events: VecDeque::new(),
            log: VecDeque::new(),
            state: None,
            state_asked: None,
            raw: String::new(),
            raw_generation: u64::MAX,
            raw_error: String::new(),
            script: options.script.into(),
            script_wait: None,
            command: Loose(serde_json::json!({"command": "focus_next"})),
            command_filter: String::new(),
            events_filter: String::new(),
            log_filter: String::new(),
            windows_filter: String::new(),
            log_lowest: Level::Debug,
            trial: None,
            try_out: None,
            bindings: Default::default(),
            #[cfg(target_os = "linux")]
            all_keys: None,
            #[cfg(target_os = "linux")]
            all_keys_refused: false,
            #[cfg(target_os = "linux")]
            pinch: None,
            #[cfg(target_os = "linux")]
            pinch_asked: false,
            #[cfg(windows)]
            every_key: None,
            #[cfg(windows)]
            every_key_refused: false,
            appearance_preview: Default::default(),
            layout_preview: Default::default(),
            workspaces: Default::default(),
            zones_edit: Default::default(),
            picking: false,
            arrangement: Default::default(),
            night_light: Default::default(),
            wallpaper_preview: Default::default(),
            history: Default::default(),
            sandbox: None,
            base: None,
            graph: Default::default(),
        }
    }

    /// The next steps of the script, as far as they can be taken now.
    fn run_script(&mut self, context: &egui::Context) {
        while let Some(step) = self.script.front().cloned() {
            // Like a person: only with the configuration on the screen
            // and nothing on its way.
            let ready = (self.session.is_some() || self.file.is_some()) && !self.applying;
            let waiting = self.script_wait.is_some_and(|until| Instant::now() < until);
            if !ready || waiting {
                context.request_repaint_after(Duration::from_millis(50));
                return;
            }
            self.script_wait = None;
            self.script.pop_front();
            match step {
                Step::Set(pointer, value) => {
                    self.document.set(&pointer, value);
                    self.follow_edits();
                }
                Step::Apply => {
                    self.follow_edits();
                    self.apply();
                }
                Step::Try => {
                    self.follow_edits();
                    self.begin_trial();
                }
                Step::Keep => self.end_trial(true),
                Step::Back => self.end_trial(false),
                Step::Undo => {
                    self.document.undo();
                    self.follow_edits();
                }
                Step::Run(command) => self.link.send(Request::Dispatch(command.to_string())),
                Step::Live(on) => self.live = on,
                Step::Sandbox(on) => {
                    if let Some(sandbox) = self.sandbox.take() {
                        sandbox.stop();
                    }
                    if on {
                        if let Some(config) = self.document.config() {
                            match crate::sandbox::Sandbox::start(config, self.document.generation())
                            {
                                Ok(sandbox) => self.sandbox = Some(sandbox),
                                Err(error) => log::error!("sandbox: {error}"),
                            }
                        }
                    }
                }
                Step::Page(page) => self.page = page,
                Step::Record(binding) => {
                    self.page = Page::Bindings;
                    self.bindings.record(binding);
                }
                Step::RecordNode(node) => {
                    self.page = Page::Graph;
                    self.graph.record(node);
                }
                Step::Wait(milliseconds) => {
                    self.script_wait = Some(Instant::now() + Duration::from_millis(milliseconds));
                }
                Step::Quit => context.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }

    fn toast(&mut self, tone: Tone, text: impl Into<String>) {
        self.toasts.push(Toast {
            tone,
            text: text.into(),
            since: Instant::now(),
        });
    }

    fn connected(&self) -> bool {
        self.peer.is_some()
    }

    /// Unsaved edits: the document is not what the window manager runs
    /// (or what the file holds).
    fn dirty(&self) -> bool {
        match &self.session {
            Some(session) => session.is_dirty(),
            None => self.file.is_some() && self.document.can_undo(),
        }
    }

    // --- what the window manager says -------------------------------

    fn take_in(&mut self) {
        for event in self.link.poll() {
            match event {
                Event::Connected(peer) => {
                    // A window manager that started again counts its
                    // revisions from the start.
                    if let Some(session) = &mut self.session {
                        session.revision_lost();
                    }
                    self.peer = Some(peer);
                    self.line_down.clear();
                }
                Event::Disconnected(reason) => {
                    self.picking = false;
                    if self.peer.take().is_some() {
                        self.toast(Tone::Warn, "The window manager went away. Trying again.");
                    }
                    self.applying = false;
                    self.line_down = reason;
                    // (What it said of itself is of a window manager that
                    // is gone: nothing is shown as if it still ran.)
                    self.state = None;
                }
                Event::Configuration(config, revision) => {
                    self.fetch_problem = None;
                    self.remote_configuration(*config, revision)
                }
                Event::Applied(revision) => {
                    self.applying = false;
                    if let Some(session) = &mut self.session {
                        session.applied(revision);
                    }
                    // Edits made while it was on its way are a new draft.
                    self.seen_generation = self.sent_generation;
                    if !self.live {
                        self.toast(Tone::Good, format!("Applied (revision {revision})"));
                    }
                }
                Event::Conflict(message) => {
                    self.applying = false;
                    self.try_out = None;
                    self.toast(Tone::Warn, message);
                    self.link.send(Request::Fetch);
                }
                // The configuration could not be fetched: what the window
                // shows is not what the window manager runs.
                Event::Log { level: gevrek_link::Level::Error, ref message, .. }
                    if message.starts_with(gevrek_link::FETCH_FAILED) && self.session.is_none() =>
                {
                    self.fetch_problem = Some(message.clone());
                }
                Event::ApplyFailed(message) => {
                    self.applying = false;
                    self.try_out = None;
                    crate::tryout::forget_revert();
                    self.live = false;
                    self.toast(Tone::Bad, format!("Not applied: {message}"));
                }
                Event::Changed { revision, origin } => {
                    let notice = self
                        .session
                        .as_ref()
                        .map_or(RemoteNotice::Fetch, |session| {
                            session.on_remote_revision(revision, &origin)
                        });
                    if notice == RemoteNotice::Fetch {
                        self.link.send(Request::Fetch);
                    }
                }
                Event::Runtime {
                    kind,
                    timestamp_ms,
                    window,
                    fields,
                } => {
                    if kind == "window.picked" {
                        self.window_picked(&fields);
                    }
                    if kind == "output.trial" {
                        let field = |name: &str| {
                            fields
                                .iter()
                                .find(|(key, _)| key == name)
                                .map(|(_, value)| value.as_str())
                        };
                        self.trial = match field("state") {
                            Some("started") => {
                                let seconds = field("seconds")
                                    .and_then(|text| text.parse::<f64>().ok())
                                    .filter(|seconds| seconds.is_finite())
                                    .unwrap_or(15.0)
                                    .clamp(0.0, 600.0);
                                Some(Instant::now() + Duration::from_secs_f64(seconds))
                            }
                            _ => None,
                        };
                    }
                    let fields = fields
                        .iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect::<Vec<_>>()
                        .join("  ");
                    self.events.push_back(EventRow {
                        time_ms: timestamp_ms,
                        kind,
                        window,
                        fields,
                    });
                    while self.events.len() > KEPT_EVENTS {
                        self.events.pop_front();
                    }
                }
                Event::Log {
                    level,
                    message,
                    timestamp_ms,
                } => {
                    self.log.push_back(LogRow {
                        time_ms: timestamp_ms,
                        level,
                        message,
                    });
                    while self.log.len() > KEPT_LOG {
                        self.log.pop_front();
                    }
                }
                Event::Dispatched { command, error } => match error {
                    None => self.toast(Tone::Note, format!("Done: {command}")),
                    Some(error) => self.toast(Tone::Bad, format!("{command}: {error}")),
                },
                Event::State(state) => self.state = Some(state),
                Event::Graph(event) => self.graph.take_in(event),
            }
        }
    }

    fn remote_configuration(&mut self, config: Config, revision: u64) {
        match &mut self.session {
            None => {
                self.document.replace(&config);
                self.session = Some(Session::new(self.client_id.clone(), config, revision));
                self.forget_changes();
            }
            Some(session) => match session.remote_fetched(config, revision) {
                Fetched::UpToDate => {}
                Fetched::FastForwarded => {
                    let config = session.config().clone();
                    self.document.replace(&config);
                    self.forget_changes();
                    // Said in the log too (a check reads it there).
                    log::info!(
                        "The configuration changed in the window manager (revision {revision}): showing it"
                    );
                    self.toast(
                        Tone::Note,
                        "The configuration was changed elsewhere; showing the new one.",
                    );
                }
                // The banner asks.
                Fetched::Conflict => self.live = false,
            },
        }
    }

    /// The document is what the window manager has: nothing to send.
    fn forget_changes(&mut self) {
        self.seen_generation = self.document.generation();
        self.sent_generation = self.document.generation();
    }

    /// After the page was drawn: tell the session what the document is
    /// now, and send it when edits are live.
    fn follow_edits(&mut self) {
        let generation = self.document.generation();
        if generation != self.seen_generation {
            self.seen_generation = generation;
            self.changed_at = Instant::now();
            if let (Some(session), Some(config)) = (&mut self.session, self.document.config()) {
                if serde_json::to_value(session.base()).ok().as_ref() == Some(self.document.value())
                {
                    session.discard_draft();
                } else {
                    session.set_draft(config.clone());
                }
            }
        }
        let waiting = generation != self.sent_generation;
        if self.live && waiting && !self.applying && self.changed_at.elapsed() >= LIVE_PAUSE {
            self.apply();
        }
    }

    fn can_apply(&self) -> bool {
        let conflict = self.session.as_ref().is_some_and(Session::has_conflict);
        self.document.config().is_some()
            && !self.applying
            && !conflict
            && self.dirty()
            && self.file_problem.is_none()
    }

    fn apply(&mut self) {
        let Some(config) = self.document.config().cloned() else {
            return;
        };
        if self.file_problem.is_some() {
            return;
        }
        if let Some(path) = self.file.clone() {
            match config.save(&path) {
                Ok(()) => {
                    self.document.replace(&config);
                    self.forget_changes();
                    // The file is what History compares with: read it anew.
                    self.base = None;
                    self.toast(Tone::Good, format!("Saved {}", path.display()));
                }
                Err(error) => self.toast(Tone::Bad, format!("Not saved: {error}")),
            }
            return;
        }
        let Some(request) = self.session.as_ref().and_then(Session::begin_apply) else {
            self.sent_generation = self.document.generation();
            return;
        };
        self.sent_generation = self.document.generation();
        self.applying = true;
        self.link.send(Request::Apply {
            config: Box::new(request.config),
            base_revision: request.base_revision,
            client_id: request.client_id,
        });
    }

    /// Apply the draft on trial: what the window manager runs now is kept
    /// to go back to.
    fn begin_trial(&mut self) {
        let Some(before) = self.session.as_ref().map(|session| session.base().clone()) else {
            return;
        };
        self.live = false;
        self.apply();
        if self.applying {
            // (Should this program fall over while the trial runs, the
            // configuration from before is sent all the same.)
            let tell = self.link.teller();
            let client_id = self.client_id.clone();
            let back = before.clone();
            crate::tryout::revert_on_panic(move || {
                tell(Request::Apply {
                    config: Box::new(back.clone()),
                    base_revision: 0,
                    client_id: client_id.clone(),
                });
            });
            self.try_out = Some(TryOut::begin(before, Instant::now()));
        }
    }

    /// The trial ends: kept, the configuration simply stays. Not kept, the
    /// one from before is applied again. The window then shows that one;
    /// what was tried is one Undo away, as a draft.
    fn end_trial(&mut self, keep: bool) {
        let Some(trial) = self.try_out.take() else {
            return;
        };
        crate::tryout::forget_revert();
        if keep {
            // A configuration that changes the displays is on trial at
            // the window manager too, with a time of its own: kept here,
            // it is kept there. (Seen on the real displays, 2026-11-15:
            // kept in the studio, the display went back all the same.)
            if self.trial.take().is_some() {
                self.link.send(Request::Dispatch(
                    r#"{"command":"output_trial","answer":"keep"}"#.to_string(),
                ));
            }
            self.toast(Tone::Good, "Kept.");
            return;
        }
        let before = trial.before().clone();
        self.document.load(&before);
        if let Some(session) = &mut self.session {
            if serde_json::to_value(session.base()).ok().as_ref() == Some(self.document.value()) {
                session.discard_draft();
            } else {
                session.set_draft(before);
            }
        }
        self.seen_generation = self.document.generation();
        self.apply();
        self.toast(
            Tone::Note,
            "The configuration from before is back. Undo shows what you tried again, as a draft.",
        );
    }

    /// Every frame: a trial whose time is up goes back by itself, once the
    /// window manager has answered the apply that began it.
    fn follow_trial(&mut self, now: Instant) {
        if !self.applying && self.try_out.as_ref().is_some_and(|trial| trial.over(now)) {
            self.end_trial(false);
        }
    }

    fn discard(&mut self) {
        if let Some(session) = &mut self.session {
            session.discard_draft();
            let config = session.config().clone();
            self.document.replace(&config);
            self.forget_changes();
        }
    }

    // --- drawing -----------------------------------------------------

    fn palette(&mut self, context: &egui::Context) -> Palette {
        if !self.fonts_ready {
            theme::install_fonts(context, self.appearance.system_font);
            self.fonts_ready = true;
        }
        let system = context.system_theme().unwrap_or(egui::Theme::Dark);
        // The accent follows the desktop: the colour around the focused
        // window. Where the running desktop takes that from its wallpaper
        // (its state names it then), that one.
        let from_wallpaper = self
            .state
            .as_ref()
            .and_then(|state| state["accent_rgb"].as_u64())
            .map(|colour| ((colour >> 16) as u8, (colour >> 8) as u8, colour as u8));
        let ring = from_wallpaper
            .or_else(|| {
                self.document
                    .config()
                    .map(|config| config.desktop.appearance.active_border_color)
            })
            .map_or(Color32::from_rgb(64, 224, 208), |(red, green, blue)| {
                Color32::from_rgb(red, green, blue)
            });
        // Where Windows has a contrast theme on, the studio takes its
        // own high-contrast colours, as the shell's surfaces do. (The
        // toolkit says of such a theme that it is a light one, whatever
        // it is: seen with "High Contrast Black", 2026-11-15.)
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut chosen = self.appearance;
        #[cfg(windows)]
        if !chosen.preset.is_high_contrast() && gevrek_system::look::high_contrast() {
            chosen.preset = theme::Preset::HighContrastDark;
        }
        let palette = chosen.resolve(system, ring);
        // Nothing moves by itself here either when the desktop is asked
        // to keep still (`animations.reduce_motion`).
        let mut look = chosen;
        look.reduce_motion = self
            .document
            .config()
            .is_some_and(|config| config.desktop.animations.reduce_motion);
        // Or when Windows' own setting asks every program to (on Linux
        // gevrek's setting is the desktop's).
        #[cfg(windows)]
        {
            look.reduce_motion |= gevrek_system::look::reduce_motion();
        }
        theme::apply(context, &palette, &look);
        palette
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        use egui::{Key, Modifiers};
        let pressed = |modifiers: Modifiers, key: Key| {
            context.input_mut(|input| input.consume_key(modifiers, key))
        };
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || pressed(Modifiers::COMMAND, Key::Y)
        {
            self.document.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            self.document.undo();
        }
        if pressed(Modifiers::COMMAND, Key::K) {
            self.search = Some(String::new());
        }
        if pressed(Modifiers::COMMAND, Key::S) && self.can_apply() {
            self.apply();
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, palette: &Palette) {
        // The bar is the window's own title bar (the window has none of
        // the system's): held where nothing else is, it moves the
        // window; pressed twice, the window fills the display or is
        // itself again. (Under what stands on it: those come first.)
        let context = ui.ctx().clone();
        let maximized = context.input(|input| input.viewport().maximized.unwrap_or(false));
        let grip = ui.interact(
            ui.max_rect(),
            ui.scope_id().with("the window's own title bar"),
            egui::Sense::click_and_drag(),
        );
        if grip.double_clicked() {
            context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if grip.drag_started_by(egui::PointerButton::Primary) {
            context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            pointer_taken(&context);
        }
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            // The mark, then the name (lowercase, as the brand writes it).
            gevrek_ui::brand::show_mark(ui, 18.0, palette.chrome);
            ui.label(
                RichText::new("gevrek")
                    .strong()
                    .size(16.0)
                    .color(palette.text),
            );
            ui.label(RichText::new("studio").size(16.0).color(palette.muted));
            ui.add_space(10.0);
            // Who is on the other end.
            match (&self.peer, &self.file) {
                (Some(peer), _) => {
                    let backend = match peer.backend.as_str() {
                        "smithay" => "Linux",
                        "win32" => "Windows",
                        other => other,
                    };
                    widgets::pill(
                        ui,
                        icons::PLUGS_CONNECTED,
                        &format!("{backend} · {}", peer.version),
                        palette.ok,
                    )
                    .on_hover_text(format!("The window manager at {}", peer.address));
                }
                (None, Some(path)) => {
                    widgets::pill(ui, icons::FILE, &file_name(path), palette.blue).on_hover_text(
                        format!(
                            "Editing the file {}; no window manager is asked",
                            path.display()
                        ),
                    );
                }
                (None, None) => {
                    widgets::pill(ui, icons::PLUGS, "not connected", palette.error)
                        .on_hover_text(format!("{}: {}", self.address, self.line_down));
                }
            }
            // What the configuration's own check says.
            let (errors, warnings) = self.problem_counts();
            let check = if self.document.error().is_some() {
                widgets::pill(ui, icons::X_CIRCLE, "not read", palette.error)
            } else if errors > 0 {
                widgets::pill(
                    ui,
                    icons::X_CIRCLE,
                    &format!("{errors} to fix"),
                    palette.error,
                )
            } else if warnings > 0 {
                widgets::pill(
                    ui,
                    icons::WARNING,
                    &format!("{warnings} to look at"),
                    palette.warn,
                )
            } else {
                widgets::pill(ui, icons::CHECK_CIRCLE, "valid", palette.ok)
            };
            if check
                .on_hover_text("What the configuration's own check says. Click for the list.")
                .clicked()
            {
                self.page = Page::Problems;
            }
            // Whether the document is what runs.
            if self.applying {
                widgets::pill(ui, icons::ARROWS_CLOCKWISE, "applying", palette.blue);
            } else if self.dirty() {
                widgets::pill(ui, icons::PENCIL_SIMPLE, "draft", palette.warn)
                    .on_hover_text("There are changes the window manager does not run yet");
            } else if self.connected() || self.file.is_some() {
                widgets::pill(
                    ui,
                    icons::CHECK,
                    if self.file.is_some() {
                        "saved"
                    } else {
                        "applied"
                    },
                    palette.muted,
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(2.0);
                // The window's own buttons, at the bar's end.
                use gevrek_ui::brand_icons::BrandIcon;
                if widgets::brand_icon_button(ui, BrandIcon::Close, icons::X, "Close").clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let (sign, or, says) = if maximized {
                    (BrandIcon::Restore, icons::CORNERS_IN, "As large as it was")
                } else {
                    (BrandIcon::Maximize, icons::CORNERS_OUT, "Fill the display")
                };
                if widgets::brand_icon_button(ui, sign, or, says).clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if widgets::brand_icon_button(ui, BrandIcon::Minimize, icons::MINUS, "Put away")
                    .clicked()
                {
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.add_space(6.0);
                if widgets::icon_button(ui, icons::PAINT_BRUSH, "How this window looks").clicked() {
                    self.appearance_open = !self.appearance_open;
                }
                self.sandbox_button(ui);
                let label = if self.file.is_some() { "Save" } else { "Apply" };
                if widgets::primary_button(ui, self.can_apply(), label)
                    .on_hover_text("Ctrl+S")
                    .clicked()
                {
                    self.apply();
                }
                if self.file.is_none() {
                    if ui
                        .add_enabled(
                            self.can_apply() && self.try_out.is_none() && self.connected(),
                            egui::Button::new("Try 15 s"),
                        )
                        .on_hover_text(
                            "Apply it for 15 seconds. Unless you then keep it, what ran before comes back by itself: for a change that might take away the means to undo it.",
                        )
                        .clicked()
                    {
                        self.begin_trial();
                    }
                    if self.dirty()
                        && ui
                            .button("Discard")
                            .on_hover_text("Back to what the window manager runs")
                            .clicked()
                    {
                        self.discard();
                    }
                    let mut live = self.live;
                    ui.label("Live")
                        .on_hover_text("Send every change to the window manager as it is made");
                    if widgets::switch(ui, &mut live).changed() {
                        self.live = live && self.connected();
                    }
                }
                ui.add_enabled_ui(self.document.can_redo(), |ui| {
                    if widgets::icon_button(ui, icons::ARROW_CLOCKWISE, "Redo (Ctrl+Shift+Z)")
                        .clicked()
                    {
                        self.document.redo();
                    }
                });
                ui.add_enabled_ui(self.document.can_undo(), |ui| {
                    if widgets::icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Undo (Ctrl+Z)")
                        .clicked()
                    {
                        self.document.undo();
                    }
                });
                if ui
                    .button(format!("{}  Search   Ctrl+K", icons::MAGNIFYING_GLASS))
                    .on_hover_text("Find a page or a setting")
                    .clicked()
                {
                    self.search = Some(String::new());
                }
            });
        });
    }

    /// Start or stop the nested gevrek that runs the draft.
    fn sandbox_button(&mut self, ui: &mut Ui) {
        if let Some(reason) = crate::sandbox::unavailable() {
            ui.add_enabled(false, egui::Button::new("Sandbox"))
                .on_disabled_hover_text(reason);
            return;
        }
        let running = self.sandbox.is_some();
        let button = ui.add(egui::Button::new(format!("{}  Sandbox", icons::CUBE)).selected(running)).on_hover_text(
            "Run what is in this window in a gevrek of its own, in a window. Your desktop is not touched until you apply.",
        );
        if !button.clicked() {
            return;
        }
        match self.sandbox.take() {
            Some(sandbox) => sandbox.stop(),
            None => match self.document.config() {
                Some(config) => {
                    match crate::sandbox::Sandbox::start(config, self.document.generation()) {
                        Ok(sandbox) => {
                            self.toast(Tone::Note, "The sandbox follows what you change here. Close its window, or press the button again, to end it.");
                            self.sandbox = Some(sandbox);
                        }
                        Err(error) => self.toast(Tone::Bad, error),
                    }
                }
                None => self.toast(
                    Tone::Warn,
                    "The configuration is not valid as it is; the sandbox needs one that is.",
                ),
            },
        }
    }

    /// Keep the sandbox in step with the document, and notice its end.
    fn follow_sandbox(&mut self) {
        let Some(sandbox) = &mut self.sandbox else {
            return;
        };
        if !sandbox.alive() {
            if let Some(sandbox) = self.sandbox.take() {
                sandbox.stop();
            }
            return;
        }
        if let Some(config) = self.document.config() {
            sandbox.mirror(config, self.document.generation());
        }
    }

    /// What is wrong in the automation graph of the draft: wires the
    /// window manager would take out when it loads it.
    fn graph_problems(&self) -> Vec<gevrek_model::graph::GraphProblem> {
        self.document
            .config()
            .map(|config| config.node_graph.problems())
            .unwrap_or_default()
    }

    fn problem_counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warnings = self.graph_problems().len();
        for issue in self.document.issues() {
            match issue.severity {
                ConfigIssueSeverity::Error => errors += 1,
                ConfigIssueSeverity::Warning => warnings += 1,
            }
        }
        (errors, warnings)
    }

    fn navigation(&mut self, ui: &mut Ui, palette: &Palette) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);
                let mut group = "";
                for page in Page::ALL {
                    if page.group() != group {
                        group = page.group();
                        ui.add_space(8.0);
                        widgets::caption(ui, group);
                    }
                    let selected = self.page == page;
                    // Problems under the page's settings show on its entry.
                    let (mut errors, mut warnings) = (0, 0);
                    for place in page.places() {
                        let (more_errors, more_warnings) = self.document.issues_under(&place);
                        errors += more_errors;
                        warnings += more_warnings;
                    }
                    if page == Page::Problems {
                        (errors, warnings) = self.problem_counts();
                    }
                    // The graph has no settings of its own on a page: what
                    // is wrong in it shows on its entry.
                    if page == Page::Graph {
                        warnings += self.graph_problems().len();
                    }
                    let text = format!("{}   {}", page.icon(), page.title());
                    let response = ui
                        .with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                            ui.selectable_label(selected, RichText::new(text).size(14.0))
                        })
                        .inner;
                    if errors + warnings > 0 {
                        let colour = if errors > 0 {
                            palette.error
                        } else {
                            palette.warn
                        };
                        let at = egui::pos2(response.rect.right() - 14.0, response.rect.center().y);
                        ui.painter().circle_filled(at, 4.0, colour);
                    }
                    if response.clicked() {
                        self.page = page;
                    }
                }
                ui.add_space(12.0);
            });
    }

    fn banners(&mut self, ui: &mut Ui, palette: &Palette) {
        if let Some(problem) = self.fetch_problem.clone() {
            let mut again = false;
            banner(ui, palette.error, icons::WARNING, |ui| {
                ui.label(format!(
                    "The window manager answers, and its configuration could not be read. What is shown here is gevrek's defaults, not what runs, and nothing can be applied. ({problem})"
                ));
                again = widgets::secondary_button(ui, "Ask again").clicked();
            });
            if again {
                self.fetch_problem = None;
                self.link.send(Request::Fetch);
            }
        }
        if let Some(problem) = &self.file_problem {
            banner(ui, palette.error, icons::WARNING, |ui| {
                ui.label(format!(
                    "The file could not be read, and what is shown here is gevrek's defaults. Nothing is saved, so that the file is not written over. Put the file right and open it again. ({problem})"
                ));
            });
        }
        if let Some(until) = self.trial {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.trial = None;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
                let mut answer = None;
                banner(ui, palette.accent_fg(), icons::MONITOR, |ui| {
                    ui.label(format!(
                        "The displays have new settings. Unless you keep them they go back in {} seconds.",
                        left.as_secs() + 1
                    ));
                    if widgets::secondary_button(ui, "Keep").clicked() {
                        answer = Some("keep");
                    }
                    if widgets::secondary_button(ui, "Go back now").clicked() {
                        answer = Some("revert");
                    }
                });
                if let Some(answer) = answer {
                    self.link.send(Request::Dispatch(format!(
                        r#"{{"command":"output_trial","answer":"{answer}"}}"#
                    )));
                    self.trial = None;
                }
            }
        }
        if let Some(trial) = &self.try_out {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
            let left = trial.seconds_left(Instant::now());
            let mut keep = None;
            banner(ui, palette.accent_fg(), icons::CLOCK_COUNTDOWN, |ui| {
                ui.label(format!(
                    "This configuration is on trial. Unless you keep it, the one from before comes back in {left} seconds."
                ));
                if widgets::secondary_button(ui, "Keep").clicked() {
                    keep = Some(true);
                }
                if widgets::secondary_button(ui, "Go back now").clicked() {
                    keep = Some(false);
                }
            });
            match keep {
                // Going back waits for the answer to the apply under way.
                Some(false) if self.applying => {
                    if let Some(trial) = &mut self.try_out {
                        trial.cut_short(Instant::now());
                    }
                }
                Some(keep) => self.end_trial(keep),
                None => {}
            }
        }
        if self.session.as_ref().is_some_and(Session::has_conflict) {
            banner(ui, palette.warn, icons::GIT_MERGE, |ui| {
                ui.label("The configuration was changed elsewhere while you were editing.");
                if ui
                    .button("Keep mine")
                    .on_hover_text("Your edits stay; Apply then replaces the other change")
                    .clicked()
                {
                    if let Some(session) = &mut self.session {
                        session.keep_mine();
                    }
                }
                if ui
                    .button("Use theirs")
                    .on_hover_text("Your edits are dropped")
                    .clicked()
                {
                    if let Some(session) = &mut self.session {
                        session.take_theirs();
                        let config = session.config().clone();
                        self.document.replace(&config);
                    }
                    self.forget_changes();
                }
            });
        }
        if let Some(error) = self.document.error() {
            let error = error.to_string();
            banner(ui, palette.error, icons::X_CIRCLE, |ui| {
                ui.label(format!("This is not a configuration gevrek reads: {error}"));
                if widgets::secondary_button(ui, "Undo").clicked() {
                    self.document.undo();
                }
            });
        }
        if self.file.is_none() && !self.connected() && self.session.is_none() {
            let reason = self.line_down.clone();
            banner(ui, palette.blue, icons::PLUGS, |ui| {
                ui.label(format!(
                    "No window manager answers at {} ({reason}). What you see are gevrek's defaults; it is tried again every few seconds.",
                    self.address
                ));
            });
        }
    }

    fn page_body(&mut self, ui: &mut Ui, palette: &Palette) {
        let page = self.page;
        if page != Page::Raw {
            widgets::heading(ui, page.title(), page.subtitle());
        }
        let highlight = self
            .highlight
            .as_ref()
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(pointer, _)| pointer.clone());
        if self.highlight.is_some() && highlight.is_none() {
            self.highlight = None;
        }
        match page {
            Page::Overview => self.overview(ui, palette),
            Page::Commands => self.commands(ui, palette),
            Page::Bindings => {
                let wish = self
                    .bindings
                    .show(ui, &mut self.document, highlight.as_deref());
                if let Some(crate::bindings::Wish::Run(command)) = wish {
                    self.link.send(Request::Dispatch(command));
                }
            }
            Page::Graph => {
                // (The open windows: what a node's pattern is tried on.)
                self.ask_state();
                self.graph.take_state(self.state.as_ref());
                let features = self.peer.as_ref().map(|peer| peer.features.as_slice());
                for request in self.graph.show(ui, &mut self.document, features) {
                    self.link.send(request);
                }
            }
            Page::Windows => self.windows(ui, palette),
            Page::Events => self.events_page(ui, palette),
            Page::Log => self.log_page(ui, palette),
            Page::Raw => self.raw_page(ui, palette),
            Page::History => {
                // What the edits are compared with: what the window
                // manager runs, read again when its revision changes.
                let revision = self.session.as_ref().map_or(0, Session::base_revision);
                if self
                    .base
                    .as_ref()
                    .is_none_or(|(known, _)| *known != revision)
                {
                    let running = match (&self.session, &self.file) {
                        (Some(session), _) => serde_json::to_value(session.base()).ok(),
                        (None, Some(path)) => Config::load(path)
                            .ok()
                            .and_then(|config| serde_json::to_value(config).ok()),
                        (None, None) => None,
                    };
                    self.base = running.map(|value| (revision, value));
                }
                let base = self.base.as_ref().map(|(_, value)| value);
                self.history.show(ui, &mut self.document, base);
            }
            Page::Problems => self.problems(ui, palette),
            Page::Diagnostics => self.diagnostics(ui, palette),
            _ => {
                if page == Page::Appearance {
                    if let Some(config) = self.document.config() {
                        let appearance = config.desktop.appearance.clone();
                        widgets::card(ui, "As it will look", |ui| {
                            self.appearance_preview.show(ui, &appearance);
                        });
                    }
                }
                if page == Page::Displays {
                    self.ask_state();
                    widgets::card(ui, "Where they stand", |ui| {
                        self.arrangement
                            .show(ui, &mut self.document, self.state.as_ref());
                    });
                    widgets::card(ui, "Each display", |ui| {
                        crate::displays::settings(ui, &mut self.document, self.state.as_ref());
                    });
                }
                // The wallpaper on each display, as its fit puts it there.
                if page == Page::Wallpaper {
                    self.ask_state();
                    // What the window manager says of the wallpaper: on
                    // Windows, why the picture named here is not shown.
                    for (title, text) in
                        crate::wallpaper_preview::said_of_it(self.state.as_ref())
                    {
                        let warn = theme::pal(ui.ctx()).warn;
                        widgets::banner(ui, warn, icons::WARNING, |ui| {
                            ui.label(format!("{title}. {text}"));
                        });
                        ui.add_space(6.0);
                    }
                    if let Some(config) = self.document.config() {
                        let wallpaper = config.desktop.wallpaper.clone();
                        widgets::card(ui, "As it will look", |ui| {
                            self.wallpaper_preview
                                .show(ui, &wallpaper, self.state.as_ref());
                        });
                    }
                }
                if page == Page::Animations {
                    if let Some(config) = self.document.config() {
                        let animations = config.desktop.animations.clone();
                        widgets::card(ui, "How they move", |ui| {
                            crate::preview::motion(ui, &animations);
                        });
                        widgets::card(ui, "What it looks like", |ui| {
                            crate::preview::scenes(ui, &animations);
                        });
                    }
                }
                // Where the system keeps the idle times, its own are shown
                // above the settings that are not kept there.
                #[cfg(windows)]
                if page == Page::Session {
                    widgets::card(ui, "Idle times (the system's)", |ui| {
                        crate::system_idle::show(ui);
                    });
                }
                // The panel's modules, placed by dragging (the form below
                // leaves these lists out).
                if page == Page::Bar {
                    widgets::card(ui, "What stands on it", |ui| {
                        crate::panel_modules::show(ui, &mut self.document);
                    });
                }
                // The zones, moved and sized by dragging.
                if page == Page::Zones {
                    widgets::card(ui, "Where they are", |ui| {
                        self.zones_edit.show(ui, &mut self.document);
                    });
                }
                // The keyboard's layouts, chosen by what they are called.
                if page == Page::Input {
                    widgets::card(ui, "Keyboard layouts", |ui| {
                        crate::keyboard_layouts::show(ui, &mut self.document);
                    });
                }
                // The fingers on the touchpad, as the window manager sees
                // them now (asked several times a second).
                if page == Page::Gestures {
                    if self.connected()
                        && self
                            .state_asked
                            .is_none_or(|asked| asked.elapsed() > Duration::from_millis(110))
                    {
                        self.link.send(Request::State);
                        self.state_asked = Some(Instant::now());
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(120));
                    widgets::card(ui, "The fingers now", |ui| {
                        crate::gesture_live::show(ui, self.state.as_ref());
                    });
                }
                // Which open windows each rule holds for now.
                if page == Page::Rules {
                    self.ask_state();
                    ui.ctx().request_repaint_after(Duration::from_millis(1600));
                    widgets::card(ui, "Windows the rules hold for now", |ui| {
                        crate::rules_live::show(ui, &self.document, self.state.as_ref());
                    });
                    // A rule for a window that is pointed at.
                    widgets::card(ui, "A rule for a window on the desktop", |ui| {
                        ui.horizontal(|ui| {
                            let text = if self.picking {
                                "Now press on the window (Escape: none)"
                            } else {
                                "Pick a window by pressing on it"
                            };
                            if ui
                                .add_enabled(self.connected(), egui::Button::new(text))
                                .clicked()
                            {
                                // (Asked again while it is on, it ends.)
                                self.link
                                    .send(Request::Dispatch(r#"{"command":"pick_window"}"#.into()));
                                self.picking = !self.picking;
                            }
                            if !self.connected() {
                                ui.label(
                                    RichText::new("when a window manager is connected")
                                        .color(palette.muted),
                                );
                            }
                        });
                    });
                    // The order they are tried in, changed by dragging.
                    widgets::card(ui, "The order they are tried in", |ui| {
                        crate::reorder::show(
                            ui,
                            &mut self.document,
                            "/desktop/window_rules",
                            |rule| {
                                let count = |key: &str| rule[key].as_array().map_or(0, Vec::len);
                                let (conditions, effects) = (count("predicates"), count("effects"));
                                let plural = |number: usize, word: &str| {
                                    format!("{number} {word}{}", if number == 1 { "" } else { "s" })
                                };
                                crate::reorder::Line {
                                    name: rule["name"]
                                        .as_str()
                                        .filter(|name| !name.is_empty())
                                        .unwrap_or("a rule without a name")
                                        .to_string(),
                                    detail: format!(
                                        "{}, {}",
                                        plural(conditions, "condition"),
                                        plural(effects, "effect")
                                    ),
                                    enabled: rule["enabled"].as_bool().unwrap_or(true),
                                }
                            },
                        );
                    });
                }
                for card in page.cards() {
                    // The displays are set in the cards above, each by
                    // its own controls: no second list of them here.
                    if card.pointer == "/desktop/outputs" {
                        continue;
                    }
                    // A display's scale is set with the display. The list
                    // of scales by name is the older way to say the same:
                    // shown only where a file still has something in it.
                    if card.pointer == "/desktop/platform/wayland"
                        && matches!(card.show, Show::Only(_))
                    {
                        let older = self
                            .document
                            .get("/desktop/platform/wayland/output_scales")
                            .and_then(|scales| scales.as_object())
                            .is_some_and(|scales| !scales.is_empty());
                        widgets::card(ui, card.title, |ui| {
                            let mut form = Form::new(&mut self.document);
                            form.highlight = highlight.as_deref();
                            form.only(ui, card.pointer, &["output_scale"]);
                            if older {
                                ui.label(
                                    RichText::new(
                                        "Scales by a display's name, from an older file. A display's own scale, above, comes first.",
                                    )
                                    .size(12.0)
                                    .color(palette.muted),
                                );
                                form.only(ui, card.pointer, &["output_scales"]);
                            }
                        });
                        continue;
                    }
                    // The night light's settings are a span of the day
                    // and a colour: drawn as those, not as four lines.
                    if card.pointer == "/desktop/night_light" {
                        let connected = self.connected();
                        let asked = widgets::card(ui, card.title, |ui| {
                            self.night_light.show(ui, &mut self.document, connected)
                        });
                        if let Some(command) = asked {
                            self.link.send(Request::Dispatch(command));
                        }
                        continue;
                    }
                    // The workspaces are each display's own: shown display
                    // by display, with what runs of them now.
                    if card.pointer == "/desktop/workspaces" {
                        self.ask_state();
                        let state = self.state.as_ref();
                        widgets::card(ui, card.title, |ui| {
                            self.workspaces.show(ui, &mut self.document, state);
                        });
                        // How a layout places so many windows, below the
                        // list it is chosen in.
                        if let Some(config) = self.document.config().cloned() {
                            widgets::card(ui, "Where the windows go", |ui| {
                                self.layout_preview.show(ui, &config);
                            });
                        }
                        continue;
                    }
                    // The theme's palette and accent are colours: chosen
                    // by looking at them, not by their names.
                    if card.pointer == "/desktop/appearance/theme" {
                        widgets::card(ui, card.title, |ui| {
                            crate::theme_card::show(ui, &mut self.document);
                        });
                        continue;
                    }
                    widgets::card(ui, card.title, |ui| {
                        let mut form = Form::new(&mut self.document);
                        form.highlight = highlight.as_deref();
                        match card.show {
                            Show::Place => {
                                if let Some(node) = gevrek_schema::schema().at(card.pointer) {
                                    form.place(ui, card.pointer, node, card.title, 0);
                                }
                            }
                            Show::AllBut(skip) | Show::Rest(skip) => {
                                form.fields(ui, card.pointer, skip)
                            }
                            Show::Only(names) => form.only(ui, card.pointer, names),
                        }
                    });
                }
            }
        }
    }

    fn overview(&mut self, ui: &mut Ui, palette: &Palette) {
        widgets::card(ui, "Connection", |ui| match (&self.peer, &self.file) {
            (Some(peer), _) => {
                fact(
                    ui,
                    "Window manager",
                    &format!("gevrek {} ({})", peer.version, peer.backend),
                );
                fact(ui, "Address", &peer.address);
                fact(ui, "System", &peer.platform);
                let revision = self.session.as_ref().map_or(0, Session::base_revision);
                fact(ui, "Configuration", &format!("revision {revision}"));
            }
            (None, Some(path)) => fact(ui, "File", &path.display().to_string()),
            (None, None) => {
                ui.colored_label(
                    palette.error,
                    format!("Not connected to {}: {}", self.address, self.line_down),
                );
            }
        });
        // The tutorial: how far the person has come, and the ways to it.
        if self.peer.is_some() {
            let progress = gevrek_tutorial::Progress::load(&gevrek_tutorial::progress::file());
            let (finished, of) = progress.count();
            let shell = |action: &str| {
                Request::Dispatch(
                    serde_json::json!({"command": "shell", "action": action}).to_string(),
                )
            };
            let mut ask = None;
            widgets::card(ui, "Learn gevrek", |ui| {
                ui.label(
                    RichText::new(match (finished, progress.next()) {
                        (_, None) => format!("You have taken all {of} lessons of the tutorial. Each can be taken again."),
                        (0, _) => format!("The tutorial teaches gevrek by doing, on practice windows of its own: {of} short lessons, left at any step."),
                        (_, Some((lesson, _))) => format!("{finished} of {of} lessons taken. Next: {}.", lesson.title),
                    })
                    .color(palette.muted),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let begin = if finished == 0 && progress.left_at.is_none() {
                        "Begin the tutorial"
                    } else {
                        "Go on with the tutorial"
                    };
                    if widgets::primary_button(ui, true, begin).clicked() {
                        ask = Some("tutorial");
                    }
                    if widgets::secondary_button(ui, "The welcome").clicked() {
                        ask = Some("welcome");
                    }
                });
            });
            if let Some(action) = ask {
                self.link.send(shell(action));
            }
        }
        if let Some(config) = self.document.config() {
            let desktop = &config.desktop;
            let counts = [
                ("Key bindings", desktop.bindings.len(), Page::Bindings),
                ("Window rules", desktop.window_rules.len(), Page::Rules),
                ("Workspaces", desktop.workspaces.len(), Page::Workspaces),
                ("Displays set up", desktop.outputs.len(), Page::Displays),
                ("Startup programs", desktop.startup.len(), Page::Startup),
                ("Scratchpads", desktop.scratchpads.len(), Page::Scratchpads),
            ];
            let mut go = None;
            widgets::card(ui, "In this configuration", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (name, count, page) in counts {
                        if widgets::secondary_button(ui, format!("{count}  {name}")).clicked() {
                            go = Some(page);
                        }
                    }
                });
            });
            if let Some(page) = go {
                self.page = page;
            }
        }
        let mut load = None;
        widgets::card(ui, "Start from an example", |ui| {
            ui.label(
                RichText::new("Loading puts the example's look and way of working in this window: its layouts, bindings, panel, colours and rules. What is yours and the machine's stays: keyboard and pointer, displays, session, night light, wallpaper, and your own rules. Nothing runs until you apply, and Undo brings yours back.")
                    .color(palette.muted),
            );
            ui.add_space(6.0);
            for example in gevrek_model::examples::built_in_examples() {
                ui.horizontal(|ui| {
                    if widgets::secondary_button(ui, "Load").clicked() {
                        load = Some(example.config.clone());
                    }
                    ui.label(RichText::new(example.name).strong());
                });
                ui.indent(example.id, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(example.description).color(palette.muted))
                            .wrap(),
                    );
                });
                ui.add_space(4.0);
            }
        });
        if let Some(config) = load {
            // (Over the person's own: their keyboard, displays and rules stay.)
            let config = match self.document.config() {
                Some(own) => gevrek_model::examples::put_on(&config, own),
                None => config,
            };
            self.document.load(&config);
            self.toast(
                Tone::Note,
                "Example loaded. Apply to run it, Undo to take it back.",
            );
        }
    }

    /// Every command the model has, each with a form for what it takes.
    fn commands(&mut self, ui: &mut Ui, palette: &Palette) {
        let Some(node) = gevrek_schema::schema().at(COMMAND) else {
            return;
        };
        let gevrek_schema::Kind::Union(variants) = node.kind() else {
            return;
        };
        let text = serde_json::to_string(&self.command.0).unwrap_or_default();
        widgets::card(ui, "The command", |ui| {
            Form::new(&mut self.command).value(ui, node, "Command");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let run = widgets::primary_button(ui, self.peer.is_some(), "Run now");
                if run
                    .on_disabled_hover_text("No window manager to run it")
                    .clicked()
                {
                    self.link.send(Request::Dispatch(text.clone()));
                }
                if ui
                    .button("Copy")
                    .on_hover_text("As it is written in a binding or sent by a script")
                    .clicked()
                {
                    ui.ctx().copy_text(text.clone());
                }
                if widgets::secondary_button(ui, "Copy for a terminal").clicked() {
                    ui.ctx().copy_text(format!("gevrek dispatch '{text}'"));
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new(&text).monospace().color(palette.muted));
        });
        widgets::card(ui, "All commands", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.command_filter)
                    .hint_text("Find a command")
                    .desired_width(320.0),
            );
            ui.add_space(6.0);
            let needle = self.command_filter.trim().to_lowercase();
            for variant in &variants {
                let name = gevrek_schema::title(variant.name);
                let first_line = variant.description.lines().next().unwrap_or("");
                if !needle.is_empty()
                    && !format!("{name} {} {}", variant.name, variant.description)
                        .to_lowercase()
                        .contains(&needle)
                {
                    continue;
                }
                let chosen = variant.matches(&self.command.0);
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(chosen, RichText::new(name).strong())
                        .clicked()
                    {
                        self.command = Loose(variant.blank());
                    }
                    ui.label(RichText::new(variant.name).monospace().color(palette.faint));
                    ui.add(
                        egui::Label::new(RichText::new(first_line).color(palette.muted)).truncate(),
                    );
                });
            }
        });
    }

    /// Ask the window manager what it has now; again every second and a
    /// half while a page that shows it is open.
    fn ask_state(&mut self) {
        let due = self
            .state_asked
            .is_none_or(|asked| asked.elapsed() > Duration::from_millis(1500));
        if due && self.connected() {
            self.link.send(Request::State);
            self.state_asked = Some(Instant::now());
        }
    }

    fn windows(&mut self, ui: &mut Ui, palette: &Palette) {
        self.ask_state();
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        let Some(state) = &self.state else {
            ui.colored_label(palette.muted, "Nothing known yet.");
            return;
        };
        search_field(
            ui,
            &mut self.windows_filter,
            "Search the windows by title or class",
        );
        let typed = self.windows_filter.clone();
        let mut rule = None;
        for monitor in state["monitors"].as_array().into_iter().flatten() {
            let name = monitor["name"].as_str().unwrap_or("?");
            let size = format!("{}×{}", monitor["size"]["width"], monitor["size"]["height"]);
            widgets::card(ui, &format!("{name}   {size}"), |ui| {
                for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
                    let shown = workspace["id"] == monitor["workspace"]["id"];
                    let windows = workspace["detailed_windows"].as_array().map_or(0, Vec::len);
                    let listed: Vec<&Value> = workspace["detailed_windows"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|window| {
                            inspect::found(
                                &typed,
                                &[
                                    window["title"].as_str().unwrap_or(""),
                                    window["class"].as_str().unwrap_or(""),
                                ],
                            )
                        })
                        .collect();
                    // An empty workspace is only shown when it is the one on
                    // screen, and not at all while something is searched.
                    if listed.is_empty() && (!shown || windows > 0 || !typed.trim().is_empty()) {
                        continue;
                    }
                    let title = format!(
                        "Workspace {}{}   {}",
                        workspace["name"].as_str().unwrap_or("?"),
                        if shown { "  (shown)" } else { "" },
                        workspace["layout"].as_str().unwrap_or("")
                    );
                    ui.label(RichText::new(title).strong());
                    for window in listed {
                        ui.horizontal(|ui| {
                            let focused = window["is_focused"].as_bool().unwrap_or(false);
                            widgets::dot(
                                ui,
                                focused,
                                if focused {
                                    palette.accent
                                } else {
                                    palette.faint
                                },
                            );
                            if widgets::small_icon_button(
                                ui,
                                icons::FUNNEL,
                                "Make a rule for windows like this one",
                            )
                            .clicked()
                            {
                                rule = rule_for(window);
                            }
                            ui.label(window["title"].as_str().unwrap_or(""));
                            ui.label(
                                RichText::new(format!(
                                    "{}   {}×{} at {},{}",
                                    window["class"].as_str().unwrap_or(""),
                                    window["size"]["width"],
                                    window["size"]["height"],
                                    window["position"]["x"],
                                    window["position"]["y"],
                                ))
                                .color(palette.muted),
                            );
                        });
                    }
                    ui.add_space(4.0);
                }
            });
        }
        // A rule made from a window: added to the list, and shown there
        // to say what is to happen to such windows.
        if let Some(rule) = rule {
            self.add_rule(rule);
        }
    }

    /// A rule made from a window, added to the list and shown there.
    fn add_rule(&mut self, rule: Value) {
        let at = self
            .document
            .get("/desktop/window_rules")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        self.document.edit("/desktop/window_rules", |rules| {
            if let Value::Array(rules) = rules {
                rules.push(rule);
            }
        });
        self.page = Page::Rules;
        self.highlight = Some((
            format!("/desktop/window_rules/{at}"),
            Instant::now() + Duration::from_secs(3),
        ));
        self.toast(Tone::Note, "A rule for windows like that one was added at the end of the list. It floats them: change what it does here.");
    }

    /// The window manager says which window was pressed on, after it
    /// was asked to have one picked: a rule for windows like it.
    fn window_picked(&mut self, fields: &[(String, String)]) {
        if !std::mem::take(&mut self.picking) {
            // (Somebody else's asking.)
            return;
        }
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map_or("", |(_, value)| value.as_str())
        };
        match rule_for(&serde_json::json!({"class": field("class"), "title": field("title")})) {
            Some(rule) => self.add_rule(rule),
            None if field("cancelled") == "true" => {
                self.toast(Tone::Note, "No window was picked.");
            }
            None => self.toast(
                Tone::Warn,
                "That window has neither a class nor a title to know it by.",
            ),
        }
    }

    fn events_page(&mut self, ui: &mut Ui, palette: &Palette) {
        let typed = self.events_filter.clone();
        let shown: Vec<&EventRow> = self
            .events
            .iter()
            .filter(|row| {
                let window = row
                    .window
                    .map(|window| format!("window {window}"))
                    .unwrap_or_default();
                inspect::found(&typed, &[&row.kind, &row.fields, &window])
            })
            .collect();
        let mut clear = false;
        ui.horizontal(|ui| {
            search_field(ui, &mut self.events_filter, "Search the events");
            ui.label(counted(shown.len(), self.events.len(), "events"));
            if ui
                .button(format!("{}  Copy", icons::COPY))
                .on_hover_text("The events shown, as text")
                .clicked()
            {
                let text: Vec<String> = shown
                    .iter()
                    .map(|row| inspect::event_line(row.time_ms, &row.kind, row.window, &row.fields))
                    .collect();
                ui.ctx().copy_text(text.join("\n"));
            }
            clear = widgets::secondary_button(ui, "Clear").clicked();
        });
        widgets::card_frame(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for row in &shown {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(clock(row.time_ms))
                                    .monospace()
                                    .color(palette.faint),
                            );
                            ui.label(
                                RichText::new(&row.kind)
                                    .monospace()
                                    .color(palette.accent_fg()),
                            );
                            if let Some(window) = row.window {
                                ui.label(
                                    RichText::new(format!("window {window}")).color(palette.muted),
                                );
                            }
                            ui.label(RichText::new(&row.fields).monospace().color(palette.muted));
                        });
                    }
                });
        });
        if clear {
            self.events.clear();
        }
    }

    fn log_page(&mut self, ui: &mut Ui, palette: &Palette) {
        let typed = self.log_filter.clone();
        let lowest = self.log_lowest;
        let shown: Vec<&LogRow> = self
            .log
            .iter()
            .filter(|row| {
                inspect::at_least(row.level, lowest) && inspect::found(&typed, &[&row.message])
            })
            .collect();
        let mut clear = false;
        ui.horizontal(|ui| {
            search_field(ui, &mut self.log_filter, "Search the log");
            egui::ComboBox::from_id_salt("log-lowest-level")
                .selected_text(format!("from {}", inspect::level_name(self.log_lowest)))
                .show_ui(ui, |ui| {
                    for level in inspect::LEVELS {
                        ui.selectable_value(
                            &mut self.log_lowest,
                            level,
                            format!("from {}", inspect::level_name(level)),
                        );
                    }
                });
            ui.label(counted(shown.len(), self.log.len(), "lines"));
            if ui
                .button(format!("{}  Copy", icons::COPY))
                .on_hover_text("The lines shown, as text")
                .clicked()
            {
                let text: Vec<String> = shown
                    .iter()
                    .map(|row| inspect::log_line(row.time_ms, row.level, &row.message))
                    .collect();
                ui.ctx().copy_text(text.join("\n"));
            }
            clear = widgets::secondary_button(ui, "Clear").clicked();
        });
        widgets::card_frame(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for row in &shown {
                        let colour = match row.level {
                            Level::Error => palette.error,
                            Level::Warning => palette.warn,
                            Level::Info => palette.text,
                            Level::Debug => palette.muted,
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(clock(row.time_ms))
                                    .monospace()
                                    .color(palette.faint),
                            );
                            ui.label(RichText::new(&row.message).monospace().color(colour));
                        });
                    }
                });
        });
        if clear {
            self.log.clear();
        }
    }
    fn raw_page(&mut self, ui: &mut Ui, palette: &Palette) {
        widgets::heading(ui, Page::Raw.title(), Page::Raw.subtitle());
        // The text follows the document until it is typed in.
        if self.raw_generation != self.document.generation() && self.raw_error.is_empty() {
            self.raw = serde_json::to_string_pretty(self.document.value()).unwrap_or_default();
            self.raw_generation = self.document.generation();
        }
        ui.horizontal(|ui| {
            if ui
                .button("Take the text")
                .on_hover_text("Read the text as the configuration (it can be undone)")
                .clicked()
            {
                match serde_json::from_str::<Config>(&self.raw) {
                    Ok(config) => {
                        self.document.load(&config);
                        self.raw_error.clear();
                    }
                    Err(error) => self.raw_error = error.to_string(),
                }
            }
            if widgets::secondary_button(ui, "Back to the document").clicked() {
                self.raw_error.clear();
                self.raw_generation = u64::MAX;
            }
            if !self.raw_error.is_empty() {
                ui.colored_label(palette.error, &self.raw_error);
            }
        });
        // What the configuration's own check says, each with the line it
        // is about. (Of the document: text that was typed in and not yet
        // taken has not been checked.)
        let places = crate::rawtext::lines(&self.raw);
        let mut go_to_line = None;
        if !self.document.issues().is_empty() {
            widgets::card_frame(ui, |ui| {
                for issue in self.document.issues() {
                    let (icon, colour) = match issue.severity {
                        ConfigIssueSeverity::Error => (icons::X_CIRCLE, palette.error),
                        ConfigIssueSeverity::Warning => (icons::WARNING, palette.warn),
                    };
                    let line = crate::rawtext::line_of(&places, &pointer_of(&issue.path));
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(colour, icon);
                        if let Some(line) = line {
                            if ui
                                .link(RichText::new(format!("line {}", line + 1)).monospace())
                                .clicked()
                            {
                                go_to_line = Some(line);
                            }
                        }
                        ui.label(RichText::new(&issue.path).monospace().color(palette.muted));
                        ui.label(&issue.message);
                    });
                }
            });
        }
        // Colours by what each piece of the text is.
        let colours = RawColours {
            key: palette.blue,
            text: palette.ok,
            number: palette.purple,
            word: palette.warn,
            mark: palette.muted,
            other: palette.text,
        };
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = raw_layout(
                text.as_str(),
                &colours,
                egui::TextStyle::Monospace.resolve(ui.style()),
            );
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        // What can be written where the cursor stands, by the
        // configuration's own description of itself: a press writes it,
        // and so does Ctrl+Space for the first.
        let field = egui::Id::unique("raw-configuration");
        let cursor = egui::TextEdit::load_state(ui.ctx(), field)
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index)
            .map(|chars| {
                self.raw
                    .char_indices()
                    .nth(chars.into())
                    .map_or(self.raw.len(), |(byte, _)| byte)
            });
        let standing = cursor.and_then(|at| crate::rawtext::standing(&self.raw, at));
        let offers = standing
            .as_ref()
            .map(crate::rawtext::offers)
            .unwrap_or_default();
        let mut write = None;
        if !offers.is_empty() {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Space)) {
                write = offers.first().map(|(word, _)| word.clone());
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Here:").color(palette.muted));
                for (word, meaning) in offers.iter().take(10) {
                    let button = ui
                        .add(egui::Button::new(RichText::new(word).monospace().size(12.5)).small());
                    let button = if meaning.is_empty() {
                        button
                    } else {
                        button.on_hover_text(meaning)
                    };
                    if button.clicked() {
                        write = Some(word.clone());
                    }
                }
                if offers.len() > 10 {
                    ui.label(
                        RichText::new(format!("and {} more: write on", offers.len() - 10))
                            .color(palette.muted),
                    );
                }
                ui.label(RichText::new("Ctrl+Space writes the first").color(palette.faint));
            });
        }
        let mut cursor_after = None;
        if let (Some(word), Some(at), Some(standing)) = (write, cursor, &standing) {
            let (written, byte) = crate::rawtext::completed(&self.raw, at, standing, &word);
            cursor_after = Some(written[..byte].chars().count());
            self.raw = written;
            if self.raw_error.is_empty() {
                self.raw_error = "edited: \"Take the text\" reads it".to_string();
            }
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.raw)
                        .id(field)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .layouter(&mut layouter),
                );
                if response.changed() && self.raw_error.is_empty() {
                    // Typed in: the text is the person's until it is taken or dropped.
                    self.raw_error = "edited: \"Take the text\" reads it".to_string();
                }
                // After an offer was written: the cursor behind it, and
                // the keys in the text again.
                if let Some(chars) = cursor_after {
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(chars),
                            )));
                        state.store(ui.ctx(), field);
                    }
                    response.request_focus();
                }
                if let Some(line) = go_to_line {
                    // The cursor to the beginning of that line, and the line into view.
                    let at = egui::text::CCursor::new(crate::rawtext::chars_before_line(
                        &self.raw, line,
                    ));
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(at)));
                        state.store(ui.ctx(), field);
                    }
                    response.request_focus();
                    let row = ui.text_style_height(&egui::TextStyle::Monospace);
                    let top = response.rect.top() + row * line as f32;
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            egui::pos2(response.rect.left(), top),
                            egui::vec2(10.0, row),
                        ),
                        Some(egui::Align::Center),
                    );
                }
            });
    }

    fn problems(&mut self, ui: &mut Ui, palette: &Palette) {
        if let Some(error) = self.document.error() {
            widgets::card(ui, "Not read", |ui| {
                ui.colored_label(palette.error, error);
            });
            return;
        }
        let in_the_graph = self.graph_problems();
        if self.document.issues().is_empty() && in_the_graph.is_empty() {
            widgets::card_frame(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(palette.ok, icons::CHECK_CIRCLE);
                    ui.label("Nothing to fix and nothing to look at.");
                });
            });
            return;
        }
        let mut go = None;
        if !in_the_graph.is_empty() {
            let mut open = false;
            widgets::card(ui, "In the automation graph", |ui| {
                for problem in &in_the_graph {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(palette.warn, icons::WARNING);
                        ui.label(&problem.text);
                    });
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(
                            "The window manager leaves these wires out when it loads the graph.",
                        )
                        .color(palette.muted),
                    );
                    open = ui.link("Open the graph").clicked();
                });
            });
            if open {
                // The nodes to look at are the chosen ones there.
                self.graph
                    .select_only(in_the_graph.iter().filter_map(|problem| problem.node));
                self.page = Page::Graph;
            }
        }
        if self.document.issues().is_empty() {
            return;
        }
        widgets::card_frame(ui, |ui| {
            for issue in self.document.issues() {
                let (icon, colour) = match issue.severity {
                    ConfigIssueSeverity::Error => (icons::X_CIRCLE, palette.error),
                    ConfigIssueSeverity::Warning => (icons::WARNING, palette.warn),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(colour, icon);
                    let pointer = pointer_of(&issue.path);
                    if ui.link(RichText::new(&issue.path).monospace()).clicked() {
                        go = Some(pointer);
                    }
                    ui.label(&issue.message);
                });
            }
        });
        if let Some(pointer) = go {
            self.show_setting(&pointer);
        }
    }

    fn diagnostics(&mut self, ui: &mut Ui, palette: &Palette) {
        widgets::card(ui, "This program", |ui| {
            fact(ui, "gevrek-studio", env!("CARGO_PKG_VERSION"));
            fact(
                ui,
                "Settings known",
                &gevrek_schema::catalogue().len().to_string(),
            );
            fact(
                ui,
                "Configuration file",
                &gevrek_model::paths::config_file().display().to_string(),
            );
            fact(
                ui,
                "Its appearance",
                &self.appearance_path.display().to_string(),
            );
        });
        widgets::card(ui, "The window manager", |ui| match &self.peer {
            Some(peer) => {
                fact(ui, "Version", &peer.version);
                fact(ui, "Backend", &peer.backend);
                fact(ui, "System", &peer.platform);
                fact(
                    ui,
                    "Configuration format",
                    &peer.config_schema_version.to_string(),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("What it can do").strong());
                ui.horizontal_wrapped(|ui| {
                    for feature in &peer.features {
                        widgets::pill(ui, icons::CHECK, feature, palette.muted);
                    }
                });
            }
            None => {
                ui.colored_label(palette.muted, "Not connected.");
            }
        });
    }

    /// Go to the page that shows a setting and put a ring around it.
    fn show_setting(&mut self, pointer: &str) {
        // The page is found by the longest place it shows of the pointer.
        let mut place = pointer.to_string();
        loop {
            if let Some(page) = Page::of(&place) {
                self.page = page;
                break;
            }
            match place.rsplit_once('/') {
                Some((parent, _)) if !parent.is_empty() => place = parent.to_string(),
                _ => break,
            }
        }
        self.highlight = Some((pointer.to_string(), Instant::now() + Duration::from_secs(3)));
    }

    fn search_window(&mut self, context: &egui::Context, palette: &Palette) {
        let Some(mut query) = self.search.take() else {
            return;
        };
        let mut keep = true;
        let mut go_page = None;
        let mut go_setting = None;
        egui::Window::new("Search")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 90.0])
            .fixed_size([620.0, 420.0])
            .show(context, |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .hint_text("A page or a setting")
                        .desired_width(f32::INFINITY),
                );
                field.request_focus();
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
                let needle = query.trim().to_lowercase();
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        let mut shown = 0;
                        for page in Page::ALL {
                            if needle.is_empty() || page.title().to_lowercase().contains(&needle) {
                                if ui
                                    .selectable_label(
                                        false,
                                        format!("{}   {}", page.icon(), page.title()),
                                    )
                                    .clicked()
                                {
                                    go_page = Some(page);
                                }
                                shown += 1;
                            }
                        }
                        if needle.len() < 2 {
                            return;
                        }
                        ui.separator();
                        for setting in gevrek_schema::catalogue() {
                            if shown > 60 {
                                break;
                            }
                            let hay = format!(
                                "{} {} {} {}",
                                setting.title,
                                setting.group.join(" "),
                                setting.description,
                                setting.pointer
                            )
                            .to_lowercase();
                            if !needle.split_whitespace().all(|word| hay.contains(word)) {
                                continue;
                            }
                            let Some(page) = Page::of(&setting.pointer) else {
                                continue;
                            };
                            shown += 1;
                            let response = ui.selectable_label(
                                false,
                                format!(
                                    "{}   {}  ›  {}",
                                    page.icon(),
                                    setting.group.join(" › "),
                                    setting.title
                                ),
                            );
                            let first_line = setting.description.lines().next().unwrap_or("");
                            if !first_line.is_empty() {
                                ui.label(
                                    RichText::new(format!("        {first_line}"))
                                        .size(12.0)
                                        .color(palette.muted),
                                );
                            }
                            if response.clicked() {
                                go_setting = Some(setting.pointer.clone());
                            }
                        }
                    });
            });
        if let Some(page) = go_page {
            self.page = page;
            keep = false;
        }
        if let Some(pointer) = go_setting {
            self.show_setting(&pointer);
            keep = false;
        }
        if keep {
            self.search = Some(query);
        }
    }

    fn appearance_window(&mut self, context: &egui::Context) {
        if !self.appearance_open {
            return;
        }
        let mut open = true;
        let before = self.appearance;
        egui::Window::new("How this window looks")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for preset in gevrek_ui::Preset::ALL {
                        if ui
                            .selectable_label(self.appearance.preset == preset, preset.name())
                            .clicked()
                        {
                            self.appearance.preset = preset;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Spacing");
                    let options = [
                        (gevrek_ui::Density::Comfortable, "Comfortable"),
                        (gevrek_ui::Density::Compact, "Compact"),
                    ];
                    if let Some(density) = widgets::segmented(
                        ui,
                        ui.scope_id().with("density"),
                        Some(&self.appearance.density),
                        &options,
                        false,
                    ) {
                        self.appearance.density = density;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Corners");
                    ui.add(widgets::Slider::new(&mut self.appearance.radius, 0..=16).suffix(" px"));
                });
                ui.horizontal(|ui| {
                    ui.label("Accent");
                    let follows = self.appearance.accent == gevrek_ui::Accent::FocusRing;
                    let options = [(true, "The desktop's"), (false, "The palette's")];
                    if let Some(desktop) = widgets::segmented(
                        ui,
                        ui.scope_id().with("accent"),
                        Some(&follows),
                        &options,
                        false,
                    ) {
                        self.appearance.accent = if desktop {
                            gevrek_ui::Accent::FocusRing
                        } else {
                            gevrek_ui::Accent::Theme
                        };
                    }
                });
            });
        self.appearance_open = open;
        if self.appearance != before {
            if let Err(error) = self.appearance.save(&self.appearance_path) {
                log::warn!("the appearance was not saved: {error}");
            }
        }
    }

    fn toasts_overlay(&mut self, context: &egui::Context, palette: &Palette) {
        self.toasts.retain(|toast| toast.since.elapsed() < TOAST);
        if self.toasts.is_empty() {
            return;
        }
        context.request_repaint_after(Duration::from_millis(250));
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .order(egui::Order::Foreground)
            .show(context, |ui| {
                for toast in &self.toasts {
                    let (icon, colour) = match toast.tone {
                        Tone::Good => (icons::CHECK_CIRCLE, palette.ok),
                        Tone::Note => (icons::INFO, palette.blue),
                        Tone::Warn => (icons::WARNING, palette.warn),
                        Tone::Bad => (icons::X_CIRCLE, palette.error),
                    };
                    egui::Frame::new()
                        .fill(palette.surface)
                        .stroke(egui::Stroke::new(1.0, colour))
                        .corner_radius(8)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.set_max_width(420.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(colour, icon);
                                ui.label(&toast.text);
                            });
                        });
                    ui.add_space(6.0);
                }
            });
    }
}

impl eframe::App for Studio {
    fn raw_input_hook(&mut self, context: &egui::Context, input: &mut egui::RawInput) {
        let_go(context, input);
        // Two fingers moved apart or together on the touchpad: the zoom
        // the toolkit would have told of, had it heard them.
        #[cfg(target_os = "linux")]
        if let Some(pinch) = &self.pinch {
            let factor = pinch.take();
            if (factor - 1.0).abs() > 0.0001 {
                input.events.push(egui::Event::Zoom(factor));
            }
        }
    }

    fn on_exit(&mut self) {
        // A trial nobody kept does not outlive the window that would have
        // ended it.
        self.applying = false;
        let on_trial = self.try_out.is_some();
        self.end_trial(false);
        if on_trial {
            // The configuration from before is on its way to the window
            // manager, on the line's own thread: the window does not
            // close (and the program end, the line with it) before it is
            // answered, or a few seconds have passed.
            let until = Instant::now() + Duration::from_secs(4);
            let mut answered = false;
            while !answered && Instant::now() < until {
                answered = self.link.poll().iter().any(|event| {
                    matches!(
                        event,
                        Event::Applied(_)
                            | Event::ApplyFailed(_)
                            | Event::Conflict(_)
                            | Event::Disconnected(_)
                    )
                });
                if !answered {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            if !answered {
                log::warn!("the configuration from before the trial was sent, and not answered in time");
            }
        }
        crate::tryout::forget_revert();
        // The sandbox is this window's; it ends with it.
        if let Some(sandbox) = self.sandbox.take() {
            sandbox.stop();
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        // While the keys of a binding (or of a node that starts at a key)
        // are recorded, every key is the studio's (the window manager's
        // own bindings would run instead).
        #[cfg(any(target_os = "linux", windows))]
        let recording = self.bindings.is_recording() || self.graph.is_recording();
        #[cfg(target_os = "linux")]
        if !std::mem::replace(&mut self.pinch_asked, true) {
            let context = ui.ctx().clone();
            match crate::pinch::Pinch::listen(frame, move || context.request_repaint()) {
                Ok(pinch) => self.pinch = Some(pinch),
                Err(error) => log::info!("pinches are not heard: {error}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            if !recording {
                self.all_keys = None;
                self.bindings.every_key = false;
                self.graph.every_key = false;
            } else {
                if self.all_keys.is_none() && !self.all_keys_refused {
                    match crate::all_keys::AllKeys::take(frame) {
                        Ok(all_keys) => self.all_keys = Some(all_keys),
                        Err(error) => {
                            // Said once; the key alone is recorded then.
                            log::info!("only the key is recorded: {error}");
                            self.all_keys_refused = true;
                        }
                    }
                }
                self.bindings.every_key = self.all_keys.is_some();
                self.graph.every_key = self.all_keys.is_some();
            }
        }
        #[cfg(windows)]
        {
            if !recording {
                self.every_key = None;
                self.bindings.every_key = false;
                self.graph.every_key = false;
            } else {
                if self.every_key.is_none() && !self.every_key_refused {
                    let context = ui.ctx().clone();
                    self.every_key =
                        gevrek_system::every_key::EveryKey::take(move || context.request_repaint());
                    if self.every_key.is_none() {
                        // Said once; the key alone is recorded then.
                        log::info!("only the key is recorded: the keyboard cannot be listened to");
                        self.every_key_refused = true;
                    }
                }
                self.bindings.every_key = self.every_key.is_some();
                self.graph.every_key = self.every_key.is_some();
                if let Some(heard) = self.every_key.as_ref().and_then(|keys| keys.heard()) {
                    // A key without a name in the model is not one a
                    // binding can have: the next one is waited for.
                    if let Some(heard) = crate::bindings::Heard::of_windows(
                        heard.key,
                        heard.win,
                        heard.ctrl,
                        heard.alt,
                        heard.shift,
                    ) {
                        if self.graph.is_recording() {
                            self.graph.heard = Some(heard);
                        } else {
                            self.bindings.heard = Some(heard);
                        }
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = frame;
        self.frame(ui);
    }
}

/// How near the window's edge the pointer makes it larger or smaller,
/// in points.
const EDGE: f32 = 5.0;

/// The edge or corner of a window of this size that a place is at, if
/// it is at one: as the direction the window is resized in from there.
pub fn edge_at(size: egui::Vec2, at: egui::Pos2) -> Option<egui::ResizeDirection> {
    use egui::ResizeDirection as To;
    if at.x < 0.0 || at.y < 0.0 || at.x > size.x || at.y > size.y {
        return None;
    }
    let (west, east) = (at.x <= EDGE, at.x >= size.x - EDGE);
    let (north, south) = (at.y <= EDGE, at.y >= size.y - EDGE);
    Some(match (north, south, west, east) {
        (true, _, true, _) => To::NorthWest,
        (true, _, _, true) => To::NorthEast,
        (_, true, true, _) => To::SouthWest,
        (_, true, _, true) => To::SouthEast,
        (true, ..) => To::North,
        (_, true, ..) => To::South,
        (_, _, true, _) => To::West,
        (_, _, _, true) => To::East,
        _ => return None,
    })
}

/// The window has no frame of the system's: its own edges make it
/// larger and smaller (not while it fills the display).
fn resize_by_the_edges(context: &egui::Context) {
    use egui::ResizeDirection as To;
    let (size, at, pressed, whole) = context.input(|input| {
        (
            input.viewport_rect().size(),
            input.pointer.hover_pos(),
            input.pointer.primary_pressed(),
            input.viewport().maximized.unwrap_or(false)
                || input.viewport().fullscreen.unwrap_or(false),
        )
    });
    let Some(to) = at.filter(|_| !whole).and_then(|at| edge_at(size, at)) else {
        return;
    };
    context.set_cursor_icon(match to {
        To::North | To::South => egui::CursorIcon::ResizeVertical,
        To::East | To::West => egui::CursorIcon::ResizeHorizontal,
        To::NorthWest | To::SouthEast => egui::CursorIcon::ResizeNwSe,
        To::NorthEast | To::SouthWest => egui::CursorIcon::ResizeNeSw,
    });
    if pressed {
        context.send_viewport_cmd(egui::ViewportCommand::BeginResize(to));
        pointer_taken(context);
    }
}

/// Where it is kept that the desktop took the pointer from the window.
fn pointer_taken_id() -> egui::Id {
    egui::Id::unique("the desktop took the pointer from the studio's window")
}

/// The desktop takes the pointer for moving or resizing the window, and
/// the window is not told when the button is let go: the toolkit would
/// go on thinking it held, and the next press on anything would be none.
/// Noted here, and mended at the next frame's start ([`let_go`]).
fn pointer_taken(context: &egui::Context) {
    context.data_mut(|data| data.insert_temp(pointer_taken_id(), true));
}

/// Tell the toolkit that the button is let go, when the desktop took the
/// pointer since the last frame.
fn let_go(context: &egui::Context, input: &mut egui::RawInput) {
    let taken = context
        .data_mut(|data| data.remove_temp::<bool>(pointer_taken_id()))
        .unwrap_or(false);
    if !taken {
        return;
    }
    let at = context
        .input(|input| input.pointer.latest_pos())
        .unwrap_or_default();
    input.events.insert(
        0,
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    );
}

impl Studio {
    /// One frame of the whole window. Apart from `eframe::App::ui` so
    /// that a test can draw the window without a window.
    fn frame(&mut self, ui: &mut Ui) {
        let context = ui.ctx().clone();
        resize_by_the_edges(&context);
        self.take_in();
        self.run_script(&context);
        let palette = self.palette(&context);
        self.shortcuts(&context);

        egui::Panel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ui, |ui| self.top_bar(ui, &palette));
        egui::Panel::left("pages")
            .resizable(false)
            .exact_size(232.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| self.navigation(ui, &palette));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.bg)
                    .inner_margin(egui::Margin::symmetric(24, 8)),
            )
            .show(ui, |ui| {
                self.banners(ui, &palette);
                if self.page == Page::Graph {
                    // The sheet takes the whole room and moves by itself.
                    self.page_body(ui, &palette);
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt(self.page.key())
                    .show(ui, |ui| {
                        ui.set_max_width(980.0);
                        self.page_body(ui, &palette);
                        ui.add_space(24.0);
                    });
            });
        self.search_window(&context, &palette);
        self.appearance_window(&context);
        self.toasts_overlay(&context, &palette);

        self.follow_trial(Instant::now());
        self.follow_edits();
        self.follow_sandbox();
        // A live edit is sent a moment after the last change.
        if self.live && self.document.generation() != self.sent_generation {
            context.request_repaint_after(LIVE_PAUSE);
        }
    }
}

/// A banner over the page (the design system's), with room under it.
fn banner(ui: &mut Ui, colour: Color32, icon: &str, add: impl FnOnce(&mut Ui)) {
    widgets::banner(ui, colour, icon, add);
    ui.add_space(6.0);
}

fn fact(ui: &mut Ui, name: &str, value: &str) {
    let palette = theme::pal(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).color(palette.muted));
        ui.label(value);
    });
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A rule for windows like `window` (as the window manager reports it):
/// by its class where it has one, else by its title. It floats them, as
/// the first thing a rule is usually wanted for; the rules page is where
/// that is changed.
pub fn rule_for(window: &Value) -> Option<Value> {
    let class = window["class"].as_str().unwrap_or("").trim();
    let title = window["title"].as_str().unwrap_or("").trim();
    let (kind, value) = if !class.is_empty() {
        ("Class", class)
    } else if !title.is_empty() {
        ("Title", title)
    } else {
        return None;
    };
    Some(serde_json::json!({
        "name": format!("Windows of {value}"),
        "enabled": true,
        "predicates": [{"kind": kind, "strategy": "Equals", "value": value, "negate": false, "case_sensitive": false}],
        "effects": ["Float"],
    }))
}

/// The place an issue names (`desktop.outputs[1].name`) as a JSON pointer.
fn pointer_of(path: &str) -> String {
    let mut pointer = String::new();
    for part in path.replace('[', ".").replace(']', "").split('.') {
        if !part.is_empty() {
            pointer.push('/');
            pointer.push_str(part);
        }
    }
    pointer
}

/// The field a live list is searched with.
fn search_field(ui: &mut Ui, typed: &mut String, hint: &str) {
    widgets::search_field(ui, typed, hint, 260.0);
}

/// "12 of 340 events", or "340 events" when all are shown.
fn counted(shown: usize, all: usize, what: &str) -> String {
    if shown == all {
        format!("{all} {what}")
    } else {
        format!("{shown} of {all} {what}")
    }
}

/// The colours of the Raw page's text.
struct RawColours {
    key: Color32,
    text: Color32,
    number: Color32,
    word: Color32,
    mark: Color32,
    other: Color32,
}

/// The raw configuration laid out with a colour for each kind of piece.
fn raw_layout(text: &str, colours: &RawColours, font: egui::FontId) -> egui::text::LayoutJob {
    use crate::rawtext::Kind;
    let mut job = egui::text::LayoutJob::default();
    let mut put = |from: usize, to: usize, colour: Color32| {
        if from < to {
            job.append(
                &text[from..to],
                0.0,
                egui::TextFormat::simple(font.clone(), colour),
            );
        }
    };
    let mut at = 0;
    for piece in crate::rawtext::pieces(text) {
        // The white space between two pieces.
        put(at, piece.start, colours.other);
        let colour = match piece.kind {
            Kind::Key => colours.key,
            Kind::Text => colours.text,
            Kind::Number => colours.number,
            Kind::Word => colours.word,
            Kind::Mark => colours.mark,
            Kind::Other => colours.other,
        };
        put(piece.start, piece.end, colour);
        at = piece.end;
    }
    put(at, text.len(), colours.other);
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_edges_make_it_larger_and_smaller() {
        use egui::ResizeDirection as To;
        let size = egui::vec2(800.0, 600.0);
        let at = |x: f32, y: f32| edge_at(size, egui::pos2(x, y));
        assert_eq!(at(2.0, 300.0), Some(To::West));
        assert_eq!(at(798.0, 300.0), Some(To::East));
        assert_eq!(at(400.0, 1.0), Some(To::North));
        assert_eq!(at(400.0, 599.0), Some(To::South));
        assert_eq!(at(1.0, 1.0), Some(To::NorthWest));
        assert_eq!(at(799.0, 598.0), Some(To::SouthEast));
        assert_eq!(at(799.0, 2.0), Some(To::NorthEast));
        assert_eq!(at(3.0, 597.0), Some(To::SouthWest));
        // Inside, and outside: no edge.
        assert_eq!(at(400.0, 300.0), None);
        assert_eq!(at(20.0, 20.0), None);
        assert_eq!(at(-4.0, 300.0), None);
    }

    /// A studio on a file that is not there (gevrek's defaults), with no
    /// window manager: what the window is before anything answers.
    fn studio(context: &egui::Context) -> Studio {
        let nowhere =
            std::env::temp_dir().join(format!("gevrek-studio-test-{}", std::process::id()));
        Studio::new(
            context,
            Options {
                source: Source::File(nowhere.join("config.json")),
                page: Page::Overview,
                appearance: nowhere.join("appearance.json"),
                script: Vec::new(),
            },
        )
    }

    /// Draw one frame of the window at an ordinary size and give back every
    /// piece of text that was drawn.
    fn drawn(context: &egui::Context, studio: &mut Studio) -> Vec<String> {
        fn texts(shape: &egui::Shape, into: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => into.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, into)),
                _ => {}
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| studio.frame(ui));
        // Nothing is put on a screen here: the font pictures are let go.
        output.textures_delta.clear();
        let mut found = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut found);
        }
        found
    }

    #[test]
    fn every_page_draws_with_nothing_running() {
        let context = egui::Context::default();
        let mut studio = studio(&context);
        for page in Page::ALL {
            studio.page = page;
            // Twice: what a page sets up in its first frame it uses in the next.
            drawn(&context, &mut studio);
            let texts = drawn(&context, &mut studio);
            assert!(
                texts.iter().any(|text| text.contains(page.title())),
                "{page:?} does not show its name"
            );
        }
    }

    use gevrek_ui::picture::Painter;

    /// The size of the pictures that are kept of the pages.
    #[cfg(target_os = "linux")]
    const PICTURE: (u32, u32) = (1400, 900);

    /// How much of one picture is another than in the other: the share
    /// of the pixels of which a colour is off by more than a little.
    #[cfg(target_os = "linux")]
    fn differing(a: &[u8], b: &[u8]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 1.0;
        }
        let off = a
            .as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_chunks::<4>().0)
            .filter(|(one, other)| {
                one.iter()
                    .zip(other.iter())
                    .any(|(x, y)| x.abs_diff(*y) > 24)
            })
            .count();
        off as f64 / (a.len() / 4) as f64
    }

    /// Every page as a picture, compared with the one that is kept of
    /// it (`tests/pictures/<page>.png`): what a change does to how a
    /// page looks is seen, and meant or mended. `GEVREK_BLESS=1` keeps
    /// the pictures as they are drawn now.
    /// (The pictures are of the studio on Linux, where a page has what
    /// only Linux has: compared there.)
    #[cfg(target_os = "linux")]
    #[test]
    fn every_page_looks_as_its_picture() {
        let Some(mut painter) = Painter::sized(PICTURE) else {
            eprintln!("no graphics device that computes on the processor: no pictures compared");
            return;
        };
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("pictures");
        let bless = std::env::var_os("GEVREK_BLESS").is_some();
        if bless {
            std::fs::create_dir_all(&folder).expect("the pictures' folder");
        }
        let context = egui::Context::default();
        // (A file at a place that is the same on every machine: a page
        // that names it draws the same everywhere. So are gevrek's own
        // folders, which a page names too: nobody's home is in a
        // picture.)
        let nowhere = std::path::Path::new("/nonexistent/gevrek-studio-pictures");
        for (name, folder) in [
            ("GEVREK_CONFIG_DIR", "config"),
            ("GEVREK_STATE_DIR", "state"),
            ("GEVREK_CACHE_DIR", "cache"),
            ("GEVREK_RUNTIME_DIR", "runtime"),
        ] {
            std::env::set_var(name, nowhere.join(folder));
        }
        let mut studio = Studio::new(
            &context,
            Options {
                source: Source::File(nowhere.join("config.json")),
                page: Page::Overview,
                appearance: nowhere.join("appearance.json"),
                script: Vec::new(),
            },
        );
        let mut other = Vec::new();
        for page in Page::ALL {
            studio.page = page;
            // (A few frames, a tenth of a second apart: what a page sets
            // up in its first frame it uses in the next, and what moves
            // has come to rest.)
            let mut pixels = Vec::new();
            for step in 0..5 {
                pixels = painter.frame_of(
                    &context,
                    10.0 + f64::from(step) * 0.5,
                    Vec::new(),
                    |ui| studio.frame(ui),
                );
            }
            let name = format!("{page:?}").to_lowercase();
            let file = folder.join(format!("{name}.png"));
            if bless {
                image::save_buffer(
                    &file,
                    &pixels,
                    PICTURE.0,
                    PICTURE.1,
                    image::ColorType::Rgba8,
                )
                .expect("a picture is kept");
                continue;
            }
            let kept = match image::open(&file) {
                Ok(kept) => kept.to_rgba8().into_raw(),
                Err(error) => panic!("{name} has no picture ({error}): GEVREK_BLESS=1 keeps one"),
            };
            let share = differing(&pixels, &kept);
            if share > 0.004 {
                // What was drawn is kept beside it, to be looked at.
                let seen = std::env::temp_dir().join(format!("gevrek-studio-{name}.png"));
                let _ = image::save_buffer(
                    &seen,
                    &pixels,
                    PICTURE.0,
                    PICTURE.1,
                    image::ColorType::Rgba8,
                );
                other.push(format!(
                    "{name}: {:.2} % of its picture is another (drawn now: {})",
                    share * 100.0,
                    seen.display()
                ));
            }
        }
        assert!(
            other.is_empty(),
            "pages that do not look as their pictures (meant: GEVREK_BLESS=1):\n{}",
            other.join("\n")
        );
    }

    /// A page drawn into a picture, to be looked at while it is worked
    /// on (no window, no desktop):
    ///
    ///     GEVREK_LOOK=displays GEVREK_LOOK_OUT=page.png \
    ///         cargo test -p gevrek-studio drawn_to_be_looked_at -- --ignored
    ///
    /// `GEVREK_LOOK_SIZE=1400x2400` for a taller picture (a page shows
    /// more of itself), `GEVREK_LOOK_FILE` for a configuration to show,
    /// `GEVREK_LOOK_AT=x,y` for where the pointer rests, and
    /// `GEVREK_LOOK_STATE` for a file with what a window manager says of
    /// itself (`gevrek state --json`),
    /// `GEVREK_LOOK_PRESS=x,y` for a place that is pressed before the
    /// picture is taken (what a press opens is in it). `GEVREK_LOOK=gallery`
    /// draws the design system's widgets instead of a page.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK=<page>"]
    fn a_page_is_drawn_to_be_looked_at() {
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let pair = |text: &str, by: char| -> Option<(f32, f32)> {
            let (one, other) = text.split_once(by)?;
            Some((one.trim().parse().ok()?, other.trim().parse().ok()?))
        };
        let what = said("GEVREK_LOOK").expect("GEVREK_LOOK: a page's key, or gallery");
        let size = said("GEVREK_LOOK_SIZE")
            .and_then(|text| pair(&text, 'x'))
            .map_or((1400, 900), |(width, height)| (width as u32, height as u32));
        let out = std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let at = said("GEVREK_LOOK_AT").and_then(|text| pair(&text, ','));
        let press = said("GEVREK_LOOK_PRESS").and_then(|text| pair(&text, ','));
        let mut painter = Painter::sized(size).expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let file = said("GEVREK_LOOK_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/nonexistent/gevrek-studio-look/config.json"));
        let mut studio = Studio::new(
            &context,
            Options {
                source: Source::File(file),
                page: Page::from_key(&what).unwrap_or(Page::Overview),
                appearance: std::path::PathBuf::from("/nonexistent/gevrek-studio-look/appearance.json"),
                script: Vec::new(),
            },
        );
        // (What a window manager would say of itself, from a file: a
        // page that shows something of it shows that.)
        if let Some(state) = said("GEVREK_LOOK_STATE") {
            let text = std::fs::read_to_string(state).expect("GEVREK_LOOK_STATE: a file of JSON");
            studio.state = Some(serde_json::from_str(&text).expect("GEVREK_LOOK_STATE: JSON"));
        }
        let mut gallery = gevrek_ui::gallery::Gallery::default();
        if what == "gallery" {
            // (The studio sets its look up itself; the gallery alone has
            // nobody to.)
            let settings = gevrek_ui::Settings::default();
            gevrek_ui::theme::install_fonts(&context, false);
            let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
            gevrek_ui::theme::apply(&context, &palette, &settings);
        }
        let mut pixels = Vec::new();
        for step in 0..8 {
            let mut events = Vec::new();
            let point = |(x, y): (f32, f32)| egui::pos2(x, y);
            if let Some(at) = press.or(at) {
                events.push(egui::Event::PointerMoved(point(at)));
            }
            if let (Some(at), 3 | 4) = (press, step) {
                events.push(egui::Event::PointerButton {
                    pos: point(at),
                    button: egui::PointerButton::Primary,
                    pressed: step == 3,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if let (Some(at), 5..) = (at, step) {
                events.push(egui::Event::PointerMoved(point(at)));
            }
            let time = 10.0 + f64::from(step) * 0.5;
            // (What a page reads beside its drawing, a picture's file,
            // has a moment to arrive.)
            std::thread::sleep(std::time::Duration::from_millis(120));
            pixels = if what == "gallery" {
                painter.frame_of(&context, time, events, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| gallery.show(ui));
                    });
                })
            } else {
                painter.frame_of(&context, time, events, |ui| studio.frame(ui))
            };
        }
        image::save_buffer(&out, &pixels, size.0, size.1, image::ColorType::Rgba8)
            .expect("the picture is written");
        println!("drawn: {}", out.display());
    }

    #[test]
    fn a_configuration_on_trial_goes_back_by_itself_unless_it_is_kept() {
        let context = egui::Context::default();
        let size = |studio: &Studio| studio.document.config().unwrap().desktop.focus.history_size;
        // What the window manager would do, without one: answer an apply.
        let answered = |studio: &mut Studio, revision: u64| {
            studio.session.as_mut().unwrap().applied(revision);
            studio.applying = false;
            studio.seen_generation = studio.sent_generation;
        };
        let on_trial = |keep: Option<bool>| -> Studio {
            let mut studio = studio(&context);
            studio.file = None;
            studio.session = Some(Session::new("test", Config::default(), 1));
            studio.document.replace(&Config::default());
            studio.forget_changes();
            let before = size(&studio);
            studio
                .document
                .edit("/desktop/focus/history_size", |value| {
                    *value = serde_json::json!(before + 5)
                });
            studio.follow_edits();
            assert!(studio.can_apply());
            studio.begin_trial();
            assert!(studio.applying && studio.try_out.is_some());
            // Its time is not counted while the answer is on its way.
            studio.follow_trial(Instant::now() + crate::tryout::TRIAL * 2);
            assert!(studio.try_out.is_some());
            answered(&mut studio, 2);
            assert_eq!(size(&studio), before + 5);
            // The banner counts down.
            studio.page = Page::Overview;
            assert!(drawn(&context, &mut studio)
                .iter()
                .any(|text| text.contains("on trial")));
            match keep {
                Some(keep) => studio.end_trial(keep),
                None => studio.follow_trial(Instant::now() + crate::tryout::TRIAL),
            }
            assert!(studio.try_out.is_none());
            studio
        };
        let before = Config::default().desktop.focus.history_size;

        // Nobody answers: what ran before is sent again and shown.
        let mut left_alone = on_trial(None);
        assert!(
            left_alone.applying,
            "the configuration from before is on its way"
        );
        assert_eq!(size(&left_alone), before);
        let sent = left_alone.session.as_ref().unwrap().config().clone();
        assert_eq!(sent.desktop.focus.history_size, before);
        answered(&mut left_alone, 3);
        assert!(!left_alone.dirty());
        // What was tried is one Undo away, as a draft.
        left_alone.document.undo();
        left_alone.follow_edits();
        assert_eq!(size(&left_alone), before + 5);
        assert!(left_alone.dirty());

        // "Go back now" does the same at once.
        let gone_back = on_trial(Some(false));
        assert!(gone_back.applying);
        assert_eq!(size(&gone_back), before);

        // Kept: it stays, and nothing more is sent.
        let kept = on_trial(Some(true));
        assert!(!kept.applying && !kept.dirty());
        assert_eq!(size(&kept), before + 5);
    }

    #[test]
    fn what_is_wrong_in_the_graph_is_listed_and_counted() {
        use gevrek_model::graph::{Connection, LogLevel, NodeGraph, NodeType};
        let context = egui::Context::default();
        let mut studio = studio(&context);
        let mut config = Config::default();
        let mut graph = NodeGraph::new();
        let trigger = graph.add_node(NodeType::ManualTrigger, [0.0, 0.0]);
        let log = graph.add_node(
            NodeType::LogMessage {
                message: String::new(),
                level: LogLevel::Info,
            },
            [100.0, 0.0],
        );
        graph
            .add_connection_by_name(trigger, "Trigger", log, "Trigger")
            .unwrap();
        config.node_graph = graph.clone();
        studio.document.replace(&config);
        let (_, sound) = studio.problem_counts();

        // A wire to a node that is not there.
        let wire = graph.connections[0].clone();
        graph.connections.push(Connection {
            to_node: wire.to_node + 500,
            ..wire
        });
        config.node_graph = graph;
        studio.document.replace(&config);
        assert_eq!(studio.graph_problems().len(), 1);
        assert_eq!(studio.problem_counts().1, sound + 1);
        studio.page = Page::Problems;
        let texts = drawn(&context, &mut studio);
        assert!(texts
            .iter()
            .any(|text| text.contains("In the automation graph")));
        assert!(texts
            .iter()
            .any(|text| text.contains("goes to a node that is not there")));

        // On the sheet the node says what is wrong with its wire.
        studio.page = Page::Graph;
        drawn(&context, &mut studio);
        let sheet = drawn(&context, &mut studio);
        assert!(
            // (Shortened to fit under the node.)
            sheet.iter().any(|text| text.contains("A wire out of")),
            "the node on the sheet does not say it"
        );
        studio.graph.select_only([trigger]);
        assert_eq!(studio.graph.selected().collect::<Vec<_>>(), vec![trigger]);
    }

    #[test]
    fn the_raw_text_is_laid_out_whole_whatever_it_is() {
        let colours = RawColours {
            key: Color32::BLUE,
            text: Color32::GREEN,
            number: Color32::RED,
            word: Color32::YELLOW,
            mark: Color32::GRAY,
            other: Color32::WHITE,
        };
        let font = egui::FontId::monospace(12.0);
        for text in [
            "",
            "{\n  \"a\": [1, true, \"ü\"]\n}\n",
            "not json at all {",
            "  \"open",
        ] {
            let job = raw_layout(text, &colours, font.clone());
            assert_eq!(job.text, text, "nothing is lost or added");
        }
        let job = raw_layout("{\"a\": 1}", &colours, font);
        // In order: the bracket, the name, the colon, the space, the number, the bracket.
        let colours: Vec<Color32> = job
            .sections
            .iter()
            .map(|section| section.format.color)
            .collect();
        assert_eq!(
            colours,
            [
                Color32::GRAY,
                Color32::BLUE,
                Color32::GRAY,
                Color32::WHITE,
                Color32::RED,
                Color32::GRAY
            ]
        );
    }

    #[test]
    fn the_events_and_the_log_show_what_is_searched_for() {
        let context = egui::Context::default();
        let mut studio = studio(&context);
        let shows = |texts: &[String], what: &str| texts.iter().any(|text| text.contains(what));

        studio.events.push_back(EventRow {
            time_ms: 1000,
            kind: "WindowFocused".to_string(),
            window: Some(42),
            fields: "title=Notes".to_string(),
        });
        studio.events.push_back(EventRow {
            time_ms: 2000,
            kind: "WorkspaceChanged".to_string(),
            window: None,
            fields: "name=web".to_string(),
        });
        studio.page = Page::Events;
        let all = drawn(&context, &mut studio);
        assert!(shows(&all, "WindowFocused") && shows(&all, "WorkspaceChanged"));
        assert!(shows(&all, "2 events"));
        studio.events_filter = "notes focused".to_string();
        let some = drawn(&context, &mut studio);
        assert!(shows(&some, "WindowFocused") && !shows(&some, "WorkspaceChanged"));
        assert!(shows(&some, "1 of 2 events"));

        for (level, message) in [
            (Level::Debug, "polled the display"),
            (Level::Warning, "slow poll of the display"),
            (Level::Error, "the hook is gone"),
        ] {
            studio.log.push_back(LogRow {
                time_ms: 3000,
                level,
                message: message.to_string(),
            });
        }
        studio.page = Page::Log;
        let all = drawn(&context, &mut studio);
        assert!(shows(&all, "polled the display") && shows(&all, "the hook is gone"));
        studio.log_lowest = Level::Warning;
        let serious = drawn(&context, &mut studio);
        assert!(!shows(&serious, "polled the display") && shows(&serious, "slow poll"));
        studio.log_filter = "hook".to_string();
        let one = drawn(&context, &mut studio);
        assert!(shows(&one, "the hook is gone") && !shows(&one, "slow poll"));
        assert!(shows(&one, "1 of 3 lines"));
    }

    #[test]
    fn an_issue_path_is_a_pointer() {
        assert_eq!(
            pointer_of("desktop.outputs[1].name"),
            "/desktop/outputs/1/name"
        );
        assert_eq!(pointer_of("desktop.shell.panel"), "/desktop/shell/panel");
    }

    #[test]
    fn a_script_is_read_step_by_step() {
        let steps = Step::parse_all(
            "page bar; set /desktop/shell/panel/height 44 ; live on; apply; wait 200; undo; quit",
        )
        .expect("steps");
        assert_eq!(
            steps,
            [
                Step::Page(Page::Bar),
                Step::Set("/desktop/shell/panel/height".to_string(), Value::from(44)),
                Step::Live(true),
                Step::Apply,
                Step::Wait(200),
                Step::Undo,
                Step::Quit,
            ]
        );
        // A value may have spaces of its own.
        let steps = Step::parse_all(r#"set /desktop/outputs [{"name": "DP-1"}]"#).expect("steps");
        assert_eq!(
            steps,
            [Step::Set(
                "/desktop/outputs".to_string(),
                serde_json::json!([{"name": "DP-1"}])
            )]
        );
        // The keys of a binding and of a node of the graph, by their numbers.
        assert_eq!(
            Step::parse_all("record 2; record-node 7").expect("steps"),
            [Step::Record(2), Step::RecordNode(7)]
        );
        assert!(Step::parse_all("record-node the first").is_err());
        assert!(Step::parse_all("dance").is_err());
        assert!(Step::parse_all("set /a").is_err());
    }

    #[test]
    fn a_rule_made_from_a_window_is_one_the_model_reads() {
        let rule = rule_for(
            &serde_json::json!({"class": "org.mozilla.firefox", "title": "Mozilla Firefox"}),
        )
        .expect("a rule");
        let mut config = Config::default();
        config.desktop.window_rules =
            serde_json::from_value(serde_json::json!([rule])).expect("the model reads it");
        assert_eq!(
            config.desktop.window_rules[0].name,
            "Windows of org.mozilla.firefox"
        );
        assert!(config
            .validate()
            .iter()
            .all(|issue| issue.severity != ConfigIssueSeverity::Error));
        // Without a class, by the title; without either, no rule.
        let by_title =
            rule_for(&serde_json::json!({"class": "", "title": "Calculator"})).expect("a rule");
        assert_eq!(by_title["predicates"][0]["kind"], "Title");
        assert!(rule_for(&serde_json::json!({"class": "", "title": " "})).is_none());
    }

    #[test]
    fn a_time_reads_as_a_clock() {
        assert_eq!(clock(3_723_004), "01:02:03.004");
    }
}
