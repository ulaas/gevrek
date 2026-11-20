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

pub mod access;
pub mod backend;
pub mod draw;
pub mod explorer;
pub mod gesture;
pub mod menu;
pub mod motion;
pub mod surfaces;
pub mod view;

use egui::Pos2;
use gesture::{Act, Doing, Gestures, Pointer};
use gevrek_zones::focus::SelectMode;
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

    /// Whether the system draws icons on the desktop itself (Windows:
    /// Explorer's). Where it does and they are put away, an icon the
    /// zones do not draw is not on the desktop at all.
    fn has_system_icons(&self) -> bool {
        false
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

    /// The programs the person pinned, by the ids the system's launcher
    /// knows them by, in their order.
    fn pinned(&self) -> Vec<String> {
        Vec::new()
    }

    /// Pin a program, or let it go again.
    fn pin(&mut self, _program: &str, _pin: bool) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    /// Whether a file is kept by a cloud and not on this machine now
    /// (opening it fetches it). Looking must fetch nothing.
    fn online_only(&mut self, _item: &ItemId) -> bool {
        false
    }

    /// Every program some items can be opened with, as lines of a menu
    /// (each carried out by [`Desk::act`]), the usual one first.
    fn programs(&mut self, _items: &[ItemId]) -> Vec<DeskAction> {
        Vec::new()
    }

    /// What there is to say of some items, as lines of a name and a
    /// value, where the system has no window of its own for it.
    fn properties(&mut self, _items: &[ItemId]) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// A menu's line that gives one item another name.
pub const RENAME: &str = "rename";
/// A menu's line that puts the items into the bin.
pub const TRASH: &str = "trash";
/// A menu's line that lets the items stay when the rest is put away with
/// the one key (or no longer).
pub const STAY: &str = "stay";
/// A menu's line that takes the items out of their zone.
pub const LOOSEN: &str = "loosen";
/// A menu's line that pins a program (or lets it go again).
pub const PIN: &str = "pin";
/// A menu's line that leads to every program the items can be opened with.
pub const CHOOSE: &str = "choose";
/// A menu's line that shows what there is to say of the items.
pub const PROPERTIES: &str = "properties";
/// How a menu's line begins that makes a folder; the folder it is made
/// in follows.
pub const NEW_FOLDER: &str = "newfolder:";

/// What the rules would do now, as it is shown before it is done.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulesPreview {
    pub scope: gevrek_zones::rules::Scope,
    /// What would go where, by which rule; and what is held back, and why.
    pub lines: Vec<gevrek_zones::rules::PlanLine>,
    /// How often the rules that ask how old an item is are tried, in
    /// seconds; none where there is no such rule or they are never tried.
    pub timed_every: Option<u64>,
    /// Whether those may move files unasked.
    pub timed_move_files: bool,
    /// Whether an earlier sorting can be taken back.
    pub can_undo: bool,
}

/// The zones that would be made by type, as they are shown before they
/// are made.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoPreview {
    pub scope: gevrek_zones::rules::Scope,
    pub plan: gevrek_zones::auto::AutoPlan,
    /// What each item of the plan is called.
    pub names: std::collections::HashMap<ItemId, String>,
    /// The display the new zones would stand on, and the zones that
    /// stand on it now (for the small picture of where they would go).
    pub display: egui::Rect,
    pub there: Vec<egui::Rect>,
    /// Whether zones made so before can be taken back.
    pub can_undo: bool,
}

pub use backend::{desk, Starter};

/// How long the pointer rests on a button of a title before it is said
/// what the button does.
const HINT_AFTER: f64 = 0.6;

/// The name of the variable that switches the trace on (`GEVREK_ZONES_TRACE=1`).
pub const TRACE: &str = "GEVREK_ZONES_TRACE";

/// The file the trace is written to, in the zones' own folder: the shell
/// is often started where what it writes to its log is seen by nobody,
/// and a file is what a person can send.
pub const TRACE_FILE: &str = "trace.log";

/// How long letters typed one after the other are one beginning of a name.
const TYPED_TOGETHER: Duration = Duration::from_millis(1000);

/// How long a folder that is away is left alone before it is asked for
/// again unasked. (The backends never ask again by themselves; a share
/// that is switched on again announces itself to nobody.)
const ASK_AGAIN: Duration = Duration::from_secs(60);

/// How many of what was found are shown while the zones are looked at.
pub const MOST_FOUND: usize = 8;

/// What is typed while the zones are looked at above the windows, and
/// what the core found for it everywhere.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Search {
    pub typed: String,
    /// The best first, as the core ranks them.
    pub hits: Vec<gevrek_zones::search::Hit>,
    /// The one Enter opens.
    pub at: usize,
}

/// Something the pointer asked for that needs a surface of its own: a
/// menu, or the box a name is typed into. [`surfaces`] opens them.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    ItemMenu { items: Vec<ItemId>, at: Pos2, lines: Vec<DeskAction> },
    ZoneMenu { zone: ZoneId, at: Pos2 },
    /// How a zone shows and orders what it holds, as a menu of its own
    /// (asked for by the button of its title).
    ViewMenu { zone: ZoneId, at: Pos2 },
    /// What the rules would do, to be looked over before it is done.
    Rules { scope: gevrek_zones::rules::Scope },
    /// The zones that would be made for the types of what lies on the
    /// desktop, to be looked over before they are made; the new ones on
    /// this display.
    AutoZones { display: DisplayId },
    /// The box the name of a new folder in a portal is typed into.
    NewFolder { zone: ZoneId },
    /// The same for a folder made in a folder that is named (the
    /// desktop's own, from an item's menu).
    NewFolderIn { parent: String },
    /// Something to be read on a sheet: a heading, and lines of a name
    /// and a value (what there is to say of some items; why something
    /// could not be done).
    Sheet { heading: String, lines: Vec<(String, String)> },
    /// The box the name is typed into under which this arrangement is kept.
    SaveLayout,
    /// The box a kept arrangement's new name is typed into.
    RenameLayout { name: String },
    /// The box for the file an arrangement is written to (a kept one, or
    /// without a name the one in use).
    ExportLayout { name: Option<String> },
    /// The box for the file an arrangement is taken from.
    ImportLayout,
    /// The box a folder's path is typed into, to make a portal of at a point.
    NewPortal { at: Pos2 },
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
    /// The crossings of the grid the moved zone's corners rest on.
    grid_marks: Vec<Pos2>,
    /// What moves by itself, and when each thing set out.
    pub motion: motion::Motion,
    /// When the zones started: what the core is told as "now" in parts
    /// of a second is counted from here.
    began: Instant,
    /// How far the clock is put forward (a test's way to let time pass).
    ahead: f64,
    /// What the core was last told the pointer is over, and whether
    /// something was carried.
    over: Option<(Option<ZoneId>, bool)>,
    /// When the core asked to be told the time next, as far as a
    /// wake-up was asked for it.
    tick_due: Option<u64>,
    /// When the rules that ask the time are next tried.
    rules_due: Option<Instant>,
    /// When the folders that are away are next asked for again; none
    /// while none is away.
    retry_due: Option<Instant>,
    /// Which files of the folders shown are not on this machine (a cloud
    /// keeps them), as far as the backend was asked.
    clouds: std::collections::HashMap<ItemId, bool>,
    /// Each zone as it last was with its items to be seen: what is drawn
    /// of it while it rolls shut.
    ghosts: BTreeMap<ZoneId, view::ZoneBox>,
    /// Everything as it was before it was hidden at once or brought
    /// back: what goes is drawn fading out of it, what comes fading in.
    fading: Option<Layout>,
    /// What is typed and found while the zones are looked at.
    pub search: Option<Search>,
    /// Where the menu of some items was last asked for: a menu that
    /// follows from one of its lines opens there too.
    menu_at: Pos2,
    /// What a screen reader is told of the zones, as the core words it:
    /// kept while one listens and the zones do not change.
    said: Option<gevrek_zones::access::AccessNode>,
    /// What each node of each display's tree stands for.
    said_ids: std::collections::HashMap<(egui::accesskit::NodeId, DisplayId), access::Spoken>,
    /// How often everything was laid out anew (for those who measure).
    pub laid_out: u64,
    /// The items' names broken into lines, by item and the width they
    /// were broken for: kept until the layout is made anew.
    labels: std::collections::HashMap<u32, std::collections::HashMap<ItemId, std::rc::Rc<[String]>>>,
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
    /// Tells a double press from two single ones, for each button.
    twice: gevrek_shellkit::presses::Counter,
    /// The presses the surface that is drawn now heard since its last
    /// frame, each with the system's time of it; none where no host said
    /// (then the frames' own time is what there is).
    presses: Option<Vec<gevrek_shellkit::Press>>,
    /// The button of a title the pointer rests on, and since when.
    rested: Option<(ZoneId, view::Button, f64)>,
    /// The next menu was asked for by a key: the keys begin on its first
    /// line.
    menu_by_keys: bool,
    /// Every press, what it hit and what came of it is written to the
    /// log (`GEVREK_ZONES_TRACE=1`, for finding out why something does
    /// not react on a real desktop).
    pub trace: bool,
    /// The file the trace goes to as well, where it was switched on by
    /// the variable.
    trace_file: Option<std::path::PathBuf>,
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
        // The trace of one trial is one file: begun anew with each start.
        let traced = std::env::var_os(TRACE).is_some_and(|value| !value.is_empty() && value != "0");
        let trace_file = traced.then(|| folder.join(TRACE_FILE)).filter(|file| {
            let begun = std::fs::create_dir_all(&folder).and_then(|()| {
                std::fs::write(file, format!("gevrek zones trace, begun at {} (seconds since 1970)\n", now()))
            });
            if let Err(error) = &begun {
                log::warn!("the zones' trace cannot be written to {}: {error}", file.display());
            }
            begun.is_ok()
        });
        log::debug!("the zones start, with their state in {}", folder.display());
        let store = Store::new(folder);
        let loaded = gevrek_zones::timed::timed(
            || format!("reading the zones' state from {}", store.file().display()),
            || store.load(),
            |loaded| gevrek_zones::timed::result(loaded, |loaded| format!("{:?}", loaded.report)),
        );
        let state = match loaded {
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
            grid_marks: Vec::new(),
            motion: motion::Motion::default(),
            began: Instant::now(),
            ahead: 0.0,
            over: None,
            tick_due: None,
            rules_due: None,
            retry_due: None,
            clouds: std::collections::HashMap::new(),
            ghosts: BTreeMap::new(),
            fading: None,
            search: None,
            menu_at: Pos2::ZERO,
            said: None,
            said_ids: std::collections::HashMap::new(),
            laid_out: 0,
            labels: std::collections::HashMap::new(),
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
            twice: gevrek_shellkit::presses::Counter::default(),
            presses: None,
            rested: None,
            menu_by_keys: false,
            trace: traced,
            trace_file,
        };
        if zones.trace {
            match &zones.trace_file {
                Some(file) => log::warn!("the zones' trace is on ({TRACE} is set): every press, what it hit and what came of it is written to {}", file.display()),
                None => log::warn!("the zones' trace is on ({TRACE} is set): written to the log at the level of information"),
            }
        }
        // The system's own icons first: no surface is opened before they
        // are safely away (or left alone).
        zones.settle_icons();
        let effects = zones.host.start(&mut zones.core, &mut *zones.desk, now());
        zones.take(effects);
        // (Asked once, here, and known from then on.)
        zones.desktop_folder();
        zones
    }

    /// The desktop's own folder, as far as the backend named one.
    pub fn desktop_known(&self) -> Option<String> {
        self.desktop.clone().flatten()
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
        self.tell_icons();
    }

    /// Tell the core whether the system's own icons are away: an icon
    /// that comes by itself then, with no zone to take it, is not drawn
    /// (`ZonesCore::is_withheld`).
    fn tell_icons(&mut self) {
        let away = self.icons == explorer::SystemIcons::Away && self.desk.has_system_icons();
        if self.core.set_system_icons_away(away) {
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
            let desktop = self.desk.desktop();
            log::debug!("the desktop's folder for the zones is {}", desktop.as_deref().unwrap_or("not known"));
            self.desktop = Some(desktop);
        }
        self.desktop.clone().flatten()
    }

    /// Another workspace is shown (for arrangements that differ by workspace).
    pub fn workspace(&mut self, name: Option<String>) {
        let effects = self.core.event(BackendEvent::Workspace { name }, now());
        let changed = !effects.is_empty();
        self.take(effects);
        if changed {
            self.look_again();
        }
    }

    /// Zones came into view: those of them that wait for a folder that
    /// is away ask for it again with the next look, not a minute later.
    fn look_again(&mut self) {
        self.retry_due = Some(Instant::now());
        (self.wake_later)(Duration::ZERO);
    }

    /// Whether a file a portal shows is not on this machine (a cloud
    /// keeps it and fetches it when it is opened). Asked of the backend
    /// once for each file, until the folder's items change.
    pub fn in_the_cloud(&mut self, item: &ItemId) -> bool {
        if let Some(known) = self.clouds.get(item) {
            return *known;
        }
        let away = self.desk.online_only(item);
        self.clouds.insert(item.clone(), away);
        away
    }

    /// Carry out a command: from a menu here, a key, or another program.
    /// Everything the pointer changes goes through here too.
    pub fn command(&mut self, command: Command) -> Vec<Effect> {
        let effects = self.host.command(&mut self.core, &mut *self.desk, command, now());
        self.take(effects.clone());
        effects
    }

    /// The seconds since the zones started, for everything that moves.
    pub fn seconds(&self) -> f64 {
        self.began.elapsed().as_secs_f64() + self.ahead
    }

    /// The same in thousandths, as the core is told the time.
    fn at_ms(&self) -> u64 {
        (self.seconds() * 1000.0) as u64
    }

    /// Let time pass without waiting for it (tests and pictures).
    #[cfg(test)]
    pub fn later(&mut self, seconds: f64) {
        self.ahead += seconds;
    }

    /// The core waits for a time (a rolled-up zone is to open, or to
    /// shut): ask to be woken then, once for each time it names.
    fn wake_for_core(&mut self) {
        let due = self.core.wake_at_ms();
        if due.is_some() && due != self.tick_due {
            let wait = due.unwrap_or_default().saturating_sub(self.at_ms());
            (self.wake_later)(Duration::from_millis(wait.max(1)));
        }
        self.tick_due = due;
    }

    /// Tell the core which zone the pointer is over, when that is another
    /// than it was last told (or something is carried now, or no longer).
    fn pointer_over(&mut self, zone: Option<ZoneId>, dragging: bool) {
        let over = Some((zone, dragging));
        if self.over == over {
            return;
        }
        self.over = over.clone();
        let Some((zone, dragging)) = over else {
            return;
        };
        self.command(Command::PointerOver { zone, at_ms: self.at_ms(), dragging });
    }

    /// The zone a point of the desktop is on.
    fn zone_at(&mut self, at: Pos2) -> Option<ZoneId> {
        match self.layout().hit(at) {
            view::Hit::Item { zone, .. } => zone,
            view::Hit::Head(zone)
            | view::Hit::Nav(zone, _)
            | view::Hit::Button(zone, _)
            | view::Hit::Tab(zone)
            | view::Hit::Edge(zone, _)
            | view::Hit::Body(zone) => Some(zone),
            view::Hit::Desk(_) | view::Hit::Nothing => None,
        }
    }

    /// Write a line of the trace, if it is switched on: when (in seconds
    /// since the zones started), and what.
    pub fn traced(&self, what: impl FnOnce() -> String) {
        if !self.trace {
            return;
        }
        let line = format!("+{:.3}s {}", self.seconds(), what());
        log::info!("zones trace {line}");
        if let Some(file) = &self.trace_file {
            use std::io::Write;
            // (A trace that cannot be written is no reason to stop.)
            // (Said once, when it first fails.)
            static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            let written = std::fs::OpenOptions::new().append(true).open(file).and_then(|mut file| writeln!(file, "{line}"));
            if let Err(error) = written {
                if !FAILED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    log::debug!("the zones' trace could not be written to {}: {error}", file.display());
                }
            }
        }
    }

    /// The host says when the system saw each press that the surface
    /// drawn next heard of.
    pub fn pressed_at(&mut self, presses: Vec<gevrek_shellkit::Press>) {
        self.presses = Some(presses);
    }

    /// The presses of the surface that is drawn now, as the host said
    /// them; none where no host did.
    fn take_presses(&mut self) -> Option<Vec<gevrek_shellkit::Press>> {
        self.presses.take()
    }

    /// A button went down at a point of the desktop. `true`: it is the
    /// second press of two in quick succession, by what a double press
    /// is on this system (`within_ms` apart at most, and no farther than
    /// `apart`).
    fn pressed_twice(&mut self, press: gevrek_shellkit::Press, within_ms: u32, apart: f32) -> bool {
        let second = self.twice.press(press, within_ms, apart);
        self.traced(|| {
            format!(
                "press {:?} at {:.0},{:.0} system-time {} ms: {} (a double press is within {within_ms} ms and {apart:.0} points)",
                press.button,
                press.at.x,
                press.at.y,
                press.time_ms,
                if second { "the SECOND of a double press" } else { "a first press" }
            )
        });
        second
    }

    /// Whether a menu that is opened now was asked for by a key (asked
    /// once: the next is the pointer's again).
    pub fn take_menu_by_keys(&mut self) -> bool {
        std::mem::take(&mut self.menu_by_keys)
    }

    /// Every program some items can be opened with, as lines of a menu.
    pub fn programs_for(&mut self, items: &[ItemId]) -> Vec<DeskAction> {
        self.desk.programs(items)
    }

    /// The pointer's shape at a point of the desktop: of what is under
    /// way, else of what a press there would take hold of.
    pub fn pointer_shape(&mut self, at: Pos2) -> egui::CursorIcon {
        match &self.gestures.doing {
            Doing::Sizing { edges, .. } => view::pointer_for_edges(*edges),
            Doing::Moving { .. } => egui::CursorIcon::Move,
            Doing::Nothing | Doing::Pressed { .. } => self.layout().pointer_at(at),
            _ => egui::CursorIcon::Default,
        }
    }

    /// The side or corner of a zone the pointer would take hold of, or
    /// has hold of: shown on the zone, so that it is seen before it is
    /// pressed.
    #[cfg(test)]
    pub fn grip_shown(&mut self) -> Option<(ZoneId, gevrek_zones::snap::Edges)> {
        self.layout();
        self.grip_shown_in(&self.layout)
    }

    /// As `grip_shown`, in a layout that is given (the one that is being
    /// drawn from, which is not the zones' own for that while).
    fn grip_shown_in(&self, layout: &Layout) -> Option<(ZoneId, gevrek_zones::snap::Edges)> {
        if let Doing::Sizing { zone, edges, .. } = &self.gestures.doing {
            return Some((zone.clone(), *edges));
        }
        if self.gestures.busy() {
            return None;
        }
        match layout.hit(self.hover?) {
            view::Hit::Edge(zone, edges) if layout.shown_for(&zone).is_some_and(|shown| !shown.locked_place) => Some((zone, edges)),
            _ => None,
        }
    }

    /// Note which button of a title the pointer rests on.
    fn rest_on(&mut self, at: Option<Pos2>) {
        let on = match at.map(|at| self.layout().hit(at)) {
            Some(view::Hit::Button(zone, button)) if !self.gestures.busy() => Some((zone, button)),
            _ => None,
        };
        match (&self.rested, on) {
            (Some((zone, button, _)), Some((on_zone, on_button))) if *zone == on_zone && *button == on_button => {}
            (_, on) => self.rested = on.map(|(zone, button)| (zone, button, self.seconds())),
        }
    }

    /// How long until it is said what the button under the pointer does,
    /// in seconds; none while the pointer rests on none, or it is said.
    pub fn hint_in(&self) -> Option<f64> {
        let (_, _, since) = self.rested.as_ref()?;
        let left = since + HINT_AFTER - self.seconds();
        (left > 0.0).then_some(left)
    }

    /// What is to be said of the button the pointer rests on, on a
    /// display: the button's place in that display's own points, and the
    /// words. None until the pointer has rested there for a moment.
    pub fn hint_on(&mut self, display: &DisplayId) -> Option<(egui::Rect, String)> {
        let (zone, button, since) = self.rested.clone()?;
        if self.seconds() - since < HINT_AFTER {
            return None;
        }
        let layout = self.layout();
        let shown = layout.zone(&zone).filter(|shown| &shown.display == display)?;
        let origin = layout.desks.iter().find(|desk| &desk.display == display)?.rect.min;
        let placed = shown.buttons.iter().find(|placed| placed.button == button)?;
        Some((placed.rect.translate(-origin.to_vec2()), view::button_says(shown, button).to_string()))
    }

    /// The zones that would be made for the types of what lies on the
    /// desktop, for the person to look over: nothing is changed by asking.
    pub fn auto_preview(&mut self, scope: gevrek_zones::rules::Scope, display: &DisplayId) -> AutoPreview {
        let plan = self.core.auto_plan(scope, None, Some(display), now());
        let on = plan.display.clone().unwrap_or_else(|| display.clone());
        let names = plan
            .zones
            .iter()
            .flat_map(|zone| zone.items.iter())
            .filter_map(|item| Some((item.clone(), self.core.item(item)?.name.clone())))
            .collect();
        let can_undo = self.core.can_undo_auto();
        let layout = self.layout();
        AutoPreview {
            scope,
            names,
            display: layout.desks.iter().find(|desk| desk.display == on).map_or(egui::Rect::ZERO, |desk| desk.rect),
            there: layout.zones.iter().filter(|zone| zone.display == on).map(|zone| zone.rect).collect(),
            can_undo,
            plan,
        }
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
                            log::warn!("{operation} failed: {error}")
                        }
                        // A file that changed has another picture.
                        ZonesEvent::ItemsChanged { .. } => {
                            self.pictures.forget_gone(&self.core);
                            self.clouds.clear();
                        }
                        // The core says when something is to be started
                        // (it ends a look above the windows with it).
                        ZonesEvent::OpenRequested { items, with } => {
                            for item in items {
                                let opened = match with {
                                    Some(program) => self.desk.open_with(item, Some(program)),
                                    None => self.desk.open(item),
                                };
                                if let Err(error) = opened {
                                    log::warn!("{item} could not be opened: {error}");
                                }
                            }
                        }
                        // What was refused shows on the zone it was about,
                        // also when it was asked by a key or from outside.
                        ZonesEvent::RefusedAbout { zone: Some(zone), .. } => {
                            let shown = self.layout.shown_for(zone).map(|shown| shown.zone.clone());
                            if let Some(shown) = shown {
                                let at = self.began.elapsed().as_secs_f64() + self.ahead;
                                self.motion.refused(&shown, at);
                            }
                        }
                        // Hidden at once, or brought back: what goes and
                        // what comes is drawn from how it was until now.
                        ZonesEvent::VisibilityChanged { peeking, .. } => {
                            if !self.motion.still && self.fading.is_none() {
                                self.fading = Some(self.layout.clone());
                            }
                            // (What comes into view and waits for a folder
                            // asks for it again at once.)
                            self.look_again();
                            if !peeking {
                                self.search = None;
                            }
                        }
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
        self.wake_for_core();
    }

    /// The shell was woken for the zones: hear what the backend has to
    /// tell, and write the state if it has been quiet long enough.
    pub fn tick(&mut self) -> Tick {
        let effects = self.host.pump(&mut self.core, &mut *self.desk, now());
        self.take(effects);
        // What the core was waiting for: its time has come.
        let at_ms = self.at_ms();
        if self.core.wake_at_ms().is_some_and(|due| due <= at_ms) {
            self.command(Command::Tick { at_ms });
        }
        // The rules that ask how old an item is come true by themselves:
        // they are tried as often as the configuration says, and the
        // zones ask to be woken for it once for each time.
        match self.core.timed_rules_every() {
            Some(every) => {
                let every = Duration::from_secs(every.max(1));
                let at = Instant::now();
                match self.rules_due {
                    Some(due) if at >= due => {
                        self.command(Command::CheckTimedRules);
                        self.rules_due = Some(at + every);
                        (self.wake_later)(every);
                    }
                    Some(_) => {}
                    None => {
                        self.rules_due = Some(at + every);
                        (self.wake_later)(every);
                    }
                }
            }
            None => self.rules_due = None,
        }
        // Folders that are away are asked for again now and then, while
        // a zone that is shown waits for one: one wake-up asked for each
        // time, and none while nothing is away.
        let away: Vec<ZoneId> = self
            .layout()
            .zones
            .iter()
            .filter(|zone| matches!(zone.listing, Some(gevrek_zones::portal::Listing::Unavailable { .. })))
            .map(|zone| zone.zone.clone())
            .collect();
        let at = Instant::now();
        match (away.is_empty(), self.retry_due) {
            (true, _) => self.retry_due = None,
            (false, Some(due)) if at < due => {}
            (false, due) => {
                // (The first time one is seen to be away, only the wait begins.)
                if due.is_some() {
                    for zone in away {
                        self.command(Command::RefreshZone { zone });
                    }
                }
                self.retry_due = Some(at + ASK_AGAIN);
                (self.wake_later)(ASK_AGAIN);
            }
        }
        // The mark of a refusal that does not move ends by the clock.
        if let Some(left) = self.motion.ends_in(self.seconds()) {
            (self.wake_later)(Duration::from_secs_f32(left + 0.01));
            self.redraw = true;
        }
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
            let store = &self.store;
            let state = self.core.state();
            let saved = gevrek_zones::timed::timed(
                || format!("writing the zones' state to {}", store.file().display()),
                || store.save(state),
                gevrek_zones::timed::done,
            );
            if let Err(error) = saved {
                log::error!("the zones' state could not be written: {error}");
            }
        }
    }

    /// What happened since this was last asked, for whoever listens.
    ///
    /// A drag says where its zone is with every movement, and whoever
    /// listens is told when it ends: of a run of such events for one
    /// zone only the last is kept (where it came to rest), and likewise
    /// of a long operation's progress.
    pub fn take_heard(&mut self) -> Vec<ZonesEvent> {
        let mut heard: Vec<ZonesEvent> = Vec::new();
        for event in std::mem::take(&mut self.heard) {
            let same_again = match (heard.last(), &event) {
                (Some(ZonesEvent::ZonePlaced { zone: before, .. }), ZonesEvent::ZonePlaced { zone, .. }) => before == zone,
                (
                    Some(ZonesEvent::OperationProgress { request: before, .. }),
                    ZonesEvent::OperationProgress { request, .. },
                ) => before == request,
                _ => false,
            };
            if same_again {
                heard.pop();
            }
            heard.push(event);
        }
        heard
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
            self.laid_out += 1;
            // (What a screen reader is told is of the zones as they were.)
            self.said = None;
            self.gestures.settle(&self.layout);
            // (Names are broken into lines once and kept; not without end.)
            if self.labels.values().map(std::collections::HashMap::len).sum::<usize>() > 8192 {
                self.labels.clear();
            }
            // Each zone as it is with its items to be seen, for when it
            // rolls shut; and nothing of zones that are gone.
            for zone in self.layout.zones.iter().filter(|zone| !(zone.rolled_up && !zone.expanded)) {
                self.ghosts.insert(zone.zone.clone(), zone.clone());
            }
            let core = &self.core;
            self.ghosts.retain(|zone, _| core.config().zone(zone).is_some());
            self.motion.settle(self.began.elapsed().as_secs_f64() + self.ahead, &|zone| {
                core.config().zone(zone).is_some()
            });
        }
        &self.layout
    }

    /// The pointer did something on one of our surfaces (in the
    /// desktop's units).
    pub fn pointer(&mut self, pointer: &Pointer) {
        self.hover = pointer.at;
        if pointer.menu || pointer.pressed {
            // (Whatever menu comes of this is the pointer's, not a key's.)
            self.menu_by_keys = false;
        }
        // A rolled-up zone the pointer rests on opens, and one it left
        // shuts: the core decides when, told what the pointer is over.
        let over = pointer.at.and_then(|at| self.zone_at(at));
        self.pointer_over(over, self.gestures.carrying());
        self.layout();
        let before = self.gestures.band();
        let acts = self.gestures.pointer(&self.layout, pointer);
        // (After the press was taken in: a button that is held says nothing.)
        self.rest_on(pointer.at);
        if self.trace && (pointer.pressed || pointer.released || pointer.menu || pointer.middle_double) {
            let hit = pointer.at.map(|at| self.layout.hit(at));
            self.traced(|| {
                format!(
                    "pointer at {:?} pressed={} double={} middle-double={} released={} menu={} down={}: hit {hit:?}; decided {acts:?}; now {:?}",
                    pointer.at.map(|at| (at.x.round(), at.y.round())),
                    pointer.pressed,
                    pointer.double,
                    pointer.middle_double,
                    pointer.released,
                    pointer.menu,
                    pointer.down,
                    self.gestures.doing
                )
            });
        }
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
        if before != self.gestures.band() || self.gestures.busy() {
            self.redraw = true;
        }
        self.perform(acts);
        self.watch_guides();
    }

    /// The pointer is on none of our surfaces any more.
    pub fn pointer_left(&mut self) {
        if self.hover.take().is_some() {
            self.redraw = true;
        }
        self.rested = None;
        if !self.gestures.busy() {
            self.pointer_over(None, false);
        }
    }

    /// A key was pressed while a surface of ours has the keyboard.
    pub fn key(&mut self, key: gesture::Key) {
        self.layout();
        self.redraw = true;
        // While the zones are looked at and something was typed, the
        // keys go to what was found.
        if self.search.as_ref().is_some_and(|search| !search.typed.is_empty()) {
            use gevrek_zones::focus::Step;
            match key {
                gesture::Key::Step { step: Step::Down, .. } | gesture::Key::Step { step: Step::Up, .. } => {
                    let down = matches!(key, gesture::Key::Step { step: Step::Down, .. });
                    if let Some(search) = &mut self.search {
                        let most = search.hits.len().min(MOST_FOUND).saturating_sub(1);
                        search.at = if down { (search.at + 1).min(most) } else { search.at.saturating_sub(1) };
                    }
                    return;
                }
                gesture::Key::Enter => {
                    let found = self.search.as_ref().and_then(|search| search.hits.get(search.at).cloned());
                    if let Some(hit) = found {
                        // To where it is (its tab comes forward, a rolled-up
                        // zone opens), and open it: the core ends the look.
                        if hit.zone.is_some() {
                            self.command(Command::FocusItem { item: Some(hit.item.clone()) });
                        } else {
                            self.command(Command::Select { items: vec![hit.item.clone()], mode: SelectMode::Replace });
                        }
                        self.command(Command::Open { items: vec![hit.item], with: None, in_place: true });
                    }
                    return;
                }
                gesture::Key::Escape => {
                    self.search = Some(Search::default());
                    return;
                }
                gesture::Key::Erase => {
                    self.erase();
                    return;
                }
                _ => {}
            }
        }
        // What is the zones' own to do with a key, as a command.
        match key {
            gesture::Key::Erase => {
                self.erase();
                return;
            }
            gesture::Key::Undo => {
                self.command(Command::UndoFileOperation);
                return;
            }
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
        // (A menu asked for by a key has the keys on its first line.)
        self.menu_by_keys = key == gesture::Key::Menu;
        self.perform(acts);
    }

    /// What is chosen, in the order it is shown.
    pub fn selection(&mut self) -> Vec<ItemId> {
        self.layout().selected()
    }

    /// The zone that has the keyboard, with what is typed into its filter.
    fn filtered(&mut self) -> Option<(ZoneId, String)> {
        let zone = self.layout().zones.iter().find(|zone| zone.focused)?;
        Some((zone.zone.clone(), zone.filter.clone().unwrap_or_default()))
    }

    /// Look everywhere for what is typed while the zones are looked at.
    fn look_for(&mut self, typed: String) {
        let hits = if typed.trim().is_empty() { Vec::new() } else { self.core.search(&typed, now()) };
        self.search = Some(Search { typed, hits, at: 0 });
        self.redraw = true;
    }

    /// Take the last letter back from what is typed: of what is looked
    /// for everywhere, or of the filter of the zone that has the keyboard.
    fn erase(&mut self) {
        if self.core.peeking() {
            let mut typed = self.search.as_ref().map(|search| search.typed.clone()).unwrap_or_default();
            typed.pop();
            self.look_for(typed);
        } else if let Some((zone, mut typed)) = self.filtered() {
            if typed.pop().is_some() {
                self.command(Command::Filter { zone: Some(zone), typed });
            }
        }
    }

    /// Letters were typed while a surface of ours has the keyboard.
    /// While the zones are looked at above the windows they are looked
    /// for everywhere; in a zone that has the keyboard they leave only
    /// the items whose names fit; and on the bare desktop the first item
    /// whose name begins with what was typed in quick succession is
    /// chosen.
    pub fn typed(&mut self, text: &str) {
        if self.core.peeking() {
            let typed = self.search.as_ref().map(|search| search.typed.clone()).unwrap_or_default();
            self.look_for(typed + text);
            return;
        }
        if let Some((zone, typed)) = self.filtered() {
            self.command(Command::Filter { zone: Some(zone), typed: typed + text });
            return;
        }
        let at = Instant::now();
        if self.typed.1.is_none_or(|last| at.duration_since(last) > TYPED_TOGETHER) {
            self.typed.0.clear();
        }
        self.typed.0.push_str(&text.to_lowercase());
        self.typed.1 = Some(at);
        self.layout();
        let among: Vec<ItemId> = match &self.gestures.anchor {
            Some(item) => self.layout.beside(item).iter().map(|shown| shown.item.clone()).collect(),
            None => self.layout.every_item().map(|shown| shown.item.clone()).collect(),
        };
        let begins = |item: &&ItemId| {
            self.core.item(item).is_some_and(|item| item.name.to_lowercase().starts_with(&self.typed.0))
        };
        if let Some(found) = among.iter().find(begins).cloned() {
            self.gestures.anchor = Some(found.clone());
            self.command(Command::Select { items: vec![found], mode: SelectMode::Replace });
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
                Act::Refuse(zone) => self.refused(&zone),
                Act::IntoFolder { items, folder } => {
                    match self.core.item(&folder).and_then(|folder| folder.path.clone()) {
                        Some(folder) => {
                            self.command(Command::MoveItemsToFolder { items, folder, copy: false });
                        }
                        None => log::warn!("{folder} is no folder that files could go into"),
                    }
                }
                Act::BringOut { items, zone, at, copy } => {
                    // (What is no file, a program that runs, has no path
                    // and stays where it is.)
                    let paths: Vec<String> = items
                        .iter()
                        .filter_map(|item| self.core.item(item).and_then(|item| item.path.clone()))
                        .collect();
                    if let (Some(folder), false) = (self.desktop_folder(), paths.is_empty()) {
                        self.command(Command::BringIn { paths, folder, copy, zone, at });
                    }
                }
                Act::ItemMenu { items, at, ours } => {
                    self.menu_at = at;
                    let own = self.desk.own_menu(&items);
                    match own {
                        Some(mut lines) => {
                            lines.extend(self.own_lines(&items));
                            self.asks.push(Ask::ItemMenu { items, at, lines })
                        }
                        // What the zones do with an item is not in the
                        // system's menu: with Shift, the menu is ours.
                        None if ours => {
                            let lines = self.own_lines(&items);
                            self.asks.push(Ask::ItemMenu { items, at, lines })
                        }
                        None => {
                            let point = gevrek_model::geometry::Point { x: at.x.round() as i32, y: at.y.round() as i32 };
                            if let Err(error) = self.desk.context_menu(&items, point) {
                                log::warn!("the menu could not be shown: {error}");
                            }
                        }
                    }
                }
                Act::ZoneMenu { zone, at } => self.asks.push(Ask::ZoneMenu { zone, at }),
                Act::ViewMenu { zone, at } => self.asks.push(Ask::ViewMenu { zone, at }),
                Act::DeskMenu { display, at } => self.asks.push(Ask::DeskMenu { display, at }),
                Act::DragOut(items) => {
                    if let Err(error) = self.desk.start_drag(&items, DragActions::ALL) {
                        log::warn!("the drag could not be handed to the system: {error}");
                    }
                }
            }
        }
    }

    /// A zone did not do what was asked of it with the pointer: it says
    /// so for a moment (it shakes, and its lock is marked).
    pub fn refused(&mut self, zone: &ZoneId) {
        // (Tabs are one thing on the desktop: the one in front shows it.)
        let shown = self.layout().shown_for(zone).map_or_else(|| zone.clone(), |shown| shown.zone.clone());
        let at = self.seconds();
        self.motion.refused(&shown, at);
        self.redraw = true;
        (self.wake_later)(Duration::ZERO);
    }

    /// Carry out a line of a menu the shell drew for some items.
    pub fn act(&mut self, items: &[ItemId], action: &str) {
        log::debug!("the menu's line {action} is carried out for {}", gevrek_zones::timed::count(items.len(), "item", "items"));
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
        if action == STAY {
            // All of them the same: the other of what the first is.
            let stays = items.first().is_some_and(|item| self.core.state().always_visible_items.contains(item));
            for item in items {
                self.command(Command::ItemAlwaysVisible { item: item.clone(), on: Some(!stays) });
            }
            return;
        }
        if action == LOOSEN {
            for item in items {
                self.command(Command::RemoveItem { item: item.clone() });
            }
            return;
        }
        if action == CHOOSE {
            // Every program they can be opened with, as a menu of its own
            // where the first one was.
            let lines = self.desk.programs(items);
            if !lines.is_empty() {
                self.asks.push(Ask::ItemMenu { items: items.to_vec(), at: self.menu_at, lines });
                self.redraw = true;
            }
            return;
        }
        if action == PROPERTIES {
            let lines = self.desk.properties(items);
            if !lines.is_empty() {
                self.asks.push(Ask::Sheet { heading: "Properties".to_string(), lines });
                self.redraw = true;
            }
            return;
        }
        if let Some(parent) = action.strip_prefix(NEW_FOLDER) {
            self.asks.push(Ask::NewFolderIn { parent: parent.to_string() });
            self.redraw = true;
            return;
        }
        if action == PIN {
            let pinned = self.desk.pinned();
            for program in items.iter().filter_map(|item| item.as_str().strip_prefix("app:")) {
                let pin = !pinned.iter().any(|known| known == program);
                if let Err(error) = self.desk.pin(program, pin) {
                    log::warn!("{program} could not be pinned or let go: {error}");
                }
            }
            (self.wake_later)(Duration::ZERO);
            return;
        }
        if let Err(error) = self.desk.act(items, action) {
            log::warn!("{action} could not be done: {error}");
        }
    }

    /// The lines of an item's menu that are the zones' own: whether it
    /// stays when the rest is put away, and out of its zone.
    fn own_lines(&self, items: &[ItemId]) -> Vec<DeskAction> {
        let stays = items.first().is_some_and(|item| self.core.state().always_visible_items.contains(item));
        let mut lines = vec![DeskAction {
            id: STAY.to_string(),
            label: if stays { "No longer stays when the rest is hidden" } else { "Stays when the rest is hidden" }.to_string(),
        }];
        if items.iter().any(|item| self.core.zone_of(item).is_some()) {
            lines.push(DeskAction { id: LOOSEN.to_string(), label: "Out of its zone".to_string() });
        }
        // A program among the launchers: pinned, or let go again.
        if let Some(program) = items.first().and_then(|item| item.as_str().strip_prefix("app:")) {
            let pinned = self.desk.pinned().iter().any(|known| known == program);
            lines.push(DeskAction {
                id: PIN.to_string(),
                label: if pinned { "No longer pinned" } else { "Pinned" }.to_string(),
            });
        }
        lines
    }

    /// Make a zone, and lay `changes` over what it is configured as
    /// (what it looks through, how it orders what it shows).
    pub fn make_zone(
        &mut self,
        title: &str,
        kind: gevrek_zones::config::ZoneKind,
        rect: Option<gevrek_zones::Rect>,
        changes: Option<serde_json::Value>,
    ) -> Option<ZoneId> {
        let effects = self.command(Command::CreateZone { title: title.to_string(), kind, rect });
        let zone = effects.iter().find_map(|effect| match effect {
            Effect::Notify(ZonesEvent::ZoneCreated { zone }) => Some(zone.clone()),
            _ => None,
        })?;
        if let Some(changes) = changes {
            self.command(Command::ModifyZone { zone: zone.clone(), changes });
        }
        Some(zone)
    }

    /// The system says the screen is shown to others, or no longer: the
    /// core covers what is not for other eyes, as the configuration says.
    pub fn screen_shared(&mut self, shared: bool) {
        let effects = self.core.event(BackendEvent::ScreenShared { shared }, now());
        self.take(effects);
        (self.wake_later)(Duration::ZERO);
    }

    /// Whether the surfaces are to be left out of pictures taken of the
    /// screen, where the system can do that: while something private is
    /// on them and the configuration wants private things kept from a
    /// shared screen. (The system does not say when the screen is
    /// shared; so what is private is never in a picture, also while the
    /// person has asked to see it.)
    pub fn kept_from_pictures(&self) -> bool {
        use gevrek_zones::config::WhenShared;
        let config = self.core.config();
        match config.privacy.when_shared {
            WhenShared::Nothing => false,
            WhenShared::Everything => true,
            WhenShared::Private => config.privacy.everything || config.zones.iter().any(|zone| zone.private),
        }
    }

    /// Something to be read by the person, on a sheet.
    pub fn say(&mut self, heading: &str, lines: Vec<(String, String)>) {
        self.asks.push(Ask::Sheet { heading: heading.to_string(), lines });
        self.redraw = true;
        (self.wake_later)(Duration::ZERO);
    }

    /// Write an arrangement to a file as one document: the kept one of
    /// that name, or without a name the one in use. The core makes the
    /// document; the file is written whole, beside its place first, so
    /// that a file that was there is never left half written.
    pub fn export_to(&mut self, name: Option<String>, path: &str) -> Result<(), String> {
        let effects = self.command(Command::ExportLayout { name });
        let document = effects.iter().find_map(|effect| match effect {
            Effect::Notify(ZonesEvent::LayoutExported { document }) => Some(document.to_value()),
            _ => None,
        });
        let Some(document) = document else {
            return Err(gevrek_zones::ipc::Answer::of(&effects).error.unwrap_or_else(|| "nothing was given to write".to_string()));
        };
        let text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
        let path = std::path::Path::new(path);
        let beside = path.with_extension("json.writing");
        let written = gevrek_zones::timed::timed(
            || format!("writing the zones' arrangement to {}", path.display()),
            || std::fs::write(&beside, text).and_then(|()| std::fs::rename(&beside, path)),
            gevrek_zones::timed::done,
        );
        written.map_err(|error| format!("{} could not be written: {error}", path.display()))
    }

    /// Take an arrangement from a file and put it in force. The core
    /// looks the document over first and keeps the arrangement there was;
    /// a file that cannot be right changes nothing. Says the name it is
    /// kept under.
    pub fn import_from(&mut self, path: &str) -> Result<String, String> {
        let text = gevrek_zones::timed::timed(
            || format!("reading an arrangement from {path}"),
            || std::fs::read_to_string(path),
            |read| gevrek_zones::timed::result(read, |text| format!("{} bytes", text.len())),
        )
        .map_err(|error| format!("{path} could not be read: {error}"))?;
        let document: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| format!("{path} is no arrangement: {error}"))?;
        let effects = self.command(Command::ImportLayout { document, apply: true });
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Notify(ZonesEvent::LayoutImported { name, .. }) => Some(name.clone()),
                _ => None,
            })
            .ok_or_else(|| gevrek_zones::ipc::Answer::of(&effects).error.unwrap_or_else(|| "it was not taken in".to_string()))
    }

    /// Make a folder in a folder that is named (the desktop's own, from
    /// the desktop's menu or an item's). Asked of the core, as a folder
    /// made in a portal is: it can be taken back like any other change
    /// to a file.
    pub fn new_folder_in(&mut self, parent: &str, name: &str) {
        self.command(Command::NewFolderIn { folder: parent.to_string(), name: name.to_string() });
        (self.wake_later)(Duration::ZERO);
    }

    /// What the rules would do now, for the person to look over: nothing
    /// is changed by asking.
    pub fn rules_preview(&self, scope: gevrek_zones::rules::Scope) -> RulesPreview {
        let plan = self.core.plan(scope, now());
        RulesPreview {
            scope,
            lines: self.core.explain(&plan),
            timed_every: self.core.timed_rules_every(),
            timed_move_files: self.core.config().timed_rules.move_files,
            can_undo: self.core.state().undo.is_some(),
        }
    }

    /// Ask for the box a zone's name is typed into, or what is looked for.
    pub fn ask(&mut self, ask: Ask) {
        self.asks.push(ask);
        // (A display's drawing opens it: one is asked for.)
        self.redraw = true;
    }

    /// Whether everything was hidden at once and is still (a double press
    /// on the bare desktop, the desktop's menu, a key): the zones and the
    /// loose items are away, and only what is always visible is left.
    pub fn hidden(&self) -> bool {
        self.core.quick_hidden()
    }

    /// Whether nothing of the zones is to be seen now: switched off, or
    /// hidden at once and not looked at for a moment. A menu or a box
    /// that was open for them has nothing left to be about.
    pub fn put_away(&self) -> bool {
        !self.core.enabled() || (self.core.quick_hidden() && !self.core.peeking())
    }

    /// The lines to show for the zone that is being moved or sized.
    fn watch_guides(&mut self) {
        let zone = match &self.gestures.doing {
            Doing::Moving { zone, .. } | Doing::Sizing { zone, .. } => zone.clone(),
            _ => {
                if !self.guides.is_empty() || !self.grid_marks.is_empty() {
                    self.guides.clear();
                    self.grid_marks.clear();
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
        let snap = &self.core.config().snap;
        let on = snap.enabled && self.gestures.tab_onto().is_none();
        self.guides =
            if on { view::guides(moved.rect, desk.area, desk.rect, &others, snap.gap as f32) } else { Vec::new() };
        self.grid_marks = if on { view::grid_marks(moved.rect, desk.area, snap.grid as f32) } else { Vec::new() };
    }

    /// The drag the surface is to begin now, if items were just taken up:
    /// the items, the paths of those that are files, and a picture to
    /// show under the pointer (the first item's, where it has come).
    pub fn take_drag_out(&mut self) -> Option<(Vec<ItemId>, Vec<String>, Option<Picture>)> {
        let (items, paths) = self.drag_out.take()?;
        log::debug!(
            "a drag of {} out of the zones begins ({} of them files)",
            gevrek_zones::timed::count(items.len(), "item", "items"),
            paths.len()
        );
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
        log::debug!(
            "the drag of {} out of the zones ended{}",
            gevrek_zones::timed::count(outgoing.len(), "item", "items"),
            if remove { "; whoever took them leaves the originals to the bin" } else { "" }
        );
        if remove {
            for (item, _) in outgoing {
                let Some(path) = self.core.item(&item).and_then(|item| item.path.clone()) else {
                    continue;
                };
                let operation = FileOperation::Trash { path };
                if let Err(error) = self.desk.perform(&operation) {
                    log::warn!("{operation} failed: {error}");
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
        self.tell_icons();
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
                self.pointer_over(None, false);
            }
            return none;
        };
        // (A rolled-up zone opens under what is carried over it.)
        let over = self.zone_at(at);
        self.pointer_over(over, true);
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
            // (A zone that takes nothing says so, and nothing is placed.)
            let placed = acts.iter().any(|act| matches!(act, Act::Do(_)));
            if placed {
                log::debug!("{} of the zones were let go, and are placed", gevrek_zones::timed::count(items.len(), "item", "items"));
            } else {
                log::debug!("{} of the zones were let go where nothing takes them", gevrek_zones::timed::count(items.len(), "item", "items"));
            }
            self.perform(acts);
            return if placed { Dropped::Placed } else { Dropped::Refused };
        }
        if payload.paths.is_empty() || !(payload.allowed.moving || payload.allowed.copy) {
            log::debug!("a drop of {} is refused: no files, or neither moved nor copied", payload.summary());
            return Dropped::Refused;
        }
        let moved = payload.allowed.moving;
        // Files from elsewhere: into a portal's folder as they are; onto
        // a zone filled by hand or the bare desktop, into the desktop's
        // folder first, and from there to where they were let go.
        let (zone, into) = match self.layout.target(at) {
            Target::Zone { zone, fills: Fills::Folder, locked: false, into, .. } => (zone, into),
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
            target => {
                log::debug!("a drop of {} is refused: {target:?} takes no files", payload.summary());
                return Dropped::Refused;
            }
        };
        // Into the folder the portal stands in (not its own, where it has
        // gone further in), or into a folder it shows that lies under the
        // pointer. Asked of the core, so that it can be taken back.
        let under = into.and_then(|folder| self.core.item(&folder).and_then(|folder| folder.path.clone()));
        let stands_in = self.layout.zone(&zone).and_then(|shown| shown.portal.as_ref()).map(|portal| portal.directory.clone());
        let Some(folder) = under.or(stands_in) else {
            log::debug!("a drop of {} on the portal {zone} is refused: it stands in no folder", payload.summary());
            return Dropped::Refused;
        };
        log::debug!(
            "a drop of {} on the portal {zone}: {} into {folder}",
            payload.summary(),
            if moved { "moved" } else { "copied" }
        );
        self.command(Command::BringIn { paths: payload.paths.clone(), folder, copy: !moved, zone: None, at: None });
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
            log::debug!("a drop of {} is refused: the desktop has no folder to bring files to", payload.summary());
            return Dropped::Refused;
        };
        let moved = payload.allowed.moving;
        log::debug!(
            "a drop of {} {}: {} into {folder}",
            payload.summary(),
            zone.as_ref().map_or_else(|| "on the desktop".to_string(), |zone| format!("on the zone {zone}")),
            if moved { "moved" } else { "copied" }
        );
        self.command(Command::BringIn { paths: payload.paths.clone(), folder, copy: !moved, zone, at });
        (self.wake_later)(Duration::ZERO);
        Dropped::Filed { moved }
    }
}

impl Drop for Zones {
    fn drop(&mut self) {
        log::debug!("the zones stop");
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
        assert!(zones.core().system_icons_away(), "the core is told");
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
        assert!(!zones.core().system_icons_away());
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
        assert!(!zones.core().system_icons_away());
        assert_eq!(zones.spots().len(), 1, "zones are shown all the same");
        zones.set_hide_icons(true);
        assert_eq!(zones.icons, SystemIcons::Away);
        assert!(zones.core().system_icons_away());
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
        zones.command(Command::Select { items: vec![id("b.txt")], mode: SelectMode::Replace });
        zones.key(gesture::Key::Delete);
        zones.tick();
        assert!(zones.core().item(&id("b.txt")).is_none());
        zones.command(Command::UndoFileOperation);
        zones.tick();
        assert!(zones.core().item(&id("b.txt")).is_some());
        // Nothing selected: the keys do nothing.
        zones.command(Command::Select { items: Vec::new(), mode: SelectMode::Replace });
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
    fn a_rolled_up_zone_opens_under_the_pointer_after_a_moment_and_shuts_when_it_left() {
        let (mut zones, woken) = zones_on(backend(), "roll");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        zones.command(Command::AddItem { zone: papers.clone(), item: id("a.txt"), index: None, cell: None });
        zones.command(Command::RollUp { zone: papers.clone(), rolled_up: Some(true) });
        let open = |zones: &mut Zones| zones.layout().zone(&papers).unwrap().expanded;
        let over = |x: f32, y: f32| Pointer { at: Some(pos2(x, y)), ..Pointer::default() };
        // Onto its title: nothing yet, and the zones ask to be woken
        // when the moment is over (no clock is looked at meanwhile).
        zones.pointer(&over(400.0, 112.0));
        assert!(!open(&mut zones));
        assert!(zones.tick_due.is_some());
        let _ = woken;
        zones.tick();
        assert!(!open(&mut zones), "woken too early: nothing");
        // Passed over and gone before the moment is over: it stays shut.
        zones.pointer(&over(1500.0, 900.0));
        zones.later(1.0);
        zones.tick();
        assert!(!open(&mut zones));
        // Rested on: open, with its items, and still rolled up.
        zones.pointer(&over(400.0, 112.0));
        zones.later(0.2);
        zones.tick();
        assert!(open(&mut zones));
        let shown = zones.layout().zone(&papers).unwrap().clone();
        assert!(shown.rolled_up && shown.items.len() == 1 && shown.rect.height() == 300.0);
        assert!(zones.core().config().zone(&papers).unwrap().rolled_up, "nothing that is kept changed");
        // Into what opened, out over its rim and back in time: open still.
        zones.pointer(&over(400.0, 300.0));
        zones.pointer(&over(1500.0, 900.0));
        zones.later(0.2);
        zones.tick();
        zones.pointer(&over(400.0, 300.0));
        zones.later(1.0);
        zones.tick();
        assert!(open(&mut zones));
        // Left for good: shut after a moment, and what it was is kept to
        // be drawn while it rolls shut.
        zones.pointer(&over(1500.0, 900.0));
        zones.later(0.5);
        zones.tick();
        assert!(!open(&mut zones));
        assert_eq!(zones.ghosts.get(&papers).map(|was| was.items.len()), Some(1));
        // The pointer leaving every surface of ours counts as leaving it.
        zones.pointer(&over(400.0, 112.0));
        zones.later(0.2);
        zones.tick();
        assert!(open(&mut zones));
        zones.pointer_left();
        zones.later(0.5);
        zones.tick();
        assert!(!open(&mut zones));
        // Something carried over its title opens it whichever way it opens.
        zones.command(Command::ModifySettings { changes: serde_json::json!({"roll_up": {"expand_on": "click"}}) });
        zones.pointer(&over(400.0, 112.0));
        zones.later(0.5);
        zones.tick();
        assert!(!open(&mut zones), "it opens at a press, not under the pointer");
        zones.pointer(&over(1500.0, 900.0));
        let carried = DropPayload { items: vec![id("b.txt")], allowed: DragActions::ALL, ..DropPayload::default() };
        zones.drag_over(Some(pos2(400.0, 112.0)), &carried);
        zones.later(0.2);
        zones.tick();
        assert!(open(&mut zones));
        assert_eq!(zones.drop_at(pos2(400.0, 300.0), &carried), Dropped::Placed);
        assert_eq!(zones.core().zone_of(&id("b.txt")), Some(&papers));
    }

    #[test]
    fn a_zone_that_refuses_says_so_and_what_is_opened_goes_through_the_core() {
        let desk = FakeDesk::new(backend());
        let said = std::sync::Arc::clone(&desk.said);
        let (mut zones, _) = zones_with(desk, "refuse");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        zones.command(Command::Lock { zone: papers.clone(), geometry: Some(true), members: Some(true) });
        // Dragged by its title: it stays, and shows that it is held.
        zones.pointer(&press(400.0, 112.0));
        zones.pointer(&hold(500.0, 212.0));
        assert_eq!(zones.core().zone_rect(&papers), Some(Rect::new(300, 100, 400, 300)));
        let now = zones.seconds();
        assert!(zones.motion.refusal(&papers, now).is_some() && zones.motion.busy(now));
        zones.pointer(&release(500.0, 212.0));
        zones.later(1.0);
        assert!(zones.motion.refusal(&papers, zones.seconds()).is_none() && !zones.motion.busy(zones.seconds()));
        // An item let go on it: not taken, and it says so again.
        let item = zones.layout().item(&id("a.txt")).unwrap().rect.center();
        zones.pointer(&press(item.x, item.y));
        zones.pointer(&hold(500.0, 250.0));
        zones.pointer(&release(500.0, 250.0));
        assert_eq!(zones.core().zone_of(&id("a.txt")), None);
        assert!(zones.motion.refusal(&papers, zones.seconds()).is_some());
        // With movement reduced it is marked, does not shake, and one
        // wake-up is asked for when the mark is to go.
        zones.motion.still = true;
        assert_eq!(zones.motion.refusal(&papers, zones.seconds()).map(|(aside, _)| aside), Some(0.0));
        assert!(!zones.motion.busy(zones.seconds()) && zones.motion.ends_in(zones.seconds()).is_some());
        // Enter opens what is chosen: asked of the core, which says when
        // it is to be started; only then is the system asked.
        zones.command(Command::Select { items: vec![id("b.txt")], mode: SelectMode::Replace });
        let before = said.lock().unwrap().len();
        zones.key(gesture::Key::Enter);
        assert_eq!(said.lock().unwrap()[before..], ["open /desk/b.txt".to_string()]);
        assert!(zones.take_heard().iter().any(|event| matches!(event, ZonesEvent::OpenRequested { .. })));
    }

    #[test]
    fn letters_typed_in_a_zone_leave_the_items_that_fit_and_the_last_can_be_taken_back() {
        let (mut zones, _) = zones_on(backend(), "filter");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        for name in ["a.txt", "b.txt"] {
            zones.command(Command::AddItem { zone: papers.clone(), item: id(name), index: None, cell: None });
        }
        zones.command(Command::FocusZone { zone: Some(papers.clone()) });
        zones.typed("b");
        let shown = zones.layout().zone(&papers).unwrap().clone();
        assert_eq!((shown.filter.as_deref(), shown.items.len(), shown.item_count), (Some("b"), 1, 2));
        zones.typed(".t");
        assert_eq!(zones.layout().zone(&papers).unwrap().filter.as_deref(), Some("b.t"));
        zones.key(gesture::Key::Erase);
        zones.key(gesture::Key::Erase);
        zones.key(gesture::Key::Erase);
        assert_eq!(zones.layout().zone(&papers).unwrap().filter, None);
        assert_eq!(zones.layout().zone(&papers).unwrap().items.len(), 2);
        zones.key(gesture::Key::Erase);
        // Escape empties it too (the core's nearest thing to let go of).
        zones.typed("a");
        zones.key(gesture::Key::Escape);
        assert_eq!(zones.layout().zone(&papers).unwrap().filter, None);
    }

    #[test]
    fn what_stays_when_the_rest_is_put_away_is_laid_out_where_it_was() {
        let (mut zones, _) = zones_on(backend(), "stays");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let keeps = zone(&mut zones, "Keeps", ZoneKind::Manual, (800, 100, 300, 200));
        zones.command(Command::AddItem { zone: papers.clone(), item: id("a.txt"), index: None, cell: None });
        let was = zones.layout().item(&id("a.txt")).unwrap().rect;
        zones.command(Command::AlwaysVisible { zone: keeps.clone(), on: Some(true) });
        zones.act(&[id("a.txt")], STAY);
        zones.command(Command::QuickHide { hidden: Some(true) });
        let layout = zones.layout().clone();
        // The zone that stays, and of the other only the item that does.
        assert_eq!(layout.zones.iter().map(|zone| zone.zone.clone()).collect::<Vec<_>>(), [keeps]);
        let desk = &layout.desks[0];
        assert!(desk.items.is_empty(), "the loose item is put away");
        assert_eq!(desk.stays.len(), 1);
        assert_eq!((desk.stays[0].shown.rect, &desk.stays[0].zone), (was, &papers));
        assert_eq!(layout.hit(was.center()), view::Hit::Item { item: id("a.txt"), zone: Some(papers.clone()) });
        // The same line of its menu takes it back; and out of its zone.
        let lines = zones.own_lines(&[id("a.txt")]);
        assert_eq!(lines.iter().map(|line| line.id.as_str()).collect::<Vec<_>>(), [STAY, LOOSEN]);
        assert!(lines[0].label.starts_with("No longer"));
        zones.act(&[id("a.txt")], STAY);
        assert!(zones.layout().desks[0].stays.is_empty());
        zones.command(Command::QuickHide { hidden: Some(false) });
        zones.act(&[id("a.txt")], LOOSEN);
        assert_eq!(zones.core().zone_of(&id("a.txt")), None);
    }

    /// A portal on a folder that has a file and a folder with a file in it.
    fn with_a_portal(name: &str) -> (Zones, ZoneId) {
        let mut backend = backend();
        backend.add_file_in("/archive", "old.txt");
        backend.add_folder_in("/archive", "sub");
        backend.add_file_in("/archive/sub", "in.txt");
        let (mut zones, _) = zones_on(backend, name);
        let archive = zone(&mut zones, "Archive", ZoneKind::Portal { directory: "/archive".to_string() }, (800, 100, 400, 300));
        zones.tick();
        (zones, archive)
    }

    fn names_in(zones: &mut Zones, zone: &ZoneId) -> Vec<String> {
        zones.layout().zone(zone).unwrap().items.iter().map(|item| item.name.clone()).collect()
    }

    #[test]
    fn a_portal_goes_into_its_folders_in_place_and_back_by_its_title() {
        let (mut zones, archive) = with_a_portal("portal-way");
        assert_eq!(names_in(&mut zones, &archive), ["old.txt", "sub"]);
        assert!(zones.layout().zone(&archive).unwrap().nav.is_empty(), "in its own folder its title is its name");
        // Twice on a folder: into it, in the same zone.
        let folder = zones.layout().item(&ItemId::new("/archive/sub")).unwrap().rect.center();
        zones.pointer(&Pointer { double: true, ..press(folder.x, folder.y) });
        zones.pointer(&release(folder.x, folder.y));
        zones.tick();
        assert_eq!(names_in(&mut zones, &archive), ["in.txt"]);
        let shown = zones.layout().zone(&archive).unwrap().clone();
        assert_eq!(shown.portal.as_ref().unwrap().directory, "/archive/sub");
        let way: Vec<&view::Nav> = shown.nav.iter().map(|part| &part.nav).collect();
        assert_eq!(
            way,
            [&view::Nav::Back, &view::Nav::Up, &view::Nav::To("/archive".to_string()), &view::Nav::To("/archive/sub".to_string())]
        );
        // A press on the first folder of the way goes there.
        let first = shown.nav[2].rect.center();
        assert_eq!(zones.layout().hit(first), view::Hit::Nav(archive.clone(), view::Nav::To("/archive".to_string())));
        zones.pointer(&press(first.x, first.y));
        zones.pointer(&release(first.x, first.y));
        zones.tick();
        assert_eq!(names_in(&mut zones, &archive), ["old.txt", "sub"]);
        // And back where it was, by the way back.
        let back = zones.layout().zone(&archive).unwrap().nav[0].clone();
        assert_eq!(back.nav, view::Nav::Back);
        zones.pointer(&press(back.rect.center().x, back.rect.center().y));
        zones.pointer(&release(back.rect.center().x, back.rect.center().y));
        zones.tick();
        assert_eq!(names_in(&mut zones, &archive), ["in.txt"]);
        // Dragged by a part of its way, the zone is moved, not sent anywhere.
        let up = zones.layout().zone(&archive).unwrap().nav[1].rect.center();
        zones.pointer(&press(up.x, up.y));
        zones.pointer(&hold(up.x - 200.0, up.y + 200.0));
        zones.pointer(&release(up.x - 200.0, up.y + 200.0));
        assert_eq!(zones.core().zone_rect(&archive).map(|rect| (rect.x, rect.y)), Some((600, 300)));
        assert_eq!(names_in(&mut zones, &archive), ["in.txt"]);
        // What is where it stands is where it stands: no way to go.
        let here = zones.layout().zone(&archive).unwrap().nav[3].rect.center();
        assert_eq!(zones.layout().hit(here), view::Hit::Head(archive));
    }

    #[test]
    fn files_go_into_a_portals_folders_and_out_of_them_and_each_can_be_taken_back() {
        let (mut zones, archive) = with_a_portal("portal-files");
        let carry = |zones: &mut Zones, item: &str, to: Pos2| {
            let from = zones.layout().item(&ItemId::new(item)).unwrap_or_else(|| panic!("{item} is shown")).rect.center();
            zones.pointer(&press(from.x, from.y));
            zones.pointer(&hold(to.x, to.y));
            zones.pointer(&release(to.x, to.y));
            zones.tick();
        };
        // A file of the desktop onto a folder the portal shows: into that folder.
        let onto = zones.layout().item(&ItemId::new("/archive/sub")).unwrap().rect.center();
        carry(&mut zones, "/desk/a.txt", onto);
        assert!(zones.core().item(&id("a.txt")).is_none(), "it is no longer on the desktop");
        assert_eq!(names_in(&mut zones, &archive), ["old.txt", "sub"]);
        zones.command(Command::EnterFolder { zone: archive.clone(), item: ItemId::new("/archive/sub") });
        zones.tick();
        assert_eq!(names_in(&mut zones, &archive), ["a.txt", "in.txt"]);
        // Ctrl+Z: back on the desktop.
        zones.key(gesture::Key::Undo);
        zones.tick();
        assert!(zones.core().item(&id("a.txt")).is_some());
        assert_eq!(names_in(&mut zones, &archive), ["in.txt"]);
        // A file of the portal onto the bare desktop: the file itself
        // comes to the desktop's folder, and lies where it was let go.
        carry(&mut zones, "/archive/sub/in.txt", pos2(300.0, 700.0));
        let came = zones.layout().item(&id("in.txt")).expect("it is on the desktop now").rect;
        assert!(came.contains(pos2(310.0, 710.0)) || (came.min.x - 300.0).abs() < 60.0, "{came:?}");
        assert_eq!(names_in(&mut zones, &archive), Vec::<String>::new());
        // A file of another program let go on the portal goes into the
        // folder it stands in, not into its own.
        let foreign = DropPayload {
            paths: vec!["/elsewhere/c.txt".to_string()],
            allowed: DragActions { copy: true, ..DragActions::default() },
            ..DropPayload::default()
        };
        assert_eq!(zones.drop_at(pos2(1000.0, 300.0), &foreign), Dropped::Filed { moved: false });
        zones.tick();
        assert_eq!(names_in(&mut zones, &archive), ["c.txt"]);
        zones.command(Command::NavigateUp { zone: archive.clone() });
        zones.tick();
        assert_eq!(names_in(&mut zones, &archive), ["old.txt", "sub"]);
        // A folder made in it, by the name typed.
        zones.command(Command::NewFolder { zone: archive.clone(), name: "Letters".to_string() });
        zones.tick();
        assert!(names_in(&mut zones, &archive).contains(&"Letters".to_string()));
        // A folder made on the desktop itself is an item of the desktop,
        // and Ctrl+Z takes it back as it takes back anything else.
        let desktop = zones.desktop_known().expect("the backend names the desktop's folder");
        zones.new_folder_in(&desktop, "Sorted");
        zones.tick();
        let made = |zones: &mut Zones| zones.layout().every_item().any(|shown| shown.name == "Sorted");
        assert!(made(&mut zones));
        zones.key(gesture::Key::Undo);
        zones.tick();
        assert!(!made(&mut zones));
    }

    #[test]
    fn what_the_rules_would_do_is_shown_before_it_is_done_and_taken_back_after() {
        use gevrek_zones::rules::{Destination, Reason, Scope};
        let mut zones = crate::zones::draw::tests::a_desk_with_rules("rules-preview");
        let preview = zones.rules_preview(Scope::All);
        let of = |name: &str| preview.lines.iter().find(|line| line.name == name).unwrap_or_else(|| panic!("{name}"));
        assert!(matches!(&of("Holiday.png").destination, Destination::Zone { title, .. } if title == "Pictures"));
        assert_eq!(of("Holiday.png").rule, "Pictures");
        assert!(matches!(&of("setup.exe").destination, Destination::Folder { folder } if folder == "/home/ada/Archive"));
        assert_eq!(of("main.rs").held, Some(Reason::Locked));
        assert_eq!(of("song.mp3").held, Some(Reason::NoSuchZone));
        // A rule asks how old a file is: it is said how often such are tried.
        assert_eq!(preview.timed_every, Some(3600));
        assert!(!preview.timed_move_files && !preview.can_undo);
        // Nothing was done by looking.
        assert!(zones.core().loose_items().iter().any(|item| item.name == "Holiday.png"));
        // Done, and taken back.
        zones.command(Command::ApplyRules { scope: Scope::All });
        zones.tick();
        let after = zones.rules_preview(Scope::All);
        assert!(after.can_undo);
        assert!(after.lines.iter().all(|line| line.held.is_some() || line.name != "Holiday.png"), "{:?}", after.lines);
        assert!(!zones.core().loose_items().iter().any(|item| item.name == "Holiday.png"));
        zones.command(Command::UndoRules);
        zones.tick();
        assert!(zones.core().loose_items().iter().any(|item| item.name == "Holiday.png"));
        assert_eq!(zones.rules_preview(Scope::All).lines.len(), preview.lines.len());
    }

    #[test]
    fn the_zones_that_fill_themselves_are_made_with_what_they_look_through() {
        use gevrek_zones::config::SmartSource;
        let (mut zones, _) = zones_on(backend(), "smart-menu");
        let lines = surfaces::smart_menu(pos2(300.0, 200.0));
        let mut made = Vec::new();
        for line in lines {
            let surfaces::Line::Choice { does: surfaces::Does::Make { title, kind, rect, changes }, .. } = line else {
                panic!("every line makes a zone");
            };
            let zone = zones.make_zone(&title, kind, rect, changes).expect("the zone is made");
            let definition = zones.core().config().zone(&zone).unwrap().clone();
            assert!(matches!(definition.kind, ZoneKind::Smart { .. }));
            made.push((definition.title.clone(), definition.source.clone()));
            // It is where the menu was asked for, and shown.
            assert!(zones.layout().zone(&zone).is_some());
        }
        assert_eq!(
            made,
            [
                ("Recent".to_string(), SmartSource::Recent),
                ("Downloaded today".to_string(), SmartSource::Downloads),
                ("Running".to_string(), SmartSource::RunningApps),
                ("Pinned".to_string(), SmartSource::Pinned),
                ("Pictures".to_string(), SmartSource::Desktop),
            ]
        );
        assert_eq!(zones.core().config().problems(), []);
        // A zone that looks through something else than the desktop is
        // known as such: what is taken out of it is brought, not put.
        let recent = zones.core().config().zones[0].id.clone();
        assert!(zones.layout().zone(&recent).unwrap().outside);
        assert_eq!(surfaces::portal_title("/home/ada/Downloads/"), "Downloads");
        assert_eq!(surfaces::portal_title(r"C:\Users\ada\Documents"), "Documents");
        assert_eq!(surfaces::portal_title("/"), "/");
    }

    #[test]
    fn what_takes_long_is_shown_on_its_zone_and_can_be_called_off() {
        let mut backend = backend();
        backend.slow_operations = true;
        let (mut zones, _) = zones_on(backend, "long");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let foreign = DropPayload {
            paths: vec!["/elsewhere/c.txt".to_string()],
            allowed: DragActions { copy: true, ..DragActions::default() },
            ..DropPayload::default()
        };
        // Into a zone: the zone says that something is under way.
        assert_eq!(zones.drop_at(pos2(500.0, 250.0), &foreign), Dropped::Filed { moved: false });
        zones.tick();
        let busy = zones.layout().zone(&papers).unwrap().busy.clone().expect("the copy is under way");
        assert_eq!((busy.what.as_str(), busy.part), ("Copying c.txt", None));
        assert!(zones.layout().under_way.is_empty(), "said on the zone, not at the desktop's foot");
        assert!(zones.core().item(&id("c.txt")).is_none(), "not there before it has ended");
        // Called off: nothing came, and nothing is under way.
        zones.command(Command::CancelOperation { request: busy.request });
        zones.tick();
        assert!(zones.layout().zone(&papers).unwrap().busy.is_none());
        assert!(zones.core().item(&id("c.txt")).is_none());
        // Onto the bare desktop: said at the desktop's foot.
        assert_eq!(zones.drop_at(pos2(1500.0, 900.0), &foreign), Dropped::Filed { moved: false });
        zones.tick();
        assert_eq!(zones.layout().under_way.iter().map(|busy| busy.what.as_str()).collect::<Vec<_>>(), ["Copying c.txt"]);
    }

    #[test]
    fn a_refusal_that_came_by_a_key_or_from_outside_shows_on_its_zone_too() {
        let (mut zones, _) = zones_on(backend(), "refused-about");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        zones.command(Command::Lock { zone: papers.clone(), geometry: Some(true), members: None });
        zones.layout();
        assert!(zones.motion.refusal(&papers, zones.seconds()).is_none());
        // As `gevrek zones send` or a bound key would ask.
        let effects = zones.command(Command::MoveZone { zone: papers.clone(), x: 10, y: 10, snap: false });
        assert!(!gevrek_zones::ipc::Answer::of(&effects).ok);
        assert!(zones.motion.refusal(&papers, zones.seconds()).is_some());
        // A refusal about a zone that is not shown shakes nothing.
        zones.later(1.0);
        zones.command(Command::RollUp { zone: ZoneId::new("no-such"), rolled_up: None });
        assert!(!zones.motion.busy(zones.seconds()));
    }

    #[test]
    fn an_arrangement_is_kept_gone_back_to_written_to_a_file_and_taken_from_one() {
        let (mut zones, _) = zones_on(backend(), "arrangements");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        zones.command(Command::AddItem { zone: papers.clone(), item: id("a.txt"), index: None, cell: None });
        let labels = |zones: &Zones| -> Vec<String> {
            surfaces::layouts_menu(zones)
                .into_iter()
                .filter_map(|line| match line {
                    surfaces::Line::Choice { label, .. } | surfaces::Line::Sub { label, .. } => Some(label),
                    surfaces::Line::Gap => None,
                })
                .collect()
        };
        assert_eq!(
            labels(&zones),
            ["Keep this arrangement as\u{2026}", "Write this arrangement to a file\u{2026}", "Take an arrangement from a file\u{2026}"]
        );
        // Kept under a name; then the zone is moved and the item taken out.
        zones.command(Command::SaveSnapshot { name: "Tidy".to_string() });
        // (What can be done with it is a menu beside its line.)
        assert!(labels(&zones).contains(&"\u{201c}Tidy\u{201d}".to_string()));
        assert!(surfaces::layouts_menu(&zones).iter().any(
            |line| matches!(line, surfaces::Line::Sub { label, lines } if label == "\u{201c}Tidy\u{201d}" && lines == &surfaces::layout_menu("Tidy"))
        ));
        zones.command(Command::MoveZone { zone: papers.clone(), x: 900, y: 500, snap: false });
        zones.command(Command::RemoveItem { item: id("a.txt") });
        // Back to it, by the line of its menu.
        let surfaces::Line::Choice { does: surfaces::Does::Command(back), .. } = &surfaces::layout_menu("Tidy")[0] else {
            panic!("the first line goes back to it");
        };
        zones.command(back.clone());
        assert_eq!(zones.core().zone_rect(&papers), Some(Rect::new(300, 100, 400, 300)));
        assert_eq!(zones.core().zone_of(&id("a.txt")), Some(&papers));
        // What there was before going back was kept too, by the zones themselves.
        assert!(zones.core().state().snapshots.iter().any(|snapshot| snapshot.automatic));
        // Written to a file, whole.
        let folder = std::env::temp_dir().join(format!("gevrek-shell-zones-export-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("tidy.json");
        let path = file.to_string_lossy().into_owned();
        assert_eq!(zones.export_to(Some("Tidy".to_string()), &path), Ok(()));
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(written["format"], "gevrek-zones-layout");
        assert_eq!(written["layout"]["name"], "Tidy");
        assert!(!folder.join("tidy.json.writing").exists(), "nothing half written is left beside it");
        // One that is not there cannot be written, and no file is made.
        let none = folder.join("none.json");
        assert!(zones.export_to(Some("No such".to_string()), &none.to_string_lossy()).is_err());
        assert!(!none.exists());
        // Taken from the file again after everything was changed: in
        // force, kept under a name of its own, and what there was kept.
        zones.command(Command::DeleteZone { zone: papers.clone() });
        assert!(zones.layout().zones.is_empty());
        let name = zones.import_from(&path).expect("the file is an arrangement");
        assert_eq!(name, "Tidy (2)");
        assert_eq!(zones.layout().zones.len(), 1);
        assert_eq!(zones.core().zone_of(&id("a.txt")), Some(&papers));
        // A file that is no arrangement changes nothing, and says why.
        std::fs::write(&none, "{\"format\": \"something else\"}").unwrap();
        let before = zones.core().config().clone();
        let why = zones.import_from(&none.to_string_lossy()).expect_err("it is no arrangement");
        assert!(!why.is_empty());
        assert_eq!(zones.core().config(), &before);
        assert!(zones.import_from(&folder.join("missing.json").to_string_lossy()).is_err());
        // What came of writing or taking in is said on a sheet.
        zones.say("The arrangement was written", vec![("File".to_string(), path.clone())]);
        assert!(matches!(zones.take_asks().as_slice(), [Ask::Sheet { heading, .. }] if heading == "The arrangement was written"));
        // The file offered lies beside the desktop's folder, not in it.
        assert_eq!(surfaces::export_path(Some("/home/ada/Desktop"), Some("Tidy up!")), "/home/ada/gevrek-zones-tidy-up.json");
        assert_eq!(surfaces::export_path(Some(r"C:\Users\ada\Desktop"), None), r"C:\Users\ada\gevrek-zones-now.json");
        assert_eq!(surfaces::export_path(None, None), "gevrek-zones-now.json");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn another_workspace_shows_its_own_zones_at_once_and_nothing_fades() {
        let (mut zones, _) = zones_on(backend(), "workspaces");
        let everywhere = zone(&mut zones, "Everywhere", ZoneKind::Manual, (300, 100, 400, 300));
        let code = zone(&mut zones, "Code", ZoneKind::Manual, (800, 100, 300, 200));
        zones.command(Command::ModifyZone { zone: code.clone(), changes: serde_json::json!({"workspaces": ["dev"]}) });
        let shown = |zones: &mut Zones| zones.layout().zones.iter().map(|zone| zone.zone.clone()).collect::<Vec<_>>();
        // A workspace that was never told hides nothing.
        assert_eq!(shown(&mut zones), [everywhere.clone(), code.clone()]);
        zones.told(&serde_json::json!({ "monitors": [] }), None);
        zones.workspace(Some("mail".to_string()));
        assert_eq!(shown(&mut zones), std::slice::from_ref(&everywhere));
        // It is a change of what is there, not a hiding: at once, with
        // nothing fading and nothing on its way.
        assert!(zones.fading.is_none() && !zones.motion.busy(zones.seconds()));
        zones.workspace(Some("dev".to_string()));
        assert_eq!(shown(&mut zones), [everywhere.clone(), code.clone()]);
        assert!(zones.fading.is_none());
        // Each workspace its own arrangement: a zone moved on the one
        // is where it was on the other.
        zones.command(Command::ModifySettings { changes: serde_json::json!({"profile_per_workspace": true}) });
        // (A workspace seen for the first time begins as the one before it was.)
        zones.workspace(Some("mail".to_string()));
        zones.workspace(Some("dev".to_string()));
        zones.command(Command::MoveZone { zone: everywhere.clone(), x: 40, y: 500, snap: false });
        zones.workspace(Some("mail".to_string()));
        assert_eq!(zones.core().zone_rect(&everywhere), Some(Rect::new(300, 100, 400, 300)));
        zones.workspace(Some("dev".to_string()));
        assert_eq!(zones.core().zone_rect(&everywhere), Some(Rect::new(40, 500, 400, 300)));
        // A display that comes has a surface asked for it, and one that
        // goes has none.
        assert_eq!(zones.spots().len(), 1);
    }

    #[test]
    fn a_folder_that_is_away_is_asked_for_again_after_a_while_and_when_its_zone_comes_into_view() {
        use gevrek_zones::portal::Listing;
        let mut backend = backend();
        backend.add_folder("//nas/films");
        backend.set_reachable("//nas/films", false);
        let (mut zones, _) = zones_on(backend, "away");
        let films = zone(&mut zones, "Films", ZoneKind::Portal { directory: "//nas/films".to_string() }, (300, 100, 400, 300));
        zones.tick();
        let away = |zones: &mut Zones| matches!(zones.layout().zone(&films).unwrap().listing, Some(Listing::Unavailable { .. }));
        assert!(away(&mut zones));
        let asked_again = |zones: &mut Zones| {
            zones.take_heard().iter().any(|event| matches!(event, ZonesEvent::ListingChanged { zone, .. } if zone == &films))
        };
        // Seen to be away: a wait begins, and nothing is asked meanwhile.
        zones.tick();
        let due = zones.retry_due.expect("a wait began");
        assert!(due > Instant::now() + Duration::from_secs(50));
        zones.take_heard();
        zones.tick();
        zones.tick();
        assert!(!asked_again(&mut zones) && zones.retry_due == Some(due), "not before its time, and one wait at a time");
        // The wait is over: asked again (still away), and the next wait begins.
        zones.retry_due = Some(Instant::now() - Duration::from_secs(1));
        zones.tick();
        assert!(asked_again(&mut zones) && away(&mut zones));
        assert!(zones.retry_due.is_some_and(|due| due > Instant::now() + Duration::from_secs(50)));
        // Put away, nothing waits for it and nothing is asked.
        zones.command(Command::QuickHide { hidden: Some(true) });
        zones.tick();
        zones.tick();
        assert!(zones.retry_due.is_none());
        zones.take_heard();
        zones.tick();
        assert!(!asked_again(&mut zones));
        // Back in view: asked again with the next look, not a minute later.
        zones.command(Command::QuickHide { hidden: Some(false) });
        zones.tick();
        assert!(asked_again(&mut zones));
        // A zone whose folder is there starts no wait at all.
        let (mut zones, _) = zones_on(self::backend(), "not-away");
        zone(&mut zones, "Archive", ZoneKind::Portal { directory: "/archive".to_string() }, (300, 100, 400, 300));
        zones.tick();
        zones.tick();
        assert!(zones.retry_due.is_none());
    }

    #[test]
    fn of_a_drags_many_movements_whoever_listens_is_told_where_the_zone_came_to_rest() {
        let (mut zones, _) = zones_on(backend(), "coalesce");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let other = zone(&mut zones, "Other", ZoneKind::Manual, (800, 100, 300, 200));
        zones.take_heard();
        for step in 0..40 {
            zones.command(Command::MoveZone { zone: papers.clone(), x: 300 + step, y: 400, snap: false });
        }
        zones.command(Command::MoveZone { zone: other.clone(), x: 900, y: 500, snap: false });
        zones.command(Command::MoveZone { zone: papers.clone(), x: 10, y: 10, snap: false });
        let placed: Vec<(ZoneId, i32)> = zones
            .take_heard()
            .into_iter()
            .filter_map(|event| match event {
                ZonesEvent::ZonePlaced { zone, rect } => Some((zone, rect.x)),
                _ => None,
            })
            .collect();
        // One for the run of forty, the other zone's, and the one after it.
        assert_eq!(placed, [(papers.clone(), 339), (other, 900), (papers, 10)]);
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

    #[test]
    fn what_a_button_of_a_title_does_is_said_when_the_pointer_has_rested_on_it() {
        let (mut zones, _) = zones_on(backend(), "hint");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let display = DisplayId::new("fake-1");
        let placed = |zones: &mut Zones, button: view::Button| {
            zones.layout().zone(&papers).unwrap().buttons.iter().find(|placed| placed.button == button).unwrap().rect
        };
        let over = |rect: egui::Rect| Pointer { at: Some(rect.center()), ..Pointer::default() };
        let roll = placed(&mut zones, view::Button::Roll);
        zones.pointer(&over(roll));
        // Not at once: only when the pointer has rested there.
        assert_eq!(zones.hint_on(&display), None);
        assert!(zones.hint_in().is_some_and(|left| left > 0.0 && left <= 0.6), "{:?}", zones.hint_in());
        zones.later(0.3);
        zones.pointer(&over(roll));
        assert_eq!(zones.hint_on(&display), None, "moving on the same button does not begin anew, and is not yet long enough");
        zones.later(0.4);
        assert_eq!(zones.hint_on(&display), Some((roll, "Roll up".to_string())));
        assert_eq!(zones.hint_in(), None, "it is said: nothing waits");
        // On to its neighbour: that one's, after a moment of its own.
        let lock = placed(&mut zones, view::Button::Lock);
        zones.pointer(&over(lock));
        assert_eq!(zones.hint_on(&display), None);
        zones.later(0.7);
        assert_eq!(zones.hint_on(&display), Some((lock, "Hold it where it is".to_string())));
        // What it says goes by how the zone stands now.
        zones.command(Command::Lock { zone: papers.clone(), geometry: Some(true), members: None });
        assert!(zones.hint_on(&display).is_some_and(|(_, words)| words.starts_with("Held where it is")));
        // Pressed, it says nothing (the press is what it does).
        zones.pointer(&Pointer { pressed: true, down: true, ..over(lock) });
        assert_eq!(zones.hint_on(&display), None);
        zones.pointer(&Pointer { released: true, ..over(lock) });
        // Off the buttons, and off every surface: nothing.
        zones.later(1.0);
        zones.pointer(&Pointer { at: Some(pos2(400.0, 300.0)), ..Pointer::default() });
        assert_eq!(zones.hint_on(&display), None);
        zones.pointer(&over(roll));
        zones.later(1.0);
        zones.pointer_left();
        assert_eq!(zones.hint_on(&display), None);
        // On another display's surface it is not said.
        zones.pointer(&over(roll));
        zones.later(1.0);
        assert_eq!(zones.hint_on(&DisplayId::new("no-such")), None);
    }

    #[test]
    fn the_pointer_has_the_shape_of_what_is_under_way() {
        use egui::CursorIcon;
        let (mut zones, _) = zones_on(backend(), "shape");
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        assert_eq!(zones.pointer_shape(pos2(698.0, 250.0)), CursorIcon::ResizeHorizontal);
        // Taken by its right side and pulled far beyond it: still the
        // arrows of that side, wherever the pointer is by now.
        zones.pointer(&press(698.0, 250.0));
        zones.pointer(&hold(760.0, 300.0));
        assert_eq!(zones.pointer_shape(pos2(900.0, 600.0)), CursorIcon::ResizeHorizontal);
        assert_eq!(zones.grip_shown().map(|(zone, edges)| (zone, edges.right)), Some((papers.clone(), true)));
        zones.pointer(&release(760.0, 300.0));
        assert!(zones.core().zone_rect(&papers).unwrap().width > 440);
        // Moved by its title: the four arrows.
        zones.pointer(&press(400.0, 112.0));
        zones.pointer(&hold(450.0, 160.0));
        assert_eq!(zones.pointer_shape(pos2(450.0, 160.0)), CursorIcon::Move);
        assert_eq!(zones.grip_shown(), None, "no side is shown while something else is under way");
        zones.pointer(&release(450.0, 160.0));
    }

    #[test]
    fn a_trace_says_every_press_what_it_hit_and_what_was_decided() {
        let (mut zones, _) = zones_on(backend(), "trace-file");
        let file = std::env::temp_dir().join(format!("gevrek-shell-zones-trace-{}.log", std::process::id()));
        std::fs::write(&file, "").unwrap();
        zones.trace = true;
        zones.trace_file = Some(file.clone());
        zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
        let main = egui::PointerButton::Primary;
        let press_seen = |at: Pos2, time_ms: u32| gevrek_shellkit::Press { button: main, at, time_ms };
        // A press on the rim and a drag: sized.
        assert!(!zones.pressed_twice(press_seen(pos2(698.0, 398.0), 1_000), 500, 6.0));
        zones.pointer(&press(698.0, 398.0));
        zones.pointer(&hold(740.0, 430.0));
        zones.pointer(&release(740.0, 430.0));
        // Twice on the bare desktop.
        zones.pressed_twice(press_seen(pos2(1500.0, 900.0), 5_000), 500, 6.0);
        zones.pointer(&press(1500.0, 900.0));
        zones.pointer(&release(1500.0, 900.0));
        assert!(zones.pressed_twice(press_seen(pos2(1501.0, 900.0), 5_180), 500, 6.0));
        zones.pointer(&Pointer { double: true, ..press(1501.0, 900.0) });
        let written = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        let lines: Vec<&str> = written.lines().collect();
        // Each line says when; the presses say the system's time, which
        // one of a pair they are, and what a double press is here.
        assert!(lines.iter().all(|line| line.starts_with('+') && line.contains("s ")), "{written}");
        assert!(lines[0].contains("press Primary at 698,398 system-time 1000 ms: a first press"), "{written}");
        assert!(lines[0].contains("within 500 ms and 6 points"), "{written}");
        // What the press hit, and what was decided of the drag and the letting go.
        let has = |parts: &[&str]| lines.iter().any(|line| parts.iter().all(|part| line.contains(part)));
        assert!(has(&["pressed=true", "hit Some(Edge(", "now Pressed"]), "{written}");
        assert!(has(&["released=true", "now Nothing"]), "{written}");
        assert!(has(&["system-time 5180 ms: the SECOND of a double press"]), "{written}");
        assert!(has(&["double=true", "hit Some(Desk(", "QuickHide"]), "{written}");
        // (A movement alone is not written: a drag would fill the file.)
        assert!(!has(&["pressed=false", "released=false", "menu=false", "middle-double=false"]), "{written}");
    }

    #[test]
    fn a_trace_of_the_presses_changes_nothing_of_what_they_do() {
        let run = |trace: bool| {
            let (mut zones, _) = zones_on(backend(), if trace { "traced" } else { "untraced" });
            zones.trace = trace;
            let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (300, 100, 400, 300));
            zones.pointer(&press(698.0, 398.0));
            zones.pointer(&hold(740.0, 430.0));
            zones.pointer(&release(740.0, 430.0));
            zones.pointer(&Pointer { double: true, ..press(1500.0, 900.0) });
            zones.pointer(&Pointer { middle_double: true, at: Some(pos2(1500.0, 900.0)), ..Pointer::default() });
            (zones.core().zone_rect(&papers), zones.hidden(), zones.core().grey_icons())
        };
        assert_eq!(run(true), run(false));
        let (rect, hidden, grey) = run(true);
        assert!(rect.is_some_and(|rect| rect.width > 420) && hidden && grey);
    }
}
