//! The zones of the desktop, as the shell shows them: containers that
//! hold the desktop's icons, and the icons that are in none.
//!
//! What a zone is and does is `gevrek_zones` (the core); this is what
//! draws it and what hears the pointer. One [`Zones`] holds the core, the
//! system's backend and the file the state is kept in. Each display has a
//! surface below the windows ([`surfaces`]) that is drawn from the core's
//! scene ([`draw`], through [`view`], which is where everything is), and
//! what the pointer does on it becomes commands to the core
//! ([`gesture`]): the same commands a key or another program gives, so
//! nothing can be done with the pointer only.
//!
//! Zones are there when `desktop.zones.enabled` is set, and only then:
//! switched off, no surface is opened and nothing is asked of the system.
//!
//! Three things come from elsewhere, and each has one door here:
//!
//! - the system ([`Desk`]: the backend's trait and the little more a
//!   renderer needs; [`desk`] gives the one of this system);
//! - drags from and to other programs, which are the surfaces' own
//!   (`gevrek_shellkit::dnd`): [`Zones::drag_over`] and
//!   [`Zones::drop_at`] coming in, [`Zones::take_drag_out`] and
//!   [`Zones::drag_ended`] going out;
//! - commands from outside and what is told back, over the line to the
//!   window manager ([`Zones::command`], [`Zones::take_heard`],
//!   [`Zones::core`]; `surfaces::Running::{asked, publish}` carry them).

pub mod backend;
pub mod draw;
pub mod explorer;
pub mod gesture;
pub mod surfaces;
pub mod view;

use egui::Pos2;
use gesture::{Act, Doing, Gestures, Pointer};
use gevrek_system::icons::Picture;
use gevrek_zones::backend::{
    BackendError, BackendEvent, DragActions, DropPayload, FileOperation, IconHandle, ZonesBackend,
};
use gevrek_zones::config::ZonesConfig;
use gevrek_zones::core::{Command, Effect, ZonesCore, ZonesEvent};
use gevrek_zones::host::Host;
use gevrek_zones::store::{LoadReport, Store};
use gevrek_zones::{DisplayId, ItemId, ZoneId};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use view::{Fills, Guide, Layout, Target};

/// The state is written this long after the last change (a drag changes
/// it with every movement of the pointer)...
const SAVE_WHEN_QUIET: Duration = Duration::from_millis(1200);
/// ...and at the latest this long after the first that is not written.
const SAVE_AT_LATEST: Duration = Duration::from_secs(10);

/// Where a display's surface goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    /// The name the surfaces' host knows the display by (the connector's
    /// name on Linux).
    pub output: String,
    /// The display's rectangle in the pixels of the screen (x, y, width,
    /// height): where the surface is put on Windows.
    pub pixels: (i32, i32, i32, i32),
}

/// One line of a menu the shell draws itself for some items, where the
/// system has no menu of its own to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskAction {
    /// What `Desk::act` is told.
    pub id: String,
    pub label: String,
}

/// The system, as far as showing the zones needs it: the backend the
/// core is written against, and the little more that only a renderer
/// asks. `gevrek_system::zones` implements it for each system; the tests
/// implement it over the backend in memory.
pub trait Desk: ZonesBackend {
    /// Call `wake` (from any thread) whenever there is something for
    /// `drain_events`: nothing here looks by the clock. (A backend that
    /// was given its waker when it was made, in [`desk`], has nothing to
    /// do here.)
    fn on_news(&mut self, _wake: Box<dyn Fn() + Send>) {}

    /// The pixels of a picture the backend said is ready.
    fn picture(&mut self, icon: IconHandle) -> Option<Picture>;

    /// Where the surface of a display goes.
    fn spot_of(&mut self, display: &DisplayId) -> Option<Spot>;

    /// The window manager's state is this now: the backend takes the
    /// displays from it (which there are, where, and the room kept).
    fn told(&mut self, _state: &serde_json::Value) {}

    /// The folder the desktop's own files lie in.
    fn desktop(&mut self) -> Option<String> {
        self.desktop_folder()
    }

    /// Put the system's own desktop icons away, after they were written
    /// down (`ZonesBackend::journal_system_icons`); taken back by
    /// `restore_system_icons`. Done where there are none. Only
    /// [`explorer::put_away`] calls this, which holds the order.
    fn hide_system_icons(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    /// The window the zones are drawn in now (Windows: its handle), for
    /// what the system shows on their behalf: its menu for an item, and
    /// how far a copy has come.
    // (Only Windows has such a window to name.)
    #[cfg_attr(not(windows), allow(dead_code))]
    fn set_owner(&mut self, _window: isize) {}

    /// Whether the system's drag carries every item, also what is no file
    /// (This PC, the bin): then every drag of items is handed to it.
    fn drags_everything(&self) -> bool {
        false
    }

    /// Begin the system's drag of these items from a surface, with the
    /// backend's own description of them (which another program reads as
    /// the system's, and we read back exactly). `false`: not here; the
    /// surface begins a drag of the files' paths itself.
    fn begin_drag(
        &mut self,
        _items: &[ItemId],
        _control: &mut gevrek_shellkit::host::Control,
        _from: gevrek_shellkit::host::SurfaceId,
        _image: Option<gevrek_shellkit::dnd::DragImage>,
    ) -> bool {
        false
    }

    /// What was just let go on a surface, read by the backend from the
    /// system's own description of it; `None` where the surface's own
    /// reading (paths) is all there is.
    fn dropped(
        &mut self,
        _control: &mut gevrek_shellkit::host::Control,
        _on: gevrek_shellkit::host::SurfaceId,
        _allowed: DragActions,
    ) -> Option<DropPayload> {
        None
    }

    /// The items a piece of the system's own drag data names (Windows: a
    /// list of shell ids, which is all that things without a path have).
    fn items_named(&mut self, _kind: &str, _bytes: &[u8]) -> Vec<ItemId> {
        Vec::new()
    }

    /// The lines of a menu for these items that the shell draws itself.
    /// `None`: the system shows its own (`ZonesBackend::context_menu`).
    /// A line whose id is [`RENAME`] or [`TRASH`] is carried out by the
    /// zones (as a command, so that it can be taken back), the others by
    /// [`Desk::act`].
    fn own_menu(&mut self, _items: &[ItemId]) -> Option<Vec<DeskAction>> {
        None
    }

    /// Carry out a line of `own_menu`.
    fn act(&mut self, _items: &[ItemId], _action: &str) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }
}

/// A menu's line that gives one item another name.
pub const RENAME: &str = "rename";
/// A menu's line that puts the items into the bin.
pub const TRASH: &str = "trash";

pub use backend::{desk, Starter};

/// How long letters typed one after the other are one beginning of a name.
const TYPED_TOGETHER: Duration = Duration::from_millis(1000);

/// Something the pointer asked for that needs a surface of its own: a
/// menu, or the box a name is typed into. [`surfaces`] opens them.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    ItemMenu { items: Vec<ItemId>, at: Pos2, lines: Vec<DeskAction> },
    ZoneMenu { zone: ZoneId, at: Pos2 },
    DeskMenu { display: DisplayId, at: Pos2 },
    Rename { zone: ZoneId },
    /// The box a file's new name is typed into.
    RenameItem { item: ItemId },
    Filter,
}

/// What a wake-up found to do.
#[derive(Debug, Default)]
pub struct Tick {
    /// The zones changed their part of the configuration: to be written
    /// as the shell writes any configuration.
    pub config: Option<ZonesConfig>,
    /// Something that is drawn changed.
    pub redraw: bool,
}

/// What a drop from outside came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// Taken: items were put into a zone or on the desktop.
    Placed,
    /// Taken: files were moved (`true`) or copied into a portal's folder.
    Filed { moved: bool },
    /// Not taken: nothing of ours is there, or it takes nothing.
    Refused,
}

/// The seconds since 1970, as the core counts time.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// The zones, running.
pub struct Zones {
    core: ZonesCore,
    host: Host,
    desk: Box<dyn Desk>,
    store: Store,
    /// What the pointer and keys are doing, and what is selected.
    pub gestures: Gestures,
    /// How far each zone is scrolled. Not remembered over a restart.
    scrolls: BTreeMap<ZoneId, f32>,
    /// Where everything is, made anew when something changed.
    layout: Layout,
    stale: bool,
    pictures: draw::Pictures,
    /// The lines shown while a zone is moved or sized.
    guides: Vec<Guide>,
    /// What is typed to find items by; those that do not fit are dimmed.
    pub filter: String,
    /// Where the pointer is, on the desktop.
    hover: Option<Pos2>,
    /// The zone whose name is being typed anew.
    pub renaming: Option<ZoneId>,
    /// Whether a drag of files is the system's from its first movement
    /// (an OLE drag, a Wayland one, begun on the surface): what is let go
    /// on a zone then comes back through [`Zones::drop_at`], as from any
    /// other program, and what is let go on another program's window is
    /// that program's. Set where there are real surfaces; without, the
    /// shell follows the pointer itself, on its own surfaces only.
    pub system_drags: bool,
    /// Items of ours that the system is dragging, each with where its
    /// corner was from the pointer.
    outgoing: Vec<(ItemId, egui::Vec2)>,
    /// A drag to be begun by the surface it started on: the items, and
    /// the paths of those that are files.
    drag_out: Option<(Vec<ItemId>, Vec<String>)>,
    /// How the system's own desktop icons stand.
    icons: explorer::SystemIcons,
    /// The configuration wants them put away while zones are shown.
    hide_icons: bool,
    /// The person asked for them back while zones are shown: they stay
    /// until the zones are switched off and on again.
    icons_given_back: bool,
    /// A drag from outside that is over us: where, and whether it is taken.
    incoming: Option<(Pos2, bool)>,
    /// When the state first and last changed without being written.
    unsaved: Option<(Instant, Instant)>,
    config_out: Option<ZonesConfig>,
    /// What happened, for whoever listens from outside.
    heard: Vec<ZonesEvent>,
    /// Asks for a wake-up after a while (to write the state).
    wake_later: Box<dyn Fn(Duration)>,
    /// The displays whose surface was found to have another scale than
    /// the backend says (said once each).
    odd_scales: Vec<DisplayId>,
    /// What was typed to go to an item by the beginning of its name, and
    /// when its last letter came.
    typed: (String, Option<Instant>),
    /// The desktop's own folder, once the backend was asked.
    desktop: Option<Option<String>>,
    /// What needs a surface of its own, until `surfaces` has opened it.
    asks: Vec<Ask>,
    redraw: bool,
}

impl Zones {
    /// Start the zones: read what was remembered from `folder` (the
    /// state's `zones` folder) and, if the configuration has them
    /// switched on, ask the system what there is. `wake` wakes the shell
    /// to call [`Zones::tick`]; it is called from other threads too (the
    /// backend has news, the state is due to be written).
    pub fn start(
        config: &ZonesConfig,
        mut desk: Box<dyn Desk>,
        folder: std::path::PathBuf,
        wake: impl Fn() + Send + Clone + 'static,
        hide_icons: bool,
    ) -> Self {
        let store = Store::new(folder);
        let state = match store.load() {
            Ok(loaded) => {
                match &loaded.report {
                    LoadReport::Fresh | LoadReport::Read => {}
                    LoadReport::Migrated { from } => {
                        log::info!("the zones' state was of layout {from} and is brought up")
                    }
                    LoadReport::Recovered { why, set_aside, backup, .. } => log::warn!(
                        "the zones' state could not be used ({why:?}); it is kept at {set_aside:?}, \
                         and what is used instead comes from {backup:?}"
                    ),
                }
                loaded.state
            }
            Err(error) => {
                log::error!("the zones' state could not be read ({error}); starting without it");
                Default::default()
            }
        };
        let news = wake.clone();
        desk.on_news(Box::new(news));
        // Later: by a thread that sleeps until then and ends.
        let wake_later = move |after: Duration| {
            if after.is_zero() {
                return wake();
            }
            let wake = wake.clone();
            let slept = std::thread::Builder::new().name("gevrek-zones-wake".to_string()).spawn(move || {
                std::thread::sleep(after);
                wake();
            });
            if let Err(error) = slept {
                log::warn!("the zones are not woken to write their state: {error}");
            }
        };
        let mut zones = Self {
            core: ZonesCore::new(config.clone(), state),
            host: Host::new(),
            desk,
            store,
            gestures: Gestures::default(),
            scrolls: BTreeMap::new(),
            layout: Layout::default(),
            stale: true,
            pictures: draw::Pictures::default(),
            guides: Vec::new(),
            filter: String::new(),
            hover: None,
            renaming: None,
            system_drags: false,
            outgoing: Vec::new(),
            drag_out: None,
            icons: explorer::SystemIcons::Theirs,
            hide_icons,
            icons_given_back: false,
            incoming: None,
            unsaved: None,
            config_out: None,
            heard: Vec::new(),
            wake_later: Box::new(wake_later),
            odd_scales: Vec::new(),
            typed: (String::new(), None),
            desktop: None,
            asks: Vec::new(),
            redraw: true,
        };
        // The system's own icons first: no surface is opened before they
        // are safely away (or left alone).
        zones.settle_icons();
        let effects = zones.host.start(&mut zones.core, &mut *zones.desk, now());
        zones.take(effects);
        zones
    }

    /// Make the system's own icons what the configuration wants: away
    /// while zones are shown and it says so, theirs otherwise. The order
    /// of the steps is `explorer`'s.
    fn settle_icons(&mut self) {
        use explorer::SystemIcons;
        let before = self.icons.clone();
        if !self.core.enabled() {
            self.icons_given_back = false;
        }
        let wanted_away = self.core.enabled() && self.hide_icons && !self.icons_given_back;
        if wanted_away {
            if self.icons != SystemIcons::Away {
                self.icons = explorer::put_away(&mut *self.desk);
            }
        } else {
            // (Whatever an earlier run left away is put back here too.)
            match explorer::put_back(&mut *self.desk, &self.icons) {
                Ok(now) => self.icons = now,
                Err(why) => log::error!("{why}"),
            }
        }
        if self.icons != before {
            if let Some(reason) = self.icons.refusal() {
                log::error!("the zones are not shown: {reason}");
                self.heard.push(ZonesEvent::Refused { reason: reason.to_string() });
            }
            self.stale = true;
            self.redraw = true;
            (self.wake_later)(Duration::ZERO);
        }
    }

    /// Whether the configuration wants the system's own icons put away
    /// while zones are shown (`desktop.platform.windows.hide_desktop_icons`).
    pub fn set_hide_icons(&mut self, hide: bool) {
        if self.hide_icons != hide {
            self.hide_icons = hide;
            // (Switched by the person: what they asked back before counts no longer.)
            self.icons_given_back = false;
        }
        self.settle_icons();
    }

    /// Why the zones are not shown although they are switched on.
    pub fn not_shown(&self) -> Option<String> {
        self.icons.refusal().map(str::to_string)
    }

    /// The core, to be read (what is told to other programs is made of it).
    pub fn core(&self) -> &ZonesCore {
        &self.core
    }

    /// The running configuration's part for the zones is this now. What
    /// the zones wrote themselves comes back here and changes nothing.
    pub fn configure(&mut self, config: &ZonesConfig) {
        if self.core.config() == config {
            return;
        }
        let effects = self.host.set_config(&mut self.core, &mut *self.desk, config.clone(), now());
        self.take(effects);
        // Switched on: the system's icons are away before a surface is
        // opened (`spots` gives none until then). Switched off: back.
        self.settle_icons();
        // (What came of the change is in force already: nothing of it is
        // to be written back.)
        self.config_out = None;
    }

    /// The measures of the look in use.
    pub fn set_density(&mut self, density: gevrek_ui::Density) {
        let metrics = view::metrics_for(density);
        if *self.core.metrics() != metrics {
            self.core.set_metrics(metrics);
            self.stale = true;
        }
    }

    /// The scale a display's surface is drawn at. It is the surface's own
    /// that everything is drawn and every picture asked for by; where the
    /// backend says another for the display, zones would lie beside where
    /// the core thinks they are, and that is said (once).
    pub fn surface_scale(&mut self, display: &DisplayId, scale: f32) -> bool {
        let said = self.core.displays().iter().find(|known| &known.id == display).map(|known| known.scale);
        let fits = said.is_none_or(|said| (said - f64::from(scale)).abs() < 0.01);
        if !fits && !self.odd_scales.contains(display) {
            log::warn!(
                "the surface of {display} is drawn at {scale}, and the backend says the display is at {said:?}"
            );
            self.odd_scales.push(display.clone());
        }
        fits
    }

    /// The window manager's state is this: the displays are the backend's
    /// to take from it, and the workspace that is shown (on the display
    /// that has the keyboard) is told to the core.
    pub fn told(&mut self, state: &serde_json::Value, workspace: Option<String>) {
        self.desk.told(state);
        self.workspace(workspace);
        // (What the backend made of it comes as its news.)
        (self.wake_later)(Duration::ZERO);
    }

    /// The desktop's own folder, asked of the backend once.
    fn desktop_folder(&mut self) -> Option<String> {
        if self.desktop.is_none() {
            self.desktop = Some(self.desk.desktop());
        }
        self.desktop.clone().flatten()
    }

    /// Another workspace is shown (for arrangements that differ by workspace).
    pub fn workspace(&mut self, name: Option<String>) {
        let effects = self.core.event(BackendEvent::Workspace { name }, now());
        self.take(effects);
    }

    /// Carry out a command: from a menu here, a key, or another program.
    /// Everything the pointer changes goes through here too.
    pub fn command(&mut self, command: Command) -> Vec<Effect> {
        let effects = self.host.command(&mut self.core, &mut *self.desk, command, now());
        self.take(effects.clone());
        effects
    }

    /// Take in what the core said followed.
    fn take(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            self.stale = true;
            self.redraw = true;
            match effect {
                Effect::Notify(event) => {
                    match &event {
                        ZonesEvent::Refused { reason } => log::info!("the zones did not: {reason}"),
                        ZonesEvent::OperationFailed { operation, error } => {
                            log::warn!("{operation:?} failed: {error}")
                        }
                        // A file that changed has another picture.
                        ZonesEvent::ItemsChanged { .. } => self.pictures.forget_gone(&self.core),
                        _ => {}
                    }
                    self.heard.push(event);
                }
                Effect::SaveConfig => {
                    // (Handed over when the shell next looks: `tick`.)
                    self.config_out = Some(self.core.config().clone());
                    (self.wake_later)(Duration::ZERO);
                }
                Effect::SaveState => {
                    let at = Instant::now();
                    let first = self.unsaved.map_or(at, |(first, _)| first);
                    if self.unsaved.is_none() {
                        (self.wake_later)(SAVE_WHEN_QUIET);
                    }
                    self.unsaved = Some((first, at));
                }
                // (The host carried out what was asked of the backend.)
                Effect::Ask(_) => {}
            }
        }
    }

    /// The shell was woken for the zones: hear what the backend has to
    /// tell, and write the state if it has been quiet long enough.
    pub fn tick(&mut self) -> Tick {
        let effects = self.host.pump(&mut self.core, &mut *self.desk, now());
        self.take(effects);
        for (request, icon) in self.host.take_icons() {
            if self.pictures.answered(request, icon) {
                self.redraw = true;
            }
        }
        if let Some((first, last)) = self.unsaved {
            let at = Instant::now();
            if at.duration_since(last) >= SAVE_WHEN_QUIET || at.duration_since(first) >= SAVE_AT_LATEST {
                self.flush();
            } else {
                (self.wake_later)(SAVE_WHEN_QUIET.saturating_sub(at.duration_since(last)));
            }
        }
        Tick { config: self.config_out.take(), redraw: std::mem::take(&mut self.redraw) }
    }

    /// Write the state now (the shell ends, the zones are switched off).
    pub fn flush(&mut self) {
        if self.unsaved.take().is_some() {
            if let Err(error) = self.store.save(self.core.state()) {
                log::error!("the zones' state could not be written: {error}");
            }
        }
    }

    /// What happened since this was last asked, for whoever listens.
    pub fn take_heard(&mut self) -> Vec<ZonesEvent> {
        std::mem::take(&mut self.heard)
    }

    /// What was asked for that needs a surface of its own.
    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    /// The displays that have a surface, each with where it goes.
    pub fn spots(&mut self) -> Vec<(DisplayId, Spot)> {
        if !self.core.enabled() || !self.icons.lets_zones_show() {
            return Vec::new();
        }
        let displays: Vec<DisplayId> = self.core.displays().iter().map(|display| display.id.clone()).collect();
        displays
            .into_iter()
            .filter_map(|display| {
                let spot = self.desk.spot_of(&display)?;
                Some((display, spot))
            })
            .collect()
    }

    /// Where everything is now.
    pub fn layout(&mut self) -> &Layout {
        if std::mem::take(&mut self.stale) {
            self.layout = Layout::of(&self.core, &self.scrolls, now());
            self.gestures.settle(&self.layout);
        }
        &self.layout
    }

    /// The pointer did something on one of our surfaces (in the
    /// desktop's units).
    pub fn pointer(&mut self, pointer: &Pointer) {
        self.hover = pointer.at;
        self.layout();
        let before = (self.gestures.selected.clone(), self.gestures.band());
        let acts = self.gestures.pointer(&self.layout, pointer);
        // Where the system drags, files are its own from the first
        // movement. (What is no file, This PC or the bin, has no path to
        // hand over: the shell carries it itself, on its own surfaces.)
        if self.system_drags {
            if let Doing::Carrying { items, .. } = &self.gestures.doing {
                let paths: Vec<String> = items
                    .iter()
                    .filter_map(|(item, _)| self.core.item(item).and_then(|item| item.path.clone()))
                    .collect();
                if paths.len() == items.len() || self.desk.drags_everything() {
                    self.outgoing = items.clone();
                    self.drag_out = Some((items.iter().map(|(item, _)| item.clone()).collect(), paths));
                    self.gestures.doing = Doing::Nothing;
                }
            }
        }
        // (For whoever listens: what a gesture did is told when it ends,
        // not a second later with the state.)
        if pointer.released {
            (self.wake_later)(Duration::ZERO);
        }
        if before != (self.gestures.selected.clone(), self.gestures.band()) || self.gestures.busy() {
            self.redraw = true;
        }
        self.perform(acts);
        self.watch_guides();
    }

    /// A key was pressed while a surface of ours has the keyboard.
    pub fn key(&mut self, key: gesture::Key) {
        self.layout();
        // What is the zones' own to do with a key, as a command.
        match key {
            gesture::Key::Delete => {
                let items = self.selection();
                if !items.is_empty() {
                    self.command(Command::TrashItems { items });
                }
                return;
            }
            gesture::Key::Rename => {
                if let [item] = self.selection().as_slice() {
                    self.asks.push(Ask::RenameItem { item: item.clone() });
                    self.redraw = true;
                }
                return;
            }
            _ => {}
        }
        let acts = self.gestures.key(&self.layout, key);
        self.redraw = true;
        self.perform(acts);
    }

    /// What is selected, in the order it is shown.
    pub fn selection(&mut self) -> Vec<ItemId> {
        self.layout();
        self.layout
            .every_item()
            .filter(|shown| self.gestures.selected.contains(&shown.item))
            .map(|shown| shown.item.clone())
            .collect()
    }

    /// Letters were typed while a surface of ours has the keyboard: the
    /// first item whose name begins with what was typed in quick
    /// succession is selected (among those shown with the one the keys
    /// are on, where there is one).
    pub fn typed(&mut self, text: &str) {
        let at = Instant::now();
        if self.typed.1.is_none_or(|last| at.duration_since(last) > TYPED_TOGETHER) {
            self.typed.0.clear();
        }
        self.typed.0.push_str(&text.to_lowercase());
        self.typed.1 = Some(at);
        self.layout();
        let among: Vec<ItemId> = match &self.gestures.focus {
            Some(item) => self.layout.beside(item).iter().map(|shown| shown.item.clone()).collect(),
            None => self.layout.every_item().map(|shown| shown.item.clone()).collect(),
        };
        let begins = |item: &&ItemId| {
            self.core.item(item).is_some_and(|item| item.name.to_lowercase().starts_with(&self.typed.0))
        };
        if let Some(found) = among.iter().find(begins).cloned() {
            self.gestures.selected = std::collections::BTreeSet::from([found.clone()]);
            self.gestures.focus = Some(found);
            self.redraw = true;
        }
    }

    /// The wheel turned over a zone.
    pub fn scroll(&mut self, at: Pos2, by: f32) {
        let zone = match self.layout().hit(at) {
            view::Hit::Item { zone: Some(zone), .. } | view::Hit::Body(zone) => zone,
            _ => return,
        };
        let Some(shown) = self.layout.zone(&zone) else {
            return;
        };
        let to = (shown.scroll - by).clamp(0.0, shown.scroll_extent);
        if to != shown.scroll {
            self.scrolls.insert(zone, to);
            self.stale = true;
            self.redraw = true;
        }
    }

    /// Do what the pointer, a key or a menu asked for.
    pub fn perform(&mut self, acts: Vec<Act>) {
        for act in acts {
            match act {
                Act::Do(command) => {
                    self.command(command);
                }
                Act::Open(items) => {
                    for item in items {
                        if let Err(error) = self.desk.open(&item) {
                            log::warn!("{item} could not be opened: {error}");
                        }
                    }
                }
                Act::ItemMenu { items, at } => match self.desk.own_menu(&items) {
                    Some(lines) => self.asks.push(Ask::ItemMenu { items, at, lines }),
                    None => {
                        let point = gevrek_model::geometry::Point { x: at.x.round() as i32, y: at.y.round() as i32 };
                        if let Err(error) = self.desk.context_menu(&items, point) {
                            log::warn!("the menu could not be shown: {error}");
                        }
                    }
                },
                Act::ZoneMenu { zone, at } => self.asks.push(Ask::ZoneMenu { zone, at }),
                Act::DeskMenu { display, at } => self.asks.push(Ask::DeskMenu { display, at }),
                Act::DragOut(items) => {
                    if let Err(error) = self.desk.start_drag(&items, DragActions::ALL) {
                        log::warn!("the drag could not be handed to the system: {error}");
                    }
                }
            }
        }
    }

    /// Carry out a line of a menu the shell drew for some items.
    pub fn act(&mut self, items: &[ItemId], action: &str) {
        // The two that are commands of the zones, and can be taken back.
        if action == TRASH {
            self.command(Command::TrashItems { items: items.to_vec() });
            return;
        }
        if action == RENAME {
            if let Some(item) = items.first() {
                self.asks.push(Ask::RenameItem { item: item.clone() });
            }
            return;
        }
        if let Err(error) = self.desk.act(items, action) {
            log::warn!("{action} could not be done: {error}");
        }
    }

    /// Ask for the box a zone's name is typed into, or what is looked for.
    pub fn ask(&mut self, ask: Ask) {
        self.asks.push(ask);
    }

    /// The lines to show for the zone that is being moved or sized.
    fn watch_guides(&mut self) {
        let zone = match &self.gestures.doing {
            Doing::Moving { zone, .. } | Doing::Sizing { zone, .. } => zone.clone(),
            _ => {
                if !self.guides.is_empty() {
                    self.guides.clear();
                    self.redraw = true;
                }
                return;
            }
        };
        self.layout();
        let Some(moved) = self.layout.zone(&zone) else {
            return;
        };
        let Some(desk) = self.layout.desks.iter().find(|desk| desk.display == moved.display) else {
            return;
        };
        let others: Vec<egui::Rect> = self
            .layout
            .zones
            .iter()
            .filter(|other| other.zone != zone && other.display == moved.display)
            .map(|other| other.rect)
            .collect();
        let gap = self.core.config().snap.gap as f32;
        self.guides = if self.core.config().snap.enabled {
            view::guides(moved.rect, desk.area, desk.rect, &others, gap)
        } else {
            Vec::new()
        };
    }

    /// The drag the surface is to begin now, if items were just taken up:
    /// the items, the paths of those that are files, and a picture to
    /// show under the pointer (the first item's, where it has come).
    pub fn take_drag_out(&mut self) -> Option<(Vec<ItemId>, Vec<String>, Option<Picture>)> {
        let (items, paths) = self.drag_out.take()?;
        let first = self.outgoing.first().map(|(item, _)| item.clone());
        let picture = first.and_then(|item| {
            let handle = self.pictures.handle_of(&item)?;
            self.desk.picture(handle)
        });
        Some((items, paths, picture))
    }

    /// As `payload_of`, with the items the backend read out of the
    /// system's own description of the drag: those of them that are ours
    /// (another program's files are described the same way).
    pub fn payload_with(
        &self,
        items: Vec<ItemId>,
        paths: &[String],
        text: Option<String>,
        allowed: DragActions,
    ) -> DropPayload {
        let mut payload = self.payload_of(paths, text, allowed);
        for item in items {
            if self.core.item(&item).is_some() && !payload.items.contains(&item) {
                payload.items.push(item);
            }
        }
        payload
    }

    /// The drag a surface began is over. `remove`: whoever took the
    /// files copied them and leaves the originals to us; those that are
    /// still there go to the bin.
    pub fn drag_ended(&mut self, remove: bool) {
        let outgoing = std::mem::take(&mut self.outgoing);
        if remove {
            for (item, _) in outgoing {
                let Some(path) = self.core.item(&item).and_then(|item| item.path.clone()) else {
                    continue;
                };
                let operation = FileOperation::Trash { path };
                if let Err(error) = self.desk.perform(&operation) {
                    log::warn!("{operation:?} failed: {error}");
                }
            }
        }
        self.redraw = true;
        (self.wake_later)(Duration::ZERO);
    }

    /// The items of ours that the system is dragging now.
    pub fn outgoing_items(&self) -> Vec<ItemId> {
        self.outgoing.iter().map(|(item, _)| item.clone()).collect()
    }

    /// What a drag from the system carries, as the zones see it: the
    /// paths that are items of ours (of the desktop, or of a portal's
    /// folder) are those items, the others are files from elsewhere.
    pub fn payload_of(&self, paths: &[String], text: Option<String>, allowed: DragActions) -> DropPayload {
        let mut payload = DropPayload { text, allowed, ..DropPayload::default() };
        for path in paths {
            let ours = self
                .outgoing
                .iter()
                .map(|(item, _)| item)
                .chain(self.layout.every_item().map(|shown| &shown.item))
                .find(|item| self.core.item(item).is_some_and(|item| item.path.as_ref() == Some(path)));
            match ours {
                Some(item) => payload.items.push(item.clone()),
                None => payload.paths.push(path.clone()),
            }
        }
        payload
    }

    /// Put the system's own icons back as they were written down (asked
    /// from outside, and also with the zones switched off).
    pub fn restore_system_icons(&mut self) -> Result<(), String> {
        let now = explorer::put_back(&mut *self.desk, &self.icons)?;
        // They stay back while these zones are shown (the configuration
        // would put them away again with its next change otherwise).
        self.icons_given_back = self.core.enabled();
        if self.icons != now {
            self.icons = now;
            self.stale = true;
            self.redraw = true;
            (self.wake_later)(Duration::ZERO);
        }
        Ok(())
    }

    /// Something is dragged over our surfaces from outside (or no longer:
    /// `at` is `None`). Says what would be done with it there, for the
    /// pointer's shape; nothing that would is nothing allowed.
    pub fn drag_over(&mut self, at: Option<Pos2>, payload: &DropPayload) -> DragActions {
        let none = DragActions::default();
        let Some(at) = at else {
            if self.incoming.take().is_some() {
                self.redraw = true;
            }
            return none;
        };
        let would = match self.layout().target(at) {
            Target::Nowhere | Target::Zone { locked: true, .. } | Target::Zone { fills: Fills::Query, .. } => none,
            // Into a folder: whatever the source allows, moved rather
            // than copied.
            Target::Zone { fills: Fills::Folder, .. } => DragActions {
                moving: payload.allowed.moving,
                copy: payload.allowed.copy && !payload.allowed.moving,
                link: false,
            },
            // A zone filled by hand and the bare desktop hold what is on
            // the desktop: an item of it is put there; a file from
            // elsewhere is brought to the desktop's folder first (moved
            // where the source lets it, else copied).
            Target::Zone { .. } | Target::Desk { .. } => {
                if !payload.items.is_empty() {
                    DragActions { moving: true, ..none }
                } else if !payload.paths.is_empty() && self.desktop_folder().is_some() {
                    DragActions {
                        moving: payload.allowed.moving,
                        copy: payload.allowed.copy && !payload.allowed.moving,
                        link: false,
                    }
                } else {
                    none
                }
            }
        };
        self.incoming = Some((at, would != none));
        self.redraw = true;
        would
    }

    /// Something was let go on our surfaces: items of ours that the
    /// system dragged, or files of another program.
    pub fn drop_at(&mut self, at: Pos2, payload: &DropPayload) -> Dropped {
        self.incoming = None;
        self.redraw = true;
        let outgoing = std::mem::take(&mut self.outgoing);
        self.layout();
        if !payload.items.is_empty() {
            // Ours: each with where it was held, when the drag began here.
            let items: Vec<(ItemId, egui::Vec2)> = payload
                .items
                .iter()
                .map(|item| {
                    let held = outgoing.iter().find(|(known, _)| known == item).map(|(_, held)| *held);
                    (item.clone(), held.unwrap_or(egui::Vec2::ZERO))
                })
                .collect();
            let acts = gesture::dropped(&self.layout, &items, at);
            if acts.is_empty() {
                return Dropped::Refused;
            }
            self.perform(acts);
            return Dropped::Placed;
        }
        if payload.paths.is_empty() || !(payload.allowed.moving || payload.allowed.copy) {
            return Dropped::Refused;
        }
        let moved = payload.allowed.moving;
        // Files from elsewhere: into a portal's folder as they are; onto
        // a zone filled by hand or the bare desktop, into the desktop's
        // folder first, and from there to where they were let go.
        let zone = match self.layout.target(at) {
            Target::Zone { zone, fills: Fills::Folder, locked: false, .. } => zone,
            Target::Zone { zone, fills: Fills::ByHand, locked: false, .. } => {
                return self.bring_in(payload, Some(zone), None);
            }
            Target::Desk { display, origin } => {
                let place = at - origin;
                let at = gevrek_zones::placement::LoosePosition {
                    display,
                    x: place.x.round() as i32,
                    y: place.y.round() as i32,
                };
                return self.bring_in(payload, None, Some(at));
            }
            _ => return Dropped::Refused,
        };
        let Some(gevrek_zones::config::ZoneKind::Portal { directory }) =
            self.core.config().zone(&zone).map(|zone| zone.kind.clone())
        else {
            return Dropped::Refused;
        };
        for path in &payload.paths {
            let operation = if moved {
                FileOperation::Move { from: path.clone(), to_folder: directory.clone() }
            } else {
                FileOperation::Copy { from: path.clone(), to_folder: directory.clone() }
            };
            if let Err(error) = self.desk.perform(&operation) {
                log::warn!("{operation:?} failed: {error}");
            }
        }
        // (What came into the folder is told by the backend, as any change of it.)
        (self.wake_later)(Duration::ZERO);
        Dropped::Filed { moved }
    }
}

impl Zones {
    /// Bring files from elsewhere to the desktop, and each into a zone or
    /// to its place once it is there.
    fn bring_in(
        &mut self,
        payload: &DropPayload,
        zone: Option<ZoneId>,
        at: Option<gevrek_zones::placement::LoosePosition>,
    ) -> Dropped {
        let Some(folder) = self.desktop_folder() else {
            return Dropped::Refused;
        };
        let moved = payload.allowed.moving;
        self.command(Command::BringIn { paths: payload.paths.clone(), folder, copy: !moved, zone, at });
        (self.wake_later)(Duration::ZERO);
        Dropped::Filed { moved }
    }
}

impl Drop for Zones {
    fn drop(&mut self) {
        // The shell ends: the desktop is the person's own again.
        if let Err(why) = explorer::put_back(&mut *self.desk, &self.icons) {
            log::error!("{why}");
        }
        self.flush();
        self.host.stop(&mut *self.desk);
    }
}

#[cfg(test)]
mod tests {
    use super::draw::tests::{zone, zones_on, zones_with, FakeDesk};
    use super::*;
    use egui::pos2;
    use gevrek_zones::config::ZoneKind;
    use gevrek_zones::item::Item;
    use gevrek_zones::testing::FakeBackend;
    use gevrek_zones::Rect;

    fn press(x: f32, y: f32) -> Pointer {
        Pointer { at: Some(pos2(x, y)), pressed: true, down: true, ..Pointer::default() }
    }

    fn hold(x: f32, y: f32) -> Pointer {
        Pointer { at: Some(pos2(x, y)), down: true, ..Pointer::default() }
    }

    fn release(x: f32, y: f32) -> Pointer {
        Pointer { at: Some(pos2(x, y)), released: true, ..Pointer::default() }
    }

    /// A desk with two files on it, an archive folder beside it, and two
    /// files somewhere else.
    fn backend() -> FakeBackend {
        let mut backend = FakeBackend::new();
        backend.add_folder("/archive");
        backend.add_folder("/elsewhere");
        backend.add("/elsewhere", Item::file("/elsewhere/c.txt"));
        backend.add("/elsewhere", Item::file("/elsewhere/d.txt"));
        backend.add("/desk", Item::file("/desk/a.txt"));
        backend.add("/desk", Item::file("/desk/b.txt"));
        backend
    }

    fn id(name: &str) -> ItemId {
        ItemId::new(format!("/desk/{name}"))
    }

    #[test]
    fn what_the_pointer_does_reaches_the_core_and_the_state_is_written_when_it_is_quiet() {
        let (mut zones, woken) = zones_on(backend(), "pointer");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let file = zones.store.file();
        zones.flush();
        let written = std::fs::read_to_string(&file).unwrap();

        // By its title, to the right and down; near nothing, so nothing pulls.
        zones.pointer(&press(400.0, 112.0));
        zones.pointer(&hold(700.0, 412.0));
        assert_eq!(zones.core().zone_rect(&papers), Some(Rect::new(600, 400, 400, 300)));
        // Near the display's left side it comes to rest on it, and a line shows where.
        zones.pointer(&hold(105.0, 412.0));
        assert_eq!(zones.core().zone_rect(&papers), Some(Rect::new(0, 400, 400, 300)));
        assert_eq!(zones.guides.len(), 1);
        assert_eq!(zones.guides[0].from.x, 0.0);
        zones.pointer(&release(105.0, 412.0));
        assert!(zones.guides.is_empty());

        // Not written with every movement: only asked to be woken for it.
        assert!(zones.unsaved.is_some());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
        assert!(*woken.lock().unwrap() > 0);
        zones.tick();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), written, "too soon after the last change");
        // Quiet for long enough: written.
        let long_ago = Instant::now() - SAVE_WHEN_QUIET - Duration::from_millis(50);
        zones.unsaved = Some((long_ago, long_ago));
        zones.tick();
        assert!(zones.unsaved.is_none());
        let now_written = std::fs::read_to_string(&file).unwrap();
        assert_ne!(now_written, written);
        // Changed without pause for too long: written all the same.
        zones.command(Command::MoveZone { zone: papers.clone(), x: 50, y: 50, snap: false });
        zones.unsaved = Some((Instant::now() - SAVE_AT_LATEST - Duration::from_millis(50), Instant::now()));
        zones.tick();
        assert_ne!(std::fs::read_to_string(&file).unwrap(), now_written);
        // And what is not written when the zones end is written then.
        zones.command(Command::MoveZone { zone: papers, x: 90, y: 90, snap: false });
        let before_the_end = std::fs::read_to_string(&file).unwrap();
        drop(zones);
        assert_ne!(std::fs::read_to_string(&file).unwrap(), before_the_end);
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn what_the_zones_change_of_the_configuration_is_handed_over_once() {
        let (mut zones, _) = zones_on(backend(), "config");
        assert!(zones.tick().config.is_none());
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let changed = zones.tick().config.expect("a zone was made: the configuration has it");
        assert_eq!(changed.zones[0].id, papers);
        assert!(zones.tick().config.is_none(), "only once");
        // It comes back from the window manager as the running
        // configuration: nothing more to do, nothing to write again.
        zones.configure(&changed);
        assert!(zones.tick().config.is_none());
        // Moving a zone is no change of the configuration.
        zones.command(Command::MoveZone { zone: papers.clone(), x: 10, y: 10, snap: false });
        assert!(zones.tick().config.is_none());
        // A change from outside (the file was edited) is taken in and not
        // sent back.
        let mut edited = changed.clone();
        edited.zones[0].title = "Paperwork".to_string();
        zones.configure(&edited);
        assert_eq!(zones.layout().zones[0].title, "Paperwork");
        assert!(zones.tick().config.is_none());
        // What happened is kept for whoever listens.
        let heard = zones.take_heard();
        assert!(heard.contains(&ZonesEvent::ZoneCreated { zone: papers }));
        assert!(zones.take_heard().is_empty());
    }

    #[test]
    fn a_double_click_opens_and_the_other_button_asks_the_system_or_for_a_menu_of_ours() {
        let desk = FakeDesk::new(backend());
        let said = std::sync::Arc::clone(&desk.said);
        let (mut zones, _) = zones_with(desk, "open");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        zones.command(Command::AddItem { zone: papers.clone(), item: id("a.txt"), index: None, cell: None });
        let item = zones.layout().item(&id("a.txt")).unwrap().rect.center();
        zones.pointer(&Pointer { double: true, ..press(item.x, item.y) });
        zones.pointer(&Pointer { menu: true, at: Some(item), ..Pointer::default() });
        // (After what was done about the system's own icons at the start.)
        assert_eq!(
            said.lock().unwrap()[2..],
            ["open /desk/a.txt".to_string(), format!("menu of 1 at {},{}", item.x.round(), item.y.round())]
        );
        assert!(zones.take_asks().is_empty(), "the system showed its own menu");
        // A zone's menu and the desktop's are ours.
        zones.pointer(&Pointer { menu: true, at: Some(pos2(400.0, 112.0)), ..Pointer::default() });
        zones.pointer(&Pointer { menu: true, at: Some(pos2(1500.0, 900.0)), ..Pointer::default() });
        assert_eq!(
            zones.take_asks(),
            [
                Ask::ZoneMenu { zone: papers, at: pos2(400.0, 112.0) },
                Ask::DeskMenu { display: DisplayId::new("fake-1"), at: pos2(1500.0, 900.0) },
            ]
        );
    }

    #[test]
    fn the_wheel_scrolls_a_zone_as_far_as_it_has() {
        let mut backend = backend();
        for number in 0..12 {
            backend.add("/desk", Item::file(&format!("/desk/file-{number}.txt")));
        }
        let (mut zones, _) = zones_on(backend, "wheel");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 200, 160));
        for number in 0..12 {
            zones.command(Command::AddItem { zone: papers.clone(), item: id(&format!("file-{number}.txt")), index: None, cell: None });
        }
        let extent = zones.layout().zone(&papers).unwrap().scroll_extent;
        assert!(extent > 100.0);
        zones.scroll(pos2(400.0, 200.0), -60.0);
        assert_eq!(zones.layout().zone(&papers).unwrap().scroll, 60.0);
        zones.scroll(pos2(400.0, 200.0), -10_000.0);
        assert_eq!(zones.layout().zone(&papers).unwrap().scroll, extent);
        zones.scroll(pos2(400.0, 200.0), 10_000.0);
        assert_eq!(zones.layout().zone(&papers).unwrap().scroll, 0.0);
        // Over the bare desktop the wheel is nobody's.
        zones.scroll(pos2(1500.0, 900.0), -60.0);
        assert!(zones.scrolls.values().all(|scroll| *scroll == 0.0));
    }

    #[test]
    fn a_drop_from_outside_goes_to_the_right_command() {
        let (mut zones, _) = zones_on(backend(), "drop");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let archive = zone(&mut zones, "Archive", ZoneKind::Portal { directory: "/archive".to_string() }, (800, 100, 400, 300));
        let smart = zone(&mut zones, "All", ZoneKind::Smart { query: Vec::new() }, (300, 500, 400, 300));
        let ours = DropPayload { items: vec![id("a.txt")], allowed: DragActions::ALL, ..DropPayload::default() };
        let foreign = DropPayload {
            paths: vec!["/elsewhere/c.txt".to_string()],
            allowed: DragActions { copy: true, ..DragActions::default() },
            ..DropPayload::default()
        };
        let moving = DragActions { moving: true, ..DragActions::default() };
        let none = DragActions::default();
        let on_papers = pos2(500.0, 250.0);
        let on_archive = pos2(1000.0, 250.0);
        let on_smart = pos2(500.0, 650.0);
        let on_desk = pos2(1500.0, 900.0);

        // What would be done, for the pointer's shape.
        assert_eq!(zones.drag_over(Some(on_papers), &ours), moving);
        assert_eq!(zones.drag_over(Some(on_desk), &ours), moving);
        assert_eq!(zones.drag_over(Some(on_archive), &ours), moving);
        assert_eq!(zones.drag_over(Some(on_smart), &ours), none);
        assert_eq!(zones.drag_over(Some(on_papers), &foreign), DragActions { copy: true, ..none }, "brought to the desktop");
        assert_eq!(zones.drag_over(Some(on_smart), &foreign), none);
        assert_eq!(zones.drag_over(Some(on_archive), &foreign), DragActions { copy: true, ..none });
        assert_eq!(zones.incoming, Some((on_archive, true)));
        assert_eq!(zones.drag_over(None, &foreign), none);
        assert_eq!(zones.incoming, None);

        // Into a zone filled by hand: the item is put there.
        assert_eq!(zones.drop_at(on_smart, &ours), Dropped::Refused);
        assert_eq!(zones.drop_at(on_papers, &ours), Dropped::Placed);
        assert_eq!(zones.core().zone_of(&id("a.txt")), Some(&papers));
        // Onto the bare desktop: it is loose again, where it was let go.
        assert_eq!(zones.drop_at(on_desk, &ours), Dropped::Placed);
        assert_eq!(zones.core().zone_of(&id("a.txt")), None);
        assert_eq!(zones.layout().item(&id("a.txt")).unwrap().rect.min, on_desk);
        // Onto a portal: the file itself goes into the folder.
        assert_eq!(zones.drop_at(on_archive, &ours), Dropped::Placed);
        zones.tick();
        assert_eq!(zones.layout().zone(&archive).unwrap().items[0].item, ItemId::new("/archive/a.txt"));
        // A file of another program, onto a zone filled by hand: copied
        // to the desktop's folder, and put into the zone when it is there.
        assert_eq!(zones.drop_at(on_smart, &foreign), Dropped::Refused);
        assert_eq!(zones.drop_at(on_papers, &foreign), Dropped::Filed { moved: false });
        zones.tick();
        assert_eq!(zones.core().zone_of(&id("c.txt")), Some(&papers));
        // Onto the bare desktop, moved: it lies where it was let go.
        let moved = DropPayload {
            paths: vec!["/elsewhere/d.txt".to_string()],
            allowed: DragActions { moving: true, ..DragActions::default() },
            ..DropPayload::default()
        };
        assert_eq!(zones.drop_at(on_desk, &moved), Dropped::Filed { moved: true });
        zones.tick();
        assert_eq!(zones.layout().item(&id("d.txt")).unwrap().rect.min, on_desk);
        // Into a portal: as it is, into the portal's folder.
        assert_eq!(zones.drop_at(on_archive, &foreign), Dropped::Filed { moved: false });
        zones.tick();
        assert_eq!(zones.layout().zone(&archive).unwrap().items.len(), 2);
        // What allows nothing we do is not taken.
        let link_only = DropPayload {
            paths: vec!["/elsewhere/c.txt".to_string()],
            allowed: DragActions { link: true, ..DragActions::default() },
            ..DropPayload::default()
        };
        assert_eq!(zones.drop_at(on_papers, &link_only), Dropped::Refused);
        let _ = smart;
    }

    #[test]
    fn where_the_system_drags_items_are_handed_over_at_once_and_come_back_as_a_drop() {
        let mut backend = backend();
        backend.add(
            "/desk",
            Item { id: ItemId::new("::computer"), kind: gevrek_zones::item::ItemKind::Virtual, name: "This PC".to_string(), ..Item::default() },
        );
        let (mut zones, _) = zones_with(FakeDesk::new(backend), "system-drag");
        zones.system_drags = true;
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let item = zones.layout().item(&id("a.txt")).unwrap().rect;
        let held = item.center();
        zones.pointer(&press(held.x, held.y));
        zones.pointer(&hold(held.x + 30.0, held.y + 30.0));
        assert_eq!(zones.take_drag_out(), Some((vec![id("a.txt")], vec!["/desk/a.txt".to_string()], None)));
        assert_eq!(zones.take_drag_out(), None, "begun once");
        assert!(!zones.gestures.busy(), "the drag is the system's now");
        // (The surface is told the button came up as the drag begins:
        // that is not the end of it.)
        zones.pointer(&Pointer { released: true, ..Pointer::default() });
        assert_eq!(zones.outgoing.len(), 1);
        // What the system hands back are paths: the one that is ours is
        // our item again, another is a file from elsewhere.
        let payload = zones.payload_of(
            &["/desk/a.txt".to_string(), "/elsewhere/z.txt".to_string()],
            None,
            DragActions::ALL,
        );
        assert_eq!((payload.items.clone(), payload.paths.clone()), (vec![id("a.txt")], vec!["/elsewhere/z.txt".to_string()]));
        // Let go on the bare desktop: it lies where it was held, not with
        // its corner under the pointer.
        let payload = DropPayload { items: vec![id("a.txt")], allowed: DragActions::ALL, ..DropPayload::default() };
        assert_eq!(zones.drop_at(pos2(1500.0, 900.0), &payload), Dropped::Placed);
        let now_at = zones.layout().item(&id("a.txt")).unwrap().rect.min;
        assert_eq!(now_at, pos2(1500.0, 900.0) + (item.min - held));
        // And into a zone.
        zones.pointer(&press(now_at.x + 20.0, now_at.y + 20.0));
        zones.pointer(&hold(now_at.x + 60.0, now_at.y + 20.0));
        assert!(zones.take_drag_out().is_some());
        assert_eq!(zones.drop_at(pos2(500.0, 250.0), &payload), Dropped::Placed);
        assert_eq!(zones.core().zone_of(&id("a.txt")), Some(&papers));
        zones.drag_ended(false);
        assert!(zones.outgoing.is_empty());
        // Taken by a program that copied and left the original to us: to the bin.
        let held = zones.layout().item(&id("b.txt")).unwrap().rect.center();
        zones.pointer(&press(held.x, held.y));
        zones.pointer(&hold(held.x + 30.0, held.y));
        assert!(zones.take_drag_out().is_some());
        zones.drag_ended(true);
        zones.tick();
        assert!(zones.core().item(&id("b.txt")).is_none());
        // What is no file cannot be handed to the system: the shell
        // carries it itself.
        let computer = zones.layout().item(&ItemId::new("::computer")).unwrap().rect.center();
        zones.pointer(&press(computer.x, computer.y));
        zones.pointer(&hold(computer.x + 30.0, computer.y));
        assert_eq!(zones.take_drag_out(), None);
        assert!(matches!(zones.gestures.doing, Doing::Carrying { .. }));
    }

    #[test]
    fn the_systems_icons_are_away_while_zones_show_and_back_when_they_do_not() {
        use explorer::SystemIcons;
        let desk = FakeDesk::new(backend());
        let said = std::sync::Arc::clone(&desk.said);
        let said = move || said.lock().unwrap().clone();
        let (mut zones, _) = zones_with(desk, "icons");
        // Started switched on: written down, put away, and only then shown.
        assert_eq!(zones.icons, SystemIcons::Away);
        assert_eq!(said(), ["journal", "hide"]);
        assert_eq!(zones.spots().len(), 1);
        assert_eq!(zones.not_shown(), None);
        // Another change of the configuration does nothing more to them.
        let mut config = zones.core().config().clone();
        config.loose_icon_size = 64;
        zones.configure(&config);
        assert_eq!(said(), ["journal", "hide"]);
        // Asked back by the person: back, and left so though zones show
        // and the configuration changes again.
        assert_eq!(zones.restore_system_icons(), Ok(()));
        assert_eq!(zones.restore_system_icons(), Ok(()));
        config.loose_icon_size = 48;
        zones.configure(&config);
        zones.set_hide_icons(true);
        assert_eq!(said(), ["journal", "hide", "restore"]);
        assert_eq!(zones.icons, SystemIcons::Theirs);
        assert_eq!(zones.spots().len(), 1);
        // Zones off: nothing to put back any more. On again: away again.
        config.enabled = false;
        zones.configure(&config);
        assert_eq!(said().len(), 3);
        config.enabled = true;
        zones.configure(&config);
        assert_eq!(said(), ["journal", "hide", "restore", "journal", "hide"]);
        // The switch for trying zones without touching the desktop: back.
        zones.set_hide_icons(false);
        assert_eq!(zones.icons, SystemIcons::Theirs);
        assert_eq!(zones.spots().len(), 1, "zones are shown all the same");
        zones.set_hide_icons(true);
        assert_eq!(zones.icons, SystemIcons::Away);
        // Off while they are away: put back.
        config.enabled = false;
        zones.configure(&config);
        assert_eq!(zones.icons, SystemIcons::Theirs);
        assert_eq!(said().last().map(String::as_str), Some("restore"));
        // And when the zones end with them away, they are put back.
        config.enabled = true;
        zones.configure(&config);
        let before = said().len();
        drop(zones);
        assert_eq!(said().len(), before + 1);
        assert_eq!(said().last().map(String::as_str), Some("restore"));
    }

    #[test]
    fn where_the_icons_cannot_be_put_away_no_zone_is_shown_and_it_is_said_why() {
        let mut desk = FakeDesk::new(backend());
        desk.fail_hide = Some(BackendError::Refused { what: "Explorer said no".to_string() });
        let said = std::sync::Arc::clone(&desk.said);
        let (mut zones, _) = zones_with(desk, "icons-refused");
        assert_eq!(*said.lock().unwrap(), ["journal", "hide", "restore"]);
        assert!(zones.spots().is_empty(), "no surface is opened");
        let why = zones.not_shown().expect("it is said why");
        assert!(why.contains("Explorer said no"), "{why}");
        assert!(zones.take_heard().contains(&ZonesEvent::Refused { reason: why }));
        // With the switch off the same zones show, and nothing is touched.
        zones.set_hide_icons(false);
        assert_eq!(zones.not_shown(), None);
        assert_eq!(zones.spots().len(), 1);
        assert_eq!(said.lock().unwrap().len(), 3);
        // Zones that are switched off never touch the desktop's icons.
        let desk = FakeDesk::new(backend());
        let said = std::sync::Arc::clone(&desk.said);
        let (mut zones, _) = zones_with(desk, "icons-off");
        let mut off = zones.core().config().clone();
        off.enabled = false;
        zones.configure(&off);
        assert_eq!(*said.lock().unwrap(), ["journal", "hide", "restore"]);
        drop(zones);
        assert_eq!(said.lock().unwrap().len(), 3);
    }

    #[test]
    fn the_keys_put_into_the_bin_rename_and_go_to_a_name() {
        let mut backend = backend();
        backend.add("/desk", Item::file("/desk/budget.xlsx"));
        let (mut zones, _) = zones_on(backend, "keys");
        // Typed in quick succession: the first name that begins so.
        zones.typed("b");
        assert_eq!(zones.selection(), [id("b.txt")]);
        zones.typed("u");
        assert_eq!(zones.selection(), [id("budget.xlsx")]);
        // After a while a letter begins anew.
        zones.typed.1 = Some(Instant::now() - TYPED_TOGETHER - Duration::from_millis(10));
        zones.typed("A");
        assert_eq!(zones.selection(), [id("a.txt")]);
        // F2: the box for its new name; the name is a command.
        zones.key(gesture::Key::Rename);
        assert_eq!(zones.take_asks(), [Ask::RenameItem { item: id("a.txt") }]);
        zones.command(Command::RenameItem { item: id("a.txt"), name: "first.txt".to_string() });
        zones.tick();
        assert!(zones.core().item(&id("first.txt")).is_some());
        // Delete: into the bin, and back out of it.
        zones.gestures.selected = std::collections::BTreeSet::from([id("b.txt")]);
        zones.key(gesture::Key::Delete);
        zones.tick();
        assert!(zones.core().item(&id("b.txt")).is_none());
        zones.command(Command::UndoFileOperation);
        zones.tick();
        assert!(zones.core().item(&id("b.txt")).is_some());
        // Nothing selected: the keys do nothing.
        zones.gestures.selected.clear();
        zones.key(gesture::Key::Delete);
        zones.key(gesture::Key::Rename);
        assert!(zones.take_asks().is_empty());
        // A menu's own lines for the two are the same commands.
        zones.act(&[id("b.txt")], TRASH);
        zones.tick();
        assert!(zones.core().item(&id("b.txt")).is_none());
        zones.act(&[id("budget.xlsx")], RENAME);
        assert_eq!(zones.take_asks(), [Ask::RenameItem { item: id("budget.xlsx") }]);
    }

    #[test]
    fn the_window_managers_state_tells_the_displays_and_the_workspace() {
        let (mut zones, woken) = zones_on(backend(), "told");
        let before = *woken.lock().unwrap();
        let state = serde_json::json!({ "monitors": [] });
        zones.told(&state, Some("2".to_string()));
        assert!(*woken.lock().unwrap() > before);
        // (The fake's displays are its own; that it was told is what counts.)
        assert_eq!(zones.core().displays().len(), 1);
    }
}
