//! The Windows half of the shell: the panel, the launcher, gevrek's own
//! notifications with their history, the on-screen display, the power
//! menu and the list of shortcuts.
//!
//! The panel of each display is a surface of the host
//! (`gevrek_shellkit::host`) at the rectangle the window manager gives
//! for it. The window manager keeps that room free of the windows it
//! lays out, as it did for the bar of before, and gives
//! it back when no panel is on the screen; the surface itself keeps no
//! room (it is no application bar). So the room is taken off once and by
//! one program, a second gevrek on the machine (the desktop checks) moves
//! nobody else's windows, and the user's own reserved edges count as they
//! do for everything else the window manager lays out. What is given up:
//! maximised windows, and windows gevrek does not manage, lie under the
//! panel, and it does not step down for a fullscreen window.
//!
//! The panel follows the window manager by its events: after one that
//! can change what a panel shows, the state is asked for, and a panel is
//! drawn again when what it shows is different. Nothing is asked on a
//! timer but for one look every half minute, for what has no event (the
//! window manager's notices; a taskbar that was moved).
//!
//! The launcher is the one of every system (`crate::launcher`) on a
//! surface in the middle of the display the pointer is on, with the
//! keyboard. It opens when the window manager says a key asked for it
//! (`shell.command`, as on Linux; `gevrek shell launcher` asks the same
//! way), and from its button on the panel. What differs from Linux:
//!
//! - an application is started here, as the Start menu starts it
//!   (`gevrek_system::apps::start`), not by the window manager running a
//!   command: a shortcut is no program with arguments;
//! - Windows lets no surface keep the keyboard from the windows, so a
//!   click elsewhere takes it away; the launcher then closes, as a menu
//!   does;
//! - the applications are read when the shell starts and again each time
//!   the launcher opens, on a thread of their own (asking the shell about
//!   every shortcut takes most of a second), and the icons likewise, one
//!   by one as their lines are first shown;
//! - what was copied (listed after a `:`) is kept by the shell itself
//!   while it runs (`gevrek_system::clipboard`): the system tells when the
//!   clipboard changes; it is not written down.
//!
//! Notifications are gevrek's own here: what the window manager has to
//! tell (its notices, each once when it arises) and what a command sends
//! (`gevrek notify`). The system's notifications stay in the system's
//! centre; gevrek does not take its place. They are the stack and the
//! cards of every system (`crate::notifications`), on a surface at the top
//! right of the display with the keyboard, under the panel, that never
//! takes the keyboard; the bell on the panel opens what arrived.
//!
//! The on-screen display says what a command asks it to (`gevrek shell
//! osd`), and how loud the sound is when that changes, whoever changed it
//! (the volume keys are the system's on Windows, and the system shows its
//! own display for them too; `desktop.shell.osd.volume: false` leaves it
//! to that one). The power menu's choices are the system's calls
//! (`gevrek_system::session`), and so are `lock` and `displays_off` asked
//! for by a key or a command. The list of shortcuts is shown when the
//! window manager asks for it (its key, `gevrek shortcuts`).

use crate::launcher::{self, Launcher};
use crate::osd::{self, Osd};
use crate::panel::{self, Notice, View};
use crate::power::{self, Choice};
use crate::{notifications, shortcuts};
use clap::Parser;
use gevrek_link::{Event, Link, Request};
use gevrek_model::settings::{PanelEdge, PanelModule, PanelProfile};
use gevrek_shellkit::host::place::Area;
use gevrek_shellkit::host::{Control, Shell, SurfaceId};
use gevrek_shellkit::{Edge, Flow, Keyboard, Level, Place};
use gevrek_system::session::{End, Outcome};
use gevrek_system::sound::{Device, Level as SoundLevel};
use gevrek_ui::theme::{self, icons};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "gevrek-shell", about = "gevrek's shell", version)]
struct Args {
    /// The window manager to listen to (default: this session's)
    #[arg(long)]
    address: Option<String>,
    /// Open the launcher at once (for trying it, and for checks)
    #[arg(long)]
    launcher: bool,
    /// Do not draw the panel (where another bar is used)
    #[arg(long)]
    no_panel: bool,
}

/// How often the state is asked for without an event to ask after.
const LOOK_EVERY: Duration = Duration::from_secs(30);

/// What reaches the shell from outside its surfaces.
enum Message {
    /// The line to the window manager has news.
    Line,
    /// Time for the look that no event asks for.
    Look,
    /// Open the launcher, or put it away (`--launcher`).
    Launcher,
    /// The applications were read.
    Apps(Vec<gevrek_system::apps::App>),
    /// An icon was looked for: what shows icons is drawn again.
    Icon,
    /// What is heard is louder, quieter, muted or heard again.
    Sound(SoundLevel),
    /// The network the machine is on, read when the system said that
    /// something about it changed (and once at the start).
    Network(Option<gevrek_system::status::Network>),
    /// The pictures of the windows, for the window switcher.
    Thumbs(Vec<crate::switcher::Thumb>),
    /// The icons programs give the taskbar are others.
    Tray(Vec<gevrek_system::tray_host::Icon>),
    /// A picture of a window was taken, to be remembered for the
    /// overview of the workspaces.
    Pictured(u64, gevrek_system::pictures::Picture),
    /// How the wireless side stands (asked beside the shell: it takes
    /// its moment), and whether networks are being looked for.
    Wifi(Option<gevrek_system::wifi::Wifi>),
    WifiLooking(bool),
    /// How bluetooth stands.
    Bluetooth(Option<gevrek_system::bluetooth::Bluetooth>),
}

/// The pixels an application's icon is asked for at: twice what it is
/// drawn at, for a display at twice the scale.
const ICON: u32 = 64;

/// The panel of one display.
struct One {
    surface: SurfaceId,
    /// Where the window manager keeps its room.
    at: panel::Rect,
    view: View,
}

/// What a panel can have open next to it.
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Layouts,
    Notices,
    /// The devices to play on.
    Sound,
    /// The icons of the tray: all of them, and which stand on the panel.
    Tray,
}

/// What the surfaces share.
struct State {
    link: Link,
    /// The panel of each display, by the window manager's name for it.
    panels: HashMap<String, One>,
    /// The menu of a panel that is open, which it is, and what tells that
    /// it still is (its drawing holds the other end).
    menu: Option<(SurfaceId, Menu, std::rc::Weak<()>)>,
    bar: Option<PanelProfile>,
    /// Where the window manager keeps room for a panel, and what each
    /// display's panel shows, as of its last state.
    places: Vec<(String, panel::Rect)>,
    views: HashMap<String, View>,
    /// The windows that state had.
    windows: std::collections::HashSet<u64>,
    notices: Rc<Vec<Notice>>,
    mode: String,
    /// The shell draws the panel (not with `--no-panel`).
    with_panel: bool,
    /// The name of this instance's panels (the end of their window class).
    name: String,
    /// The modules that were last said to be left out.
    said_left_out: Vec<PanelModule>,
    /// Whether the person is to be welcomed was asked (once a run).
    welcome_asked: bool,
    /// Whether a tutorial that was stopped left something was asked.
    tidy_asked: bool,
    /// The launcher, while it is open.
    launcher: Option<(SurfaceId, Launcher)>,
    /// The applications as they were last read, and whether they are
    /// being read now.
    apps: Vec<gevrek_system::apps::App>,
    reading_apps: bool,
    /// The windows that are open, to switch to from the launcher.
    open: Vec<launcher::Open>,
    icons: gevrek_system::icons::Cache,
    /// To hear from the threads that read.
    sender: gevrek_shellkit::host::Sender<Message>,
    settings: gevrek_ui::Settings,
    /// The colours the preferences' file asks for (high contrast takes
    /// their place while it is on), and what the configuration asks for:
    /// less movement, more contrast.
    own_preset: gevrek_ui::theme::Preset,
    asked_look: (bool, bool),
    accent: egui::Color32,
    /// The displays' rooms as of the window manager's last state.
    rooms: Vec<panel::Room>,
    /// What gevrek has told: what is shown now, and what arrived.
    notes: notifications::Stack,
    /// The corner they are shown in (`desktop.shell.notifications`).
    notes_corner: gevrek_model::settings::NotificationCorner,
    notes_surface: Option<SurfaceId>,
    /// The window manager's notices that were told as notifications (by
    /// what it knows them by): each is told once for as long as it holds.
    told: HashSet<String>,
    /// What the log last said the notifications show.
    said_notes: String,
    /// The notification centre, while it is open.
    centre: Option<SurfaceId>,
    /// The power menu: its surface, the choice that waits for its second
    /// press, and what went wrong with the last one.
    power: Option<(SurfaceId, Option<Choice>, String)>,
    /// What the on-screen display shows, and until when.
    osd: Option<(SurfaceId, Osd, Instant)>,
    /// How loud what is heard was when the system last told.
    heard: Option<SoundLevel>,
    /// What the panel shows of the machine: the sound (as the system
    /// tells it), the network (read when the system says it changed) and
    /// the battery (looked at with the look that no event asks for).
    system: panel::SystemView,
    /// A change of the volume is shown (`desktop.shell.osd.volume`).
    show_volume: bool,
    /// What was copied lately, kept for the launcher (after a `:`).
    clipboard: Option<gevrek_system::clipboard::Keeper>,
    /// The list of shortcuts, while it is open.
    shortcuts: Option<SurfaceId>,
    /// The keys of the configuration, once the window manager told it.
    keys: Option<Vec<gevrek_model::shortcuts::Shortcut>>,
    /// The icons programs give the taskbar, while the panel has a place
    /// for them (its `tray` module): who hears of them, and what they are.
    tray: Option<gevrek_system::tray_host::Host>,
    tray_items: Vec<crate::tray::Item>,
    /// Every icon programs gave, each with the name it is known by from
    /// one day to the next; and the names of those that stand on the panel
    /// (the others are in the list behind the tray's own button). An icon
    /// that is new is not on the panel.
    tray_all: Vec<(crate::tray::Item, String)>,
    tray_on_panel: HashSet<String>,
    /// The control centre, while it is open: its surface, what is held
    /// in it, and what the system's sound has (the devices to play on
    /// and to record from, the programs that play) as it was last read.
    control_centre: Option<(
        SurfaceId,
        crate::control::Held,
        gevrek_system::sound::Mixer,
    )>,
    /// The night light is switched on (as the configuration last said).
    night_light: bool,
    /// The tutorial, while it is taken: its card's surface, how far it
    /// is, and the practice windows it opened.
    tutorial: Option<(
        SurfaceId,
        crate::tutoring::Tutoring,
        Vec<std::process::Child>,
    )>,
    /// The running configuration (the tutorial shows the keys it has),
    /// and the desktop as the window manager last said it stands.
    config: Option<Box<gevrek_model::Config>>,
    /// The revision of that configuration: a change the shell makes to it
    /// (a workspace's name, whose windows the taskbar has) is based on it.
    revision: u64,
    /// The box that asks for a workspace's new name, while it is open:
    /// the name the workspace has, what is typed, and what is wrong with
    /// that.
    rename: Option<(SurfaceId, String, String, String)>,
    facts: gevrek_tutorial::Facts,
    /// The wireless side and bluetooth, as they were last read for the
    /// control centre; the network that is being joined; and whether
    /// networks are being looked for.
    wifi: Option<gevrek_system::wifi::Wifi>,
    wifi_joining: Option<String>,
    wifi_looking: bool,
    bluetooth: Option<gevrek_system::bluetooth::Bluetooth>,
    /// The window switcher, while it is up.
    switcher: Option<(SurfaceId, crate::switcher::Switcher)>,
    /// The windows as the switcher offers them, as of the last state, and
    /// the order they last had the keyboard in.
    switch_to: Vec<crate::switcher::Entry>,
    used: crate::switcher::Used,
    /// The overview of every workspace, while it is up; the displays
    /// with their workspaces and windows as of the last state; the
    /// pictures of windows that are remembered for it; and the window
    /// that had the keyboard at the last state.
    overview: Option<(SurfaceId, crate::overview::Overview)>,
    spaces: Vec<crate::overview::Display>,
    pictures: crate::overview::Remembered,
    had_keyboard: Option<u64>,
}

pub fn run() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    // `gevrek stop` of another instance must not take this one for its own.
    gevrek_model::paths::claim_process("shell");
    // Said at the start, so that whoever is about to ask for a lock in a
    // check can see first that this shell will not carry it out.
    if gevrek_system::session::is_dry_run() {
        log::warn!(
            "{} is set: nothing is asked of the session, and the displays stay on",
            gevrek_system::session::DRY_RUN
        );
    }
    let address = args
        .address
        .unwrap_or_else(|| gevrek_protocol::DEFAULT_WM_ENDPOINT.to_string());
    let shell: Shell<Message> = Shell::new()?;
    let sender = shell.sender();
    let link = Link::start(address, move || {
        sender.send(Message::Line);
    });
    let appearance = gevrek_model::paths::config_file()
        .parent()
        .map(|folder| folder.join("studio-appearance"))
        .unwrap_or_default();
    let state = Rc::new(RefCell::new(State {
        link,
        panels: HashMap::new(),
        menu: None,
        bar: None,
        places: Vec::new(),
        views: HashMap::new(),
        windows: Default::default(),
        notices: Rc::new(Vec::new()),
        mode: String::new(),
        with_panel: !args.no_panel,
        name: gevrek_model::paths::panel_surface(&gevrek_model::paths::instance()),
        said_left_out: Vec::new(),
        welcome_asked: false,
        tidy_asked: false,
        launcher: None,
        apps: Vec::new(),
        reading_apps: false,
        open: Vec::new(),
        icons: {
            let found = shell.sender();
            gevrek_system::icons::Cache::new(move || {
                found.send(Message::Icon);
            })
        },
        sender: shell.sender(),
        settings: gevrek_ui::Settings::load(&appearance),
        own_preset: gevrek_ui::Settings::load(&appearance).preset,
        asked_look: (false, false),
        accent: egui::Color32::from_rgb(64, 224, 208),
        rooms: Vec::new(),
        notes: {
            // What arrived before the shell was started anew is still
            // to be looked at under the bell.
            let mut notes = notifications::Stack::default();
            if let Ok(written) = std::fs::read_to_string(notes_file()) {
                notes.remember(&written, Instant::now(), std::time::SystemTime::now());
            }
            notes
        },
        notes_corner: Default::default(),
        notes_surface: None,
        told: HashSet::new(),
        said_notes: String::new(),
        centre: None,
        power: None,
        osd: None,
        heard: None,
        system: panel::SystemView {
            volume: gevrek_system::sound::level(Device::Speaker)
                .map(|level| (level.percent, level.muted)),
            battery: gevrek_system::status::battery(),
            // (Read on a thread of its own, below.)
            network: None,
            // (A laptop's panel; a desktop machine has none to set.)
            brightness: gevrek_system::brightness::read().map(|now| now.percent()),
        },
        show_volume: true,
        clipboard: {
            let keeper = gevrek_system::clipboard::start();
            if keeper.is_none() {
                log::warn!("what is copied is not kept for the launcher");
            }
            keeper
        },
        shortcuts: None,
        keys: None,
        control_centre: None,
        night_light: false,
        tutorial: None,
        config: None,
        revision: 0,
        rename: None,
        facts: gevrek_tutorial::Facts::default(),
        wifi: None,
        wifi_joining: None,
        wifi_looking: false,
        bluetooth: None,
        tray: None,
        tray_items: Vec::new(),
        tray_all: Vec::new(),
        tray_on_panel: read_tray_choice(),
        switcher: None,
        switch_to: Vec::new(),
        used: Default::default(),
        overview: None,
        spaces: Vec::new(),
        pictures: Default::default(),
        had_keyboard: None,
    }));
    // The network: read once, and again each time the system says that
    // something about the machine's addresses changed. Nothing is asked
    // by the clock unless the system cannot be waited on.
    let network = shell.sender();
    std::thread::Builder::new()
        .name("gevrek-shell-network".to_string())
        .spawn(move || loop {
            if !network.send(Message::Network(gevrek_system::status::network())) {
                break;
            }
            if gevrek_system::status::wait_for_network_change() {
                // A change comes as several: read when they are over.
                std::thread::sleep(Duration::from_millis(700));
            } else {
                std::thread::sleep(LOOK_EVERY);
            }
        })?;
    // The system tells when what is heard changes; nothing is asked.
    let changed = shell.sender();
    if !gevrek_system::sound::watch(move |level| {
        changed.send(Message::Sound(level));
    }) {
        log::warn!("changes of the volume are not shown");
    }
    // The look that no event asks for. The thread ends with the shell.
    let again = shell.sender();
    std::thread::Builder::new()
        .name("gevrek-shell-look".to_string())
        .spawn(move || loop {
            std::thread::sleep(LOOK_EVERY);
            if !again.send(Message::Look) {
                break;
            }
        })?;
    // The applications are there when the launcher is first asked for.
    read_apps(&state);
    // `--launcher` goes through the same door as the key does.
    if args.launcher {
        shell.sender().send(Message::Launcher);
    }
    shell.run(false, move |message, control| match message {
        Message::Line => take_in(&state, control),
        Message::Look => {
            let mut state = state.borrow_mut();
            state.link.send(Request::State);
            // The battery has no event here: looked at with the rest.
            let battery = gevrek_system::status::battery();
            // (Nor has the brightness, which the machine's own keys change.)
            let brightness = gevrek_system::brightness::read().map(|now| now.percent());
            if battery != state.system.battery || brightness != state.system.brightness {
                state.system.battery = battery;
                state.system.brightness = brightness;
                redraw_panels(&state, control);
            }
            // Nor have Windows' own settings for movement and contrast.
            if follow_look(&mut state) {
                control.redraw_all();
            }
        }
        Message::Network(network) => {
            let mut state = state.borrow_mut();
            if network != state.system.network {
                state.system.network = network;
                redraw_panels(&state, control);
            }
        }
        Message::Wifi(wifi) => {
            let mut state = state.borrow_mut();
            state.wifi = wifi;
            state.wifi_joining = None;
            if let Some((id, ..)) = &state.control_centre {
                control.redraw(*id);
            }
        }
        Message::WifiLooking(looking) => {
            let mut state = state.borrow_mut();
            state.wifi_looking = looking;
            if let Some((id, ..)) = &state.control_centre {
                control.redraw(*id);
            }
        }
        Message::Bluetooth(bluetooth) => {
            let mut state = state.borrow_mut();
            state.bluetooth = bluetooth;
            if let Some((id, ..)) = &state.control_centre {
                control.redraw(*id);
            }
        }
        Message::Launcher => toggle_launcher(&state, control),
        Message::Apps(apps) => {
            // Said aloud: reading them is the one piece of work the shell
            // does unasked after its start, and whoever measures what a
            // still shell costs has to know when it is over.
            log::info!("{} applications read", apps.len());
            let mut state = state.borrow_mut();
            state.reading_apps = false;
            state.apps = apps.clone();
            if let Some((surface, launcher)) = state.launcher.as_mut() {
                launcher.set_apps(apps);
                control.redraw(*surface);
            }
        }
        Message::Icon => {
            if let Some((surface, _)) = &state.borrow().launcher {
                control.redraw(*surface);
            }
        }
        Message::Sound(level) => heard(&state, level, control),
        Message::Tray(icons) => {
            let mut state = state.borrow_mut();
            // (Only while they are wanted: the host may have just ended.)
            if state.tray.is_some() {
                log::debug!(
                    "the tray shows {}",
                    serde_json::json!({
                        "icons": icons.len(),
                        "pictures": icons.iter().filter(|icon| icon.picture.is_some()).count(),
                        "tips": icons.iter().map(|icon| icon.tip.clone()).collect::<Vec<_>>(),
                    })
                );
                state.tray_all = icons
                    .into_iter()
                    .map(|icon| {
                        (
                            crate::tray::Item {
                                key: icon.key,
                                name: icon.name.clone(),
                                title: icon.tip.clone(),
                                hint: icon.tip,
                                picture: icon.picture,
                                stamp: icon.stamp,
                                // The menu is the program's own, on the other button.
                                menu_only: false,
                                passive: false,
                                attention: false,
                            },
                            icon.name,
                        )
                    })
                    .collect();
                tray_for_panel(&mut state);
                redraw_panels(&state, control);
                // (The list of them, when it is open, shows them too.)
                if let Some((menu, Menu::Tray, _)) = &state.menu {
                    control.redraw(*menu);
                }
            }
        }
        Message::Thumbs(thumbs) => {
            if let Some((id, switcher)) = state.borrow_mut().switcher.as_mut() {
                switcher.thumbs = thumbs.into_iter().map(|thumb| (thumb, None)).collect();
                control.redraw(*id);
            }
        }
        Message::Pictured(window, picture) => {
            let mut state = state.borrow_mut();
            state.pictures.keep(window, picture);
            if let Some((id, _)) = &state.overview {
                control.redraw(*id);
            }
        }
    })?;
    Ok(())
}

/// Read the applications again, away from the thread that draws; they
/// arrive as a message. Asked while they are being read, nothing more
/// happens.
fn read_apps(state: &Rc<RefCell<State>>) {
    let mut state = state.borrow_mut();
    if state.reading_apps {
        return;
    }
    let sender = state.sender.clone();
    let started = std::thread::Builder::new()
        .name("gevrek-shell-apps".to_string())
        .spawn(move || {
            sender.send(Message::Apps(gevrek_system::apps::installed()));
        });
    match started {
        Ok(_) => state.reading_apps = true,
        Err(error) => log::warn!("the applications are not read: {error}"),
    }
}

use crate::tray_choice::{
    keep_tray_choice, read_tray_choice, tray_list_draw, tray_list_size, TrayAsk, TRAY_MORE,
};

/// What the panel shows of the tray (see `tray_choice::for_panel`).
fn tray_for_panel(state: &mut State) {
    let colour = state.bar.as_ref().map_or((214, 220, 232), |bar| bar.text);
    state.tray_items = crate::tray_choice::for_panel(&state.tray_all, &state.tray_on_panel, colour);
}

/// The name of one of this instance's surfaces (the end of its window's
/// class): a second gevrek on the machine has surfaces of its own.
fn surface_name(base: &str) -> String {
    let instance = gevrek_model::paths::instance();
    if instance == gevrek_model::paths::MAIN_INSTANCE {
        base.to_string()
    } else {
        format!("{base}.{instance}")
    }
}

/// The colours and fonts of a surface's frame.
fn themed(state: &State, ui: &egui::Ui, fonts: &mut bool) -> theme::Palette {
    let context = ui.ctx();
    if !*fonts {
        theme::install_fonts(context, state.settings.system_font);
        *fonts = true;
    }
    let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
    theme::apply(context, &palette, &state.settings);
    palette
}

/// Whether a surface that is there for the keyboard has lost it (a click
/// on another window): it is then put away, as a menu is. Not before it
/// ever had the keyboard: it is drawn once before it hears that it has.
fn lost_keyboard(context: &egui::Context, had: &mut bool) -> bool {
    let has = context.input(|input| input.focused);
    *had |= has;
    *had && !has
}

/// Say something of gevrek's own as a notification. `stay_ms`: how long
/// it stays (0: until it is pressed; less: the usual time).
fn tell(state: &Rc<RefCell<State>>, title: &str, text: &str, stay_ms: i32, control: &mut Control) {
    {
        let mut state = state.borrow_mut();
        state
            .notes
            .tell("gevrek", title, text, stay_ms, Instant::now());
        keep_notes(&state.notes);
    }
    show_notes(state, control);
}

/// Where what arrived is written down, to be there after the shell was
/// started anew: with this instance's state.
fn notes_file() -> std::path::PathBuf {
    gevrek_model::paths::state_dir().join("notifications.json")
}

/// Write down what arrived (each time it is another).
fn keep_notes(notes: &notifications::Stack) {
    let file = notes_file();
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let written = notes.remembered(Instant::now(), std::time::SystemTime::now());
    if let Err(error) = std::fs::write(&file, written) {
        log::debug!("what arrived was not written down: {error}");
    }
}

/// How far the notifications are from the edges of the room they are in,
/// in logical pixels.
const NOTES_GAP: i32 = 12;

/// Where the notifications' surface belongs for what is on the stack: at
/// the top right of the display with the keyboard, under its panel.
/// `None` while the window manager has not told of its displays.
fn notes_area(state: &State) -> Option<Area> {
    let room = state
        .rooms
        .iter()
        .find(|room| room.focused)
        .or(state.rooms.first())?;
    let pixels = |logical: i32| gevrek_shellkit::host::place::pixels(logical, room.scale);
    let (x, y, width, height) = panel::corner_at(
        room,
        (
            pixels(notifications::NOTE_WIDTH as i32),
            pixels(notifications::height(&state.notes) as i32),
        ),
        pixels(NOTES_GAP),
        state.notes_corner,
    );
    Some(Area::new(x, y, width, height))
}

/// Say what the notifications show, when asked to
/// (`RUST_LOG=gevrek_shell=debug`) and it is another than before: the
/// desktop check reads it.
fn say_notes(state: &mut State) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let shown: Vec<&str> = state
        .notes
        .shown()
        .iter()
        .map(|note| note.summary.as_str())
        .collect();
    let now = serde_json::json!({"shown": shown, "waiting": state.notes.waiting()}).to_string();
    if state.said_notes != now {
        log::debug!("the notifications show {now}");
        state.said_notes = now;
    }
}

/// Bring the notifications' surface in line with the stack: open it,
/// give it the size for what is shown, draw it again.
fn show_notes(state: &Rc<RefCell<State>>, control: &mut Control) {
    let (surface, area, height, empty) = {
        let mut state = state.borrow_mut();
        say_notes(&mut state);
        (
            state.notes_surface,
            notes_area(&state),
            notifications::height(&state.notes),
            state.notes.is_empty(),
        )
    };
    match surface {
        Some(id) if control.is_open(id) => {
            if !empty {
                match area {
                    Some(area) => control.put_at(id, area),
                    None => control.resize(id, notifications::NOTE_WIDTH, height),
                }
            }
            control.redraw(id);
        }
        _ if empty => {}
        _ => {
            let place = Place {
                name: surface_name("gevrek-notifications"),
                edge: Edge::TopRight,
                level: Level::Overlay,
                width: notifications::NOTE_WIDTH,
                height,
                reserve: false,
                // A notification never takes the keyboard from a window.
                keyboard: Keyboard::Never,
                margin: NOTES_GAP,
                output: None,
            };
            let shared = Rc::clone(state);
            let mut fonts = false;
            let draw = move |ui: &mut egui::Ui, control: &mut Control| {
                draw_notes(&shared, ui, control, &mut fonts)
            };
            let id = match area {
                Some(area) => control.open_at(place, area, draw),
                // Not told of the displays yet: where the host puts it.
                None => control.open(place, draw),
            };
            state.borrow_mut().notes_surface = Some(id);
        }
    }
}

fn draw_notes(
    shared: &Rc<RefCell<State>>,
    ui: &mut egui::Ui,
    control: &mut Control,
    fonts: &mut bool,
) -> Flow {
    let context = ui.ctx().clone();
    let mut state = shared.borrow_mut();
    let state = &mut *state;
    let palette = themed(state, ui, fonts);
    let now = Instant::now();
    let before = state.notes.shown().len();
    // Those whose time is over go by themselves.
    state.notes.expire(now);
    let gone = notifications::draw_cards(ui, &palette, &state.notes);
    let dismissed = gone.is_some();
    if let Some((id, _)) = gone {
        state.notes.remove(id);
    }
    say_notes(state);
    if state.notes.is_empty() {
        state.notes_surface = None;
        return Flow::Close;
    }
    if state.notes.shown().len() != before || dismissed {
        if let Some(id) = state.notes_surface {
            match notes_area(state) {
                Some(area) => control.put_at(id, area),
                None => control.resize(
                    id,
                    notifications::NOTE_WIDTH,
                    notifications::height(&state.notes),
                ),
            }
        }
        context.request_repaint();
    }
    // Drawn again when the next one's time is over.
    if let Some(next) = state.notes.next_expiry() {
        context
            .request_repaint_after(next.saturating_duration_since(now) + Duration::from_millis(30));
    }
    Flow::Continue
}

/// The width of the notification centre, in logical pixels.
const CENTRE_WIDTH: u32 = 400;

/// Open the notification centre (what gevrek told, to look at again), or
/// close it when it is open. It has the keyboard while it is open, for
/// Escape; a click elsewhere puts it away.
fn toggle_centre(state: &Rc<RefCell<State>>, control: &mut Control) {
    if let Some(id) = state.borrow_mut().centre.take() {
        if control.is_open(id) {
            control.close(id);
            return;
        }
    }
    // The cards are in the corner the centre opens in, and all of them
    // are in its list: they are put away.
    state.borrow_mut().notes.put_away();
    show_notes(state, control);
    let place = Place {
        name: surface_name("gevrek-notification-centre"),
        edge: Edge::Right,
        level: Level::Overlay,
        width: CENTRE_WIDTH,
        height: 0,
        reserve: false,
        // (It does not take the keyboard from the window that has it:
        // it has the keyboard when something in it is pressed.)
        keyboard: Keyboard::OnDemand,
        margin: NOTES_GAP,
        output: None,
    };
    // Down the right side of the display with the keyboard, under its
    // panel; where the displays are not known, where the host puts it.
    let area = {
        let state = state.borrow();
        state
            .rooms
            .iter()
            .find(|room| room.focused)
            .or(state.rooms.first())
            .map(|room| {
                let pixels =
                    |logical: i32| gevrek_shellkit::host::place::pixels(logical, room.scale);
                let (x, y, width, height) = panel::corner(
                    room,
                    (pixels(CENTRE_WIDTH as i32), i32::MAX / 2),
                    pixels(NOTES_GAP),
                );
                Area::new(x, y, width, height)
            })
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut said = String::new();
    let draw = move |ui: &mut egui::Ui, control: &mut Control| {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let escape =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let asked = notifications::draw_centre(
            ui,
            &palette,
            &state.notes,
            Instant::now(),
            state.notes.quiet,
            &crate::frost::Look::plain(),
        );
        if asked.quiet != state.notes.quiet {
            state.notes.quiet = asked.quiet;
            // The bell shows it.
            for one in state.panels.values() {
                control.redraw(one.surface);
            }
        }
        if asked.forget {
            state.notes.forget_history();
            keep_notes(&state.notes);
            context.request_repaint();
        }
        // What it lists, and where its button to forget all of it is (in
        // the surface's own points): the desktop check reads it.
        if log::log_enabled!(log::Level::Debug) {
            let titles: Vec<&str> = state
                .notes
                .history()
                .iter()
                .map(|note| note.summary.as_str())
                .collect();
            let clear = asked.clear.map(|rect| [rect.center().x, rect.center().y]);
            let now = serde_json::json!({"titles": titles, "clear": clear}).to_string();
            if said != now {
                log::debug!("the history shows {now}");
                said = now;
            }
        }
        // (A press on a window or the desktop puts it away, whether or
        // not it ever had the keyboard. Losing the keyboard does not, as
        // with the control centre: `toggle_control` says why.)
        let elsewhere = state.centre.is_some_and(|own| control.pressed_outside(own));
        if escape || asked.close || elsewhere {
            state.centre = None;
            return Flow::Close;
        }
        Flow::Continue
    };
    let id = match area {
        Some(area) => control.open_at(place, area, draw),
        None => control.open(place, draw),
    };
    state.borrow_mut().centre = Some(id);
}

/// Ask the system for one of the session's ends, or to switch the
/// displays off, and say what came of it. With `GEVREK_POWER_DRY_RUN` set
/// in the shell's environment nothing is carried out (that is decided
/// where the system's calls are, `gevrek_system::session`): what would
/// have been is logged. For the desktop checks, which must never end or
/// darken the session they run in.
fn ask_system(end: End, words: &str) -> Result<(), String> {
    match gevrek_system::session::carry_out(end)? {
        Outcome::DryRun => log::warn!(
            "not carried out ({} is set): {words}",
            gevrek_system::session::DRY_RUN
        ),
        Outcome::CarriedOut => log::info!("the session is asked to: {words}"),
    }
    Ok(())
}

/// Carry out one of the power menu's choices by the system's call for it.
fn carry_out(choice: Choice) -> Result<(), String> {
    ask_system(choice.end(), choice.words())
}

/// Ask for a workspace's new name, in a small box on the display of the
/// panel whose tab was pressed. Enter gives the workspace of that display
/// the name as its title, and leaves the workspaces of that name on the
/// other displays as they are (`panel::renaming`): it is written into the
/// configuration, which the window manager takes in at once and has again
/// when it is started anew. A name that cannot be is said in the box,
/// which stays.
///
/// `place` is the display as the window manager and the configuration
/// name it, `name` what the workspace is known by there, `label` what it
/// is shown by now.
fn ask_workspace_name(
    state: &Rc<RefCell<State>>,
    control: &mut Control,
    place: gevrek_model::settings::DisplayRef,
    name: &str,
    label: &str,
) {
    if let Some((id, ..)) = state.borrow_mut().rename.take() {
        if control.is_open(id) {
            control.close(id);
        }
    }
    let surface = Place {
        name: surface_name("gevrek-rename"),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: panel::RENAME_SIZE.0,
        height: panel::RENAME_SIZE.1,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        // (The display the pointer is on, which is the one of the panel
        // whose tab was pressed. Not by the display's name: that is the
        // window manager's name for it, and a surface is put on a display
        // by Windows' name; asked for by the other, no box was made at
        // all.)
        output: None,
    };
    let key = name.to_string();
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut had_keyboard = false;
    let id = control.open(surface, move |ui, _| {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let Some((_, old, mut name, mut problem)) = state.rename.clone() else {
            return Flow::Close;
        };
        let mut close =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let before = name.clone();
        match panel::rename_box(ui, &palette, &old, &mut name, &problem) {
            Some(true) => match panel::renaming(state.config.as_deref(), &place, &key, &name) {
                panel::Renaming::Nothing => close = true,
                panel::Renaming::Refused(why) => problem = why,
                panel::Renaming::Apply(changed) => {
                    log::info!(
                        "workspace {key:?} of {} is shown as {:?} from now on",
                        place.name(),
                        name.trim()
                    );
                    apply_configuration(state, changed);
                    state.link.send(Request::StateAfter(Duration::from_millis(120)));
                    close = true;
                }
            },
            Some(false) => close = true,
            None => {
                if name != before {
                    problem.clear();
                }
            }
        }
        if close || lost_keyboard(&context, &mut had_keyboard) {
            state.rename = None;
            return Flow::Close;
        }
        if let Some(rename) = state.rename.as_mut() {
            rename.2 = name;
            rename.3 = problem;
        }
        Flow::Continue
    });
    state.borrow_mut().rename = Some((id, label.to_string(), label.to_string(), String::new()));
}

/// Give the window manager a configuration the shell changed (a
/// workspace's title, what the taskbar shows). It is the shell's own from
/// this moment, so that a second change made before the window manager
/// has answered builds on the first; the revision it answers with is
/// taken in `Event::Applied`, and a change it refuses is told and the
/// running configuration fetched again.
fn apply_configuration(state: &mut State, changed: Box<gevrek_model::Config>) {
    state.link.send(Request::Apply {
        config: changed.clone(),
        base_revision: state.revision,
        client_id: "gevrek-shell".to_string(),
    });
    state.config = Some(changed);
}

/// Open the power menu, or close it when it is open. It has the keyboard
/// while it is open; a click elsewhere puts it away.
fn toggle_power(state: &Rc<RefCell<State>>, control: &mut Control) {
    if let Some((id, ..)) = state.borrow_mut().power.take() {
        if control.is_open(id) {
            control.close(id);
            return;
        }
    }
    let place = Place {
        name: surface_name("gevrek-power"),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: power::SIZE.0,
        height: power::SIZE.1,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut had_keyboard = false;
    let mut said = String::new();
    let id = control.open(place, move |ui, _| {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let Some((_, armed, problem)) = state.power.clone() else {
            return Flow::Close;
        };
        let mut close =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let chosen = power::draw(ui, &palette, armed, &problem);
        // What it offers, and which choice waits for its second press:
        // the desktop check reads it.
        if log::log_enabled!(log::Level::Debug) {
            let entries: Vec<&str> = Choice::ALL.iter().map(|choice| choice.words()).collect();
            let now = serde_json::json!({
                "entries": entries,
                "armed": armed.map(Choice::words),
            })
            .to_string();
            if said != now {
                log::debug!("the power menu shows {now}");
                said = now;
            }
        }
        if let Some(choice) = chosen {
            match power::step(choice, armed) {
                power::Step::CarryOut(choice) => match carry_out(choice) {
                    Ok(()) => close = true,
                    Err(why) => {
                        if let Some(power) = state.power.as_mut() {
                            power.1 = None;
                            power.2 = why;
                        }
                    }
                },
                power::Step::Arm(choice) => {
                    if let Some(power) = state.power.as_mut() {
                        power.1 = Some(choice);
                        power.2.clear();
                    }
                }
            }
            context.request_repaint();
        }
        if close || lost_keyboard(&context, &mut had_keyboard) {
            state.power = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    state.borrow_mut().power = Some((id, None, String::new()));
}

/// Say something near the bottom of the display for a moment.
fn show_osd(state: &Rc<RefCell<State>>, osd: Osd, control: &mut Control) {
    let until = Instant::now() + osd::STAY;
    // Said when asked to: the desktop check reads it.
    log::debug!(
        "the on-screen display shows {}",
        serde_json::json!({"text": osd.text, "level": osd.level})
    );
    let open = state
        .borrow()
        .osd
        .as_ref()
        .map(|(id, ..)| *id)
        .filter(|id| control.is_open(*id));
    match open {
        Some(id) => {
            state.borrow_mut().osd = Some((id, osd, until));
            control.redraw(id);
        }
        None => {
            let place = Place {
                name: surface_name("gevrek-osd"),
                edge: Edge::Bottom,
                level: Level::Overlay,
                width: osd::SIZE.0,
                height: osd::SIZE.1,
                reserve: false,
                keyboard: Keyboard::Never,
                margin: osd::MARGIN,
                output: None,
            };
            let shared = Rc::clone(state);
            let mut fonts = false;
            let id = control.open(place, move |ui, _| {
                let context = ui.ctx().clone();
                let mut state = shared.borrow_mut();
                let state = &mut *state;
                let palette = themed(state, ui, &mut fonts);
                let Some((_, osd, until)) = state.osd.clone() else {
                    return Flow::Close;
                };
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    state.osd = None;
                    return Flow::Close;
                }
                context.request_repaint_after(left + Duration::from_millis(20));
                osd::draw(ui, &palette, &osd);
                Flow::Continue
            });
            state.borrow_mut().osd = Some((id, osd, until));
        }
    }
}

/// The shell's own surfaces keep still, and take their high-contrast
/// colours, when the configuration asks for it or Windows' own settings
/// do ("Animation effects" off, a contrast theme). Returns whether the
/// look is another than before.
fn follow_look(state: &mut State) -> bool {
    let (reduce, contrast) = state.asked_look;
    let reduce_motion = reduce || gevrek_system::look::reduce_motion();
    let preset = if contrast || gevrek_system::look::high_contrast() {
        gevrek_ui::theme::Preset::HighContrastDark
    } else {
        state.own_preset
    };
    let changed = state.settings.reduce_motion != reduce_motion || state.settings.preset != preset;
    state.settings.reduce_motion = reduce_motion;
    state.settings.preset = preset;
    changed
}

/// Every panel is drawn again (what they all show is another).
fn redraw_panels(state: &State, control: &mut Control) {
    for one in state.panels.values() {
        control.redraw(one.surface);
    }
}

/// The panel's sound was pressed or the wheel turned over it: muted (or
/// heard again), louder or quieter. The panel says what came of it; the
/// on-screen display does not say it a second time.
fn panel_sound(state: &Rc<RefCell<State>>, ask: panel::SoundAsk, control: &mut Control) {
    use gevrek_system::sound;
    let level = match ask {
        panel::SoundAsk::Step(by) => sound::change(Device::Speaker, by),
        panel::SoundAsk::Mute => sound::toggle_mute(Device::Speaker),
    };
    let Some(level) = level else {
        return;
    };
    let mut state = state.borrow_mut();
    // The system tells of the change too: it is not shown for that.
    state.heard = Some(level);
    state.system.volume = Some((level.percent, level.muted));
    redraw_panels(&state, control);
}

/// What is heard is another volume than before, whoever changed it (the
/// keyboard's keys are the system's): shown, unless the configuration
/// leaves that to the system's own display.
fn heard(state: &Rc<RefCell<State>>, level: SoundLevel, control: &mut Control) {
    let show = {
        let mut state = state.borrow_mut();
        // The system may tell the same twice.
        let changed = state.heard != Some(level);
        state.heard = Some(level);
        // The panel's sound says it too.
        let volume = Some((level.percent, level.muted));
        if state.system.volume != volume {
            state.system.volume = volume;
            redraw_panels(&state, control);
        }
        changed && state.show_volume
    };
    if show {
        show_osd(state, Osd::sound(Device::Speaker, Some(level)), control);
    }
}

/// A shell action that changes the sound: done, and what came of it said.
fn sound_action(state: &Rc<RefCell<State>>, action: &str, control: &mut Control) {
    use gevrek_system::sound;
    let (device, level) = match action {
        "volume_up" => (Device::Speaker, sound::change(Device::Speaker, 5)),
        "volume_down" => (Device::Speaker, sound::change(Device::Speaker, -5)),
        "volume_mute" => (Device::Speaker, sound::toggle_mute(Device::Speaker)),
        _ => (Device::Microphone, sound::toggle_mute(Device::Microphone)),
    };
    if device == Device::Speaker && level.is_some() {
        // The system tells of the change too: it is not said twice.
        state.borrow_mut().heard = level;
    }
    show_osd(state, Osd::sound(device, level), control);
}

/// Open the list of shortcuts, or close it when it is open. It has the
/// keyboard while it is open, for the search and for Escape; a click
/// elsewhere puts it away.
fn toggle_shortcuts(state: &Rc<RefCell<State>>, control: &mut Control) {
    if let Some(id) = state.borrow_mut().shortcuts.take() {
        if control.is_open(id) {
            control.close(id);
            return;
        }
    }
    let place = Place {
        name: surface_name("gevrek-shortcuts"),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: shortcuts::SIZE.0,
        height: shortcuts::SIZE.1,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 24,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut had_keyboard = false;
    let mut typed = String::new();
    let mut said = String::new();
    let id = control.open(place, move |ui, _| {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let escape =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        shortcuts::draw(ui, &palette, state.keys.as_deref(), &mut typed);
        // What is typed and how much of the list answers it: the desktop
        // check reads it.
        if log::log_enabled!(log::Level::Debug) {
            let counts = shortcuts::counts(state.keys.as_deref().unwrap_or_default(), &typed);
            let now = serde_json::json!({
                "typed": typed,
                "lines": counts.lines,
                "all": counts.all,
                "with_super": counts.with_super,
                "super_of_all": counts.super_of_all,
            })
            .to_string();
            if said != now {
                log::debug!("the shortcut list shows {now}");
                said = now;
            }
        }
        if escape || lost_keyboard(&context, &mut had_keyboard) {
            state.shortcuts = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    state.borrow_mut().shortcuts = Some(id);
}

/// Something a key (or a command) asks of the shell.
fn shell_action(
    state: &Rc<RefCell<State>>,
    action: &str,
    fields: &[(String, String)],
    control: &mut Control,
) {
    let field = |wanted: &str| {
        fields
            .iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, value)| value.as_str())
    };
    match action {
        "launcher" => toggle_launcher(state, control),
        "welcome" => crate::tutoring::open_welcome(state.borrow().link.teller()),
        "tutorial" => {
            let lesson = fields
                .iter()
                .find(|(name, _)| name == "lesson")
                .map(|(_, lesson)| lesson.clone());
            tutorial_start(state, lesson.as_deref(), control);
        }
        "tutorial_next" | "tutorial_back" | "tutorial_leave" => {
            let input = match action {
                "tutorial_next" => gevrek_tutorial::Input::Next,
                "tutorial_back" => gevrek_tutorial::Input::Back,
                _ => gevrek_tutorial::Input::Leave,
            };
            let mut state = state.borrow_mut();
            let state = &mut *state;
            let todo = state
                .tutorial
                .as_mut()
                .map(|(_, tutoring, _)| tutoring.pressed(input, Instant::now()))
                .unwrap_or_default();
            tutorial_do(state, todo);
        }
        "window_switcher" => toggle_switcher(state, control),
        gevrek_model::command::SHELL_WORKSPACE_OVERVIEW => toggle_overview(state, control),
        "notification_center" => toggle_centre(state, control),
        "power_menu" | "logout" => toggle_power(state, control),
        "control_center" => toggle_control(state, control),
        gevrek_model::command::SHELL_SHORTCUTS => toggle_shortcuts(state, control),
        // Said with a command (`gevrek notify`, `gevrek shell osd`).
        "notify" => tell(
            state,
            field("title").unwrap_or_default(),
            field("text").unwrap_or_default(),
            -1,
            control,
        ),
        "osd" => show_osd(
            state,
            Osd::said(field("text").unwrap_or_default(), field("level")),
            control,
        ),
        "do_not_disturb" => {
            let quiet = {
                let mut state = state.borrow_mut();
                state.notes.quiet = !state.notes.quiet;
                for one in state.panels.values() {
                    control.redraw(one.surface);
                }
                state.notes.quiet
            };
            let said = if quiet {
                Osd::words(icons::BELL_SLASH, "Do not disturb")
            } else {
                Osd::words(icons::BELL, "Notifications are shown again")
            };
            show_osd(state, said, control);
        }
        "volume_up" | "volume_down" | "volume_mute" | "mic_mute" => {
            sound_action(state, action, control)
        }
        // The keys for what plays: passed on to whatever plays (the
        // window manager has them bound, so no player heard them).
        "media_play_pause" => crate::media::tell(crate::media::Ask::PlayPause),
        "media_next" => crate::media::tell(crate::media::Ask::Next),
        "media_previous" => crate::media::tell(crate::media::Ask::Previous),
        "media_stop" => crate::media::tell(crate::media::Ask::Stop),
        "brightness_up" | "brightness_down" => {
            let by = if action == "brightness_up" { 5 } else { -5 };
            if let Some(now) = gevrek_system::brightness::change(by) {
                let mut state = state.borrow_mut();
                state.system.brightness = Some(now.percent());
                redraw_panels(&state, control);
            }
        }
        // Pictures of the screen are the system's own on Windows: a part
        // of it by the system's tool for that, all of it and one window
        // by the keys the system has for them (the picture is where the
        // system puts it: the clipboard, or its tool, as it is set up).
        "screenshot_region" => {
            if let Err(error) = std::process::Command::new("explorer.exe")
                .arg("ms-screenclip:")
                .spawn()
            {
                tell(
                    state,
                    "No picture was taken",
                    &error.to_string(),
                    -1,
                    control,
                );
            }
        }
        "screenshot_screen" => gevrek_system::every_key::press(0x2C),
        "screenshot_window" => gevrek_system::every_key::press_chord(&[0x12, 0x2C]),
        // The system's own lock screen.
        "lock" => {
            if let Err(why) = carry_out(Choice::Lock) {
                tell(state, "The session was not locked", &why, -1, control);
            }
        }
        // The system's own command for it; the next key or movement
        // switches them on again.
        "displays_off" => {
            if let Err(why) = ask_system(End::DisplaysOff, "Displays off") {
                tell(
                    state,
                    "The displays were not switched off",
                    &why,
                    -1,
                    control,
                );
            }
        }
        other => log::debug!("gevrek-shell does not do {other} on Windows yet"),
    }
}

/// Open the control centre, or close it when it is open: the sound and
/// where it plays, the brightness, do not disturb, the night light, the
/// keys for what plays, the lock and the power menu. It has the keyboard
/// once something in it is pressed (Escape puts it away then); a press
/// elsewhere puts it away at any time.
///
/// Losing the keyboard does not put it away, unlike the surfaces that are
/// there for the keyboard. It stands beside the windows and can be given
/// the keyboard without being pressed: when the window in front steps
/// aside, even for a moment, Windows activates the next window that may
/// be activated, and that is this one, the topmost. A window that then
/// takes the keyboard back (a TeamViewer session in front, 2026-11-15)
/// closed it within half a second, with nothing pressed. A press elsewhere
/// is heard by itself (`Control::pressed_outside`). The notification
/// centre is the same.
///
/// What Linux has there and Windows not: what is playing by its name (the
/// keys for it are there), the power profiles, bluetooth, and a recording
/// of the screen.
///
/// What the sound has (the devices, the programs that play) is read as
/// it opens, after each thing asked of it, and every few seconds while
/// it is open: a program that begins to play comes into the list.
fn toggle_control(state: &Rc<RefCell<State>>, control: &mut Control) {
    if let Some((id, _, _)) = state.borrow_mut().control_centre.take() {
        if control.is_open(id) {
            control.close(id);
            return;
        }
    }
    let place = Place {
        name: surface_name("gevrek-control-centre"),
        edge: Edge::Right,
        level: Level::Overlay,
        width: crate::control::WIDTH,
        height: 0,
        reserve: false,
        // (It does not take the keyboard from the window that has it:
        // it has the keyboard when something in it is pressed.)
        keyboard: Keyboard::OnDemand,
        margin: 12,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut read_at = Instant::now();
    let id = control.open(place, move |ui, control| {
        let context = ui.ctx().clone();
        // (Not while a slider is held: the list must not move under it.)
        if read_at.elapsed() >= Duration::from_secs(3) {
            read_at = Instant::now();
            let mut state = shared.borrow_mut();
            if let Some((_, held, kept)) = state.control_centre.as_mut() {
                if held.stream.is_none() {
                    *kept = gevrek_system::sound::mixer();
                }
            }
        }
        let asked = {
            let mut state = shared.borrow_mut();
            let state = &mut *state;
            let palette = themed(state, ui, &mut fonts);
            let escape = context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let mixer = state
                .control_centre
                .as_ref()
                .map(|(_, _, mixer)| mixer.clone())
                .unwrap_or_default();
            let facts = crate::control::Facts {
                volume: state.system.volume,
                outputs: mixer.outputs,
                inputs: mixer.inputs,
                streams: mixer.streams,
                brightness: state.system.brightness,
                quiet: state.notes.quiet,
                night_light: state.night_light,
                recording: false,
                profiles: None,
                network: state.system.network.clone(),
                playing: None,
                bluetooth: state.bluetooth.clone(),
                wifi: state.wifi.clone(),
                joining: state.wifi_joining.clone(),
                wifi_looking: state.wifi_looking,
            };
            let Some((_, held, _)) = state.control_centre.as_mut() else {
                return Flow::Close;
            };
            let mut asked =
                crate::control::draw(ui, &palette, &facts, held, &crate::frost::Look::plain());
            // (A press on a window or the desktop puts it away, whether
            // or not it ever had the keyboard. Losing the keyboard does
            // not: see above.)
            let elsewhere = state
                .control_centre
                .as_ref()
                .is_some_and(|(own, ..)| control.pressed_outside(*own));
            if escape || elsewhere {
                asked.push(crate::control::Ask::Close);
            }
            asked
        };
        let mut close = false;
        let mut then: Option<&str> = None;
        for ask in asked {
            use crate::control::Ask;
            use gevrek_system::sound;
            match ask {
                Ask::Close => close = true,
                // (The system tells how loud it is afterwards: `heard`.)
                Ask::Volume(step) => {
                    let level = sound::change(Device::Speaker, step);
                    let mut state = shared.borrow_mut();
                    if let Some(level) = level {
                        state.heard = Some(level);
                        state.system.volume = Some((level.percent, level.muted));
                        redraw_panels(&state, control);
                    }
                }
                Ask::Mute => {
                    let level = sound::toggle_mute(Device::Speaker);
                    let mut state = shared.borrow_mut();
                    if let Some(level) = level {
                        state.heard = Some(level);
                        state.system.volume = Some((level.percent, level.muted));
                        redraw_panels(&state, control);
                    }
                }
                Ask::Output(_) | Ask::Input(_) | Ask::Stream(..) | Ask::StreamMute(_) => {
                    let (what, done) = match ask {
                        Ask::Output(device) => ("to play on", sound::choose_output(device)),
                        Ask::Input(device) => ("to record from", sound::choose_input(device)),
                        Ask::Stream(program, percent) => {
                            ("a program plays at", sound::set_stream(program, percent))
                        }
                        Ask::StreamMute(program) => {
                            ("a program is heard by", sound::toggle_stream_mute(program))
                        }
                        _ => ("", true),
                    };
                    if !done {
                        log::warn!("the sound {what} was not changed");
                    }
                    let mixer = sound::mixer();
                    if let Some((_, _, kept)) = shared.borrow_mut().control_centre.as_mut() {
                        *kept = mixer;
                    }
                    read_at = Instant::now();
                }
                Ask::Brightness(step) => {
                    if let Some(now) = gevrek_system::brightness::change(step) {
                        let mut state = shared.borrow_mut();
                        state.system.brightness = Some(now.percent());
                        redraw_panels(&state, control);
                    }
                }
                Ask::Quiet(quiet) => {
                    let mut state = shared.borrow_mut();
                    state.notes.quiet = quiet;
                    redraw_panels(&state, control);
                }
                Ask::NightLight => {
                    let mut state = shared.borrow_mut();
                    // (Shown at once; the configuration says the same
                    // when it comes.)
                    state.night_light = !state.night_light;
                    let command = serde_json::json!({"command": "shell", "action": "night_light"});
                    state.link.send(Request::Dispatch(command.to_string()));
                }
                Ask::Media(what) => crate::media::tell(what),
                Ask::Lock => {
                    close = true;
                    then = Some("lock");
                }
                Ask::PowerMenu => {
                    close = true;
                    then = Some("power_menu");
                }
                // Not there on Windows; said, so that the button is no lie.
                Ask::Record => then = Some("record"),
                // The radios and the networks: asked beside the shell,
                // what came of it is told when it is known.
                Ask::Wifi(on) => {
                    let sender = shared.borrow().sender.clone();
                    std::thread::spawn(move || {
                        gevrek_system::wifi::set_enabled(on);
                        if on {
                            // (A radio that has just come on has heard
                            // nothing yet.)
                            look_for_networks(&sender);
                        } else {
                            sender.send(Message::Wifi(gevrek_system::wifi::read()));
                        }
                        sender.send(Message::Network(gevrek_system::status::network()));
                    });
                }
                Ask::WifiJoin {
                    name,
                    known,
                    password,
                } => {
                    let sender = {
                        let mut state = shared.borrow_mut();
                        state.wifi_joining = Some(name.clone());
                        state.sender.clone()
                    };
                    std::thread::spawn(move || {
                        if !gevrek_system::wifi::join(&name, known, password.as_deref()) {
                            log::warn!("the wireless network {name} was not joined");
                        }
                        sender.send(Message::Wifi(gevrek_system::wifi::read()));
                        sender.send(Message::Network(gevrek_system::status::network()));
                    });
                    context.request_repaint();
                }
                Ask::Bluetooth(on) => {
                    let sender = shared.borrow().sender.clone();
                    std::thread::spawn(move || {
                        gevrek_system::bluetooth::set_power(on);
                        // (The radio takes a moment to follow: told as
                        // it stands until it has.)
                        for _ in 0..12 {
                            let now = gevrek_system::bluetooth::read();
                            let there = now.as_ref().is_none_or(|radio| radio.powered == on);
                            sender.send(Message::Bluetooth(now));
                            if there {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(500));
                        }
                    });
                }
                // (Windows lets no program connect a device or let it
                // go: its own page does, and that is opened.)
                Ask::BluetoothDevice(..) => {
                    close = true;
                    if let Err(error) = std::process::Command::new("explorer.exe")
                        .arg("ms-settings:bluetooth")
                        .spawn()
                    {
                        log::warn!("Windows' bluetooth page was not opened: {error}");
                    }
                }
                Ask::Profile(_) => {}
            }
        }
        match then {
            Some("record") => tell(
                &shared,
                "Recording the screen",
                "gevrek does not record the screen on Windows yet.",
                -1,
                control,
            ),
            Some(action) => shell_action(&shared, action, &[], control),
            None => {}
        }
        if close {
            shared.borrow_mut().control_centre = None;
            return Flow::Close;
        }
        context.request_repaint_after(Duration::from_millis(250));
        Flow::Continue
    });
    // (What the sound has is asked of the system as it opens.)
    let mixer = gevrek_system::sound::mixer();
    // How bluetooth and the wireless side stand is asked beside the
    // shell: it takes its moment, and what was last known is shown
    // until it comes.
    let sender = state.borrow().sender.clone();
    std::thread::spawn(move || {
        sender.send(Message::Bluetooth(gevrek_system::bluetooth::read()));
        let wifi = gevrek_system::wifi::read();
        let none_heard = wifi
            .as_ref()
            .is_some_and(|wifi| wifi.enabled && wifi.networks.is_empty());
        sender.send(Message::Wifi(wifi));
        if none_heard {
            look_for_networks(&sender);
        }
    });
    state.borrow_mut().control_centre = Some((id, crate::control::Held::default(), mixer));
}

/// Begin the tutorial (with this lesson, or the one to take next), or
/// nothing when it is taken already: its card comes up at the bottom of
/// the display, over everything, and never has the keyboard.
fn tutorial_start(state: &Rc<RefCell<State>>, lesson: Option<&str>, control: &mut Control) {
    if state.borrow().tutorial.is_some() {
        return;
    }
    let now = Instant::now();
    let begun = crate::tutoring::Tutoring::begin(
        lesson,
        gevrek_tutorial::progress::file(),
        &state.borrow().facts,
        now,
    );
    let Some((tutoring, todo)) = begun else {
        log::warn!("the tutorial has no lesson to take");
        return;
    };
    log::info!("the tutorial begins with the lesson {}", tutoring.lesson().id);
    let place = Place {
        name: surface_name("gevrek-coach"),
        edge: Edge::Bottom,
        level: Level::Overlay,
        width: crate::coach::SIZE.0,
        height: crate::coach::SIZE.1,
        reserve: false,
        keyboard: Keyboard::Never,
        margin: 28,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let id = control.open(place, move |ui, _| {
        use crate::tutoring::Shown;
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let now = Instant::now();
        // The desktop as it stands, with the shell's own surfaces that
        // are up (they come and go without a word from the window
        // manager).
        let mut facts = state.facts.clone();
        if state.launcher.is_some() {
            facts.surfaces.push("gevrek-launcher".to_string());
        }
        if state.control_centre.is_some() {
            facts.surfaces.push("gevrek-control-centre".to_string());
        }
        let still = state.settings.reduce_motion;
        let Some((_, tutoring, _)) = state.tutorial.as_mut() else {
            return Flow::Close;
        };
        let mut todo = tutoring.facts(facts, now);
        todo.extend(tutoring.tick(now));
        let pressed = match tutoring.shown(now) {
            Shown::Setting(lesson) => crate::coach::draw_setting(ui, &palette, lesson),
            Shown::Step {
                lesson,
                at,
                step,
                state: step_state,
                help,
                paused,
            } => {
                let keys = match (step.act, state.config.as_deref()) {
                    (gevrek_tutorial::lessons::Act::Command(command), Some(config)) => {
                        gevrek_tutorial::keys_for(config, command)
                    }
                    _ => Vec::new(),
                };
                crate::coach::draw(
                    ui,
                    &palette,
                    &crate::coach::View {
                        lesson,
                        at,
                        step,
                        keys,
                        state: step_state,
                        help,
                        paused,
                        still,
                    },
                )
            }
            Shown::Between { done, next, count } => {
                crate::coach::draw_between(ui, &palette, done, next, count)
            }
        };
        for input in pressed {
            todo.extend(tutoring.pressed(input, now));
        }
        tutorial_do(state, todo);
        if state.tutorial.is_none() {
            return Flow::Close;
        }
        // (Looked at again a few times a second: a step is done, help
        // comes, by time and by what the window manager says.)
        ui.ctx().request_repaint_after(Duration::from_millis(200));
        Flow::Continue
    });
    let mut state = state.borrow_mut();
    let state = &mut *state;
    state.tutorial = Some((id, tutoring, Vec::new()));
    tutorial_do(state, todo);
}

/// Carry out what the tutorial asks for: commands for the window
/// manager, practice windows, its end.
fn tutorial_do(state: &mut State, todo: Vec<crate::tutoring::Do>) {
    use crate::tutoring::Do;
    for one in todo {
        match one {
            Do::Dispatch(command) => state.link.send(Request::Dispatch(command)),
            Do::OpenPractice(count) => {
                if let Some((_, _, windows)) = state.tutorial.as_mut() {
                    crate::tutoring::open_practice(windows, count);
                }
            }
            Do::End => {
                if let Some((_, tutoring, windows)) = state.tutorial.take() {
                    log::info!("the tutorial ends");
                    crate::tutoring::tidy(tutoring.before.clone(), windows, state.link.teller());
                }
            }
        }
    }
}

/// Ask the system to look for wireless networks and tell how the wireless
/// side stands until some are heard (a radio that has just come on has
/// none at first), for a dozen seconds at most.
fn look_for_networks(sender: &gevrek_shellkit::host::Sender<Message>) {
    sender.send(Message::WifiLooking(true));
    gevrek_system::wifi::rescan();
    for round in 0..8 {
        let now = gevrek_system::wifi::read();
        let done = now
            .as_ref()
            .is_none_or(|wifi| !wifi.enabled || !wifi.networks.is_empty());
        sender.send(Message::Wifi(now));
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(1500));
        // (Asked again half way: the first asking may have come before
        // the radio was ready to look.)
        if round == 3 {
            gevrek_system::wifi::rescan();
        }
    }
    sender.send(Message::WifiLooking(false));
}

/// The window switcher on its key: up, or (its key again while it is up:
/// the key is the window manager's, the surface never sees it) on to the
/// next window. While the key's modifier is held it stays, and letting go
/// shows the window the choice is on; asked for without one (a command),
/// it stays until Enter, a click or Escape.
fn toggle_switcher(state: &Rc<RefCell<State>>, control: &mut Control) {
    let open = {
        let mut state = state.borrow_mut();
        match state.switcher.as_mut() {
            Some((id, switcher)) if control.is_open(*id) => {
                // (With Shift held: back.)
                switcher.step(if gevrek_system::every_key::shift_held() {
                    -1
                } else {
                    1
                });
                Some(*id)
            }
            _ => None,
        }
    };
    if let Some(id) = open {
        control.redraw(id);
        return;
    }
    let entries = {
        let state = state.borrow();
        state.used.order(state.switch_to.clone())
    };
    let (width, height) = crate::switcher::size(entries.len());
    let place = Place {
        name: surface_name("gevrek-switcher"),
        edge: Edge::Centre,
        level: Level::Overlay,
        width,
        height,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut had_keyboard = false;
    let mut said = String::new();
    let id = control.open(place, move |ui, _| {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let Some((_, switcher)) = state.switcher.as_mut() else {
            return Flow::Close;
        };
        // (No icons here: a window's class is no application's name on
        // Windows. The line shows a letter, and the picture the window.)
        let asked = crate::switcher::draw_held(
            ui,
            &palette,
            switcher,
            &mut |_, _| None,
            Some(gevrek_system::every_key::modifier_held()),
        );
        // Said when asked to: the desktop check reads which window the
        // choice is on before it lets go.
        if log::log_enabled!(log::Level::Debug) {
            let chosen = switcher.entries.get(switcher.chosen);
            let now = serde_json::json!({
                "windows": switcher.entries.len(),
                "chosen": chosen.map(|entry| entry.id),
                "title": chosen.map(|entry| entry.title.clone()),
                "pictures": switcher.thumbs.iter().filter(|(thumb, _)| thumb.width > 0).count(),
            })
            .to_string();
            if said != now {
                log::debug!("the switcher shows {now}");
                said = now;
            }
        }
        if let Some(window) = asked.go {
            let command = serde_json::json!({
                "command": "focus_window",
                "selector": {"kind": "handle", "value": window},
            });
            state.link.send(Request::Dispatch(command.to_string()));
        }
        // A click elsewhere takes the keyboard: put away, as the launcher.
        if asked.close || lost_keyboard(&context, &mut had_keyboard) {
            state.switcher = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    // The pictures of the windows, taken beside the switcher: it is up at
    // once, and shows them when they are there. In the order of the
    // windows' numbers (windows of one title are told apart by it).
    let mut wanted: Vec<(u64, String, String)> = entries
        .iter()
        .map(|entry| (entry.id, entry.title.clone(), entry.class.clone()))
        .collect();
    wanted.sort_by_key(|(id, _, _)| *id);
    let sender = {
        let mut state = state.borrow_mut();
        state.switcher = Some((id, crate::switcher::Switcher::new(entries)));
        state.sender.clone()
    };
    let _ = std::thread::Builder::new()
        .name("gevrek-shell-thumbs".to_string())
        .spawn(move || {
            let thumbs = wanted
                .into_iter()
                .map(|(id, title, app_id)| {
                    let picture = gevrek_system::pictures::of_window(id as isize, 640);
                    let (width, height, rgba) = picture
                        .map(|picture| (picture.width, picture.height, picture.rgba))
                        .unwrap_or_default();
                    crate::switcher::Thumb {
                        title,
                        app_id,
                        width,
                        height,
                        rgba,
                    }
                })
                .collect();
            sender.send(Message::Thumbs(thumbs));
        });
}

/// Take pictures of windows, away from the thread that draws (a window
/// draws itself into memory for it, which takes as long as the window
/// takes), `after` a moment where the window has only just come into
/// sight. A window whose picture was taken a moment ago is left out.
fn take_pictures(state: &mut State, windows: Vec<u64>, after: Duration) {
    let now = Instant::now();
    let wanted: Vec<u64> = windows
        .into_iter()
        .filter(|window| state.pictures.wants(*window, now))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let sender = state.sender.clone();
    let _ = std::thread::Builder::new()
        .name("gevrek-shell-pictures".to_string())
        .spawn(move || {
            std::thread::sleep(after);
            for window in wanted {
                if let Some(picture) = gevrek_system::pictures::of_window(window as isize, 420) {
                    sender.send(Message::Pictured(window, picture));
                }
            }
        });
}

/// The overview of every workspace (`workspace_overview`): up, or put
/// away when it is up. Every display's workspaces in miniature, each
/// window in its place with the picture that is remembered of it; a
/// press or the keys go to a workspace or a window (`overview.rs`).
fn toggle_overview(state: &Rc<RefCell<State>>, control: &mut Control) {
    if let Some((id, _)) = state.borrow_mut().overview.take() {
        if control.is_open(id) {
            control.close(id);
            return;
        }
    }
    let (size, in_sight) = {
        let state = state.borrow();
        // (As large as what it shows, within the display with the keyboard.)
        let room = state
            .rooms
            .iter()
            .find(|room| room.focused)
            .or(state.rooms.first())
            .map_or((1600.0, 900.0), |room| {
                let scale = room.scale.max(0.5);
                (room.work.2 as f32 / scale, room.work.3 as f32 / scale)
            });
        (
            crate::overview::Overview::size(&state.spaces, room),
            crate::overview::in_sight(&state.spaces),
        )
    };
    let place = Place {
        name: surface_name("gevrek-overview"),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: size.0,
        height: size.1,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    let mut had_keyboard = false;
    let mut said = String::new();
    let id = control.open(place, move |ui, _| {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        let palette = themed(state, ui, &mut fonts);
        let Some((_, overview)) = state.overview.as_mut() else {
            return Flow::Close;
        };
        let asked = overview.draw(ui, &palette, &state.spaces, &state.pictures);
        // Said when asked to: what it shows, for the desktop check.
        if log::log_enabled!(log::Level::Debug) {
            let now = serde_json::json!({
                "displays": state.spaces.len(),
                "workspaces": state
                    .spaces
                    .iter()
                    .flat_map(|display| &display.spaces)
                    .map(|space| space.name.clone())
                    .collect::<Vec<_>>(),
                "windows": state
                    .spaces
                    .iter()
                    .flat_map(|display| &display.spaces)
                    .map(|space| space.panes.len())
                    .sum::<usize>(),
                "pictures": state
                    .spaces
                    .iter()
                    .flat_map(|display| &display.spaces)
                    .flat_map(|space| &space.panes)
                    .filter(|pane| state.pictures.of(pane.id).is_some())
                    .count(),
            })
            .to_string();
            if said != now {
                log::debug!("the overview shows {now}");
                said = now;
            }
        }
        if let Some(go) = &asked.go {
            state.link.send(Request::Dispatch(go.command()));
        }
        // A click elsewhere takes the keyboard: put away, as the launcher.
        if asked.close || asked.go.is_some() || lost_keyboard(&context, &mut had_keyboard) {
            state.overview = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    let mut state = state.borrow_mut();
    state.overview = Some((id, Default::default()));
    // What is in sight is taken a picture of now: as it is.
    take_pictures(&mut state, in_sight, Duration::ZERO);
    state.link.send(Request::State);
}

/// Open the launcher, or close it when it is open.
fn toggle_launcher(state: &Rc<RefCell<State>>, control: &mut Control) {
    let open = state.borrow_mut().launcher.take();
    if let Some((id, _)) = open {
        // One the host closed itself (its display went) is not open:
        // asked for, the launcher then opens.
        if control.is_open(id) {
            control.close(id);
            return;
        }
    }
    let place = Place {
        name: surface_name("gevrek-launcher"),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: launcher::SIZE.0,
        height: launcher::SIZE.1,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        output: None,
    };
    let shared = Rc::clone(state);
    let mut fonts = false;
    // The icons this surface has as pictures of its own, by name.
    let mut pictures: HashMap<String, egui::TextureHandle> = HashMap::new();
    let mut had_keyboard = false;
    let mut said = String::new();
    let id = control.open(place, move |ui, control| {
        let context = ui.ctx().clone();
        let mut held = shared.borrow_mut();
        let state = &mut *held;
        // What could not be done, where something could not: said as a
        // notification once the launcher is put away.
        let mut problem: Option<String> = None;
        if !fonts {
            theme::install_fonts(&context, state.settings.system_font);
            fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let Some((_, launcher)) = state.launcher.as_mut() else {
            return Flow::Close;
        };
        let icons = &mut state.icons;
        let hint = format!(
            "{}  An application · > a command · Ctrl+Enter: as administrator",
            theme::icons::MAGNIFYING_GLASS
        );
        let asked = launcher::draw(ui, &palette, launcher, &hint, &mut |context, entry| {
            if let Some(picture) = pictures.get(&entry.icon) {
                return Some(picture.clone());
            }
            // Not there yet: the line shows a letter, and is drawn again
            // when the icon was found.
            let found = icons.get(&entry.icon, ICON)?;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [found.width as usize, found.height as usize],
                &found.pixels,
            );
            let picture =
                context.load_texture(entry.icon.clone(), image, egui::TextureOptions::LINEAR);
            pictures.insert(entry.icon.clone(), picture.clone());
            Some(picture)
        });
        // Said when asked to (`RUST_LOG=gevrek_shell=debug`), each time
        // it is another: what is typed, how many lines answer, and the
        // line Enter would choose. The desktop check reads it before it
        // presses Enter, so that it starts nothing but what it means to.
        if log::log_enabled!(log::Level::Debug) {
            let entries = launcher.entries();
            let chosen = entries.get(launcher.chosen).map(|entry| {
                serde_json::json!({
                    "title": entry.title,
                    "app": entry.app,
                    "command": entry.command,
                })
            });
            let now = serde_json::json!({
                "typed": launcher.typed,
                "lines": entries.len(),
                "chosen": chosen,
            })
            .to_string();
            if said != now {
                log::debug!("the launcher shows {now}");
                said = now;
            }
        }
        // The keyboard went to another window (a click elsewhere): the
        // launcher is put away, as a menu is. Not before it ever had the
        // keyboard: it is drawn once before it hears that it has.
        let has_keyboard = context.input(|input| input.focused);
        had_keyboard |= has_keyboard;
        let mut close = asked.close || (had_keyboard && !has_keyboard);
        if let Some(entry) = asked.run {
            let app = entry.app.as_deref().and_then(|id| launcher.app(id));
            let elevated = asked.elevated;
            match (&entry.copy, app) {
                // With an administrator's rights: Windows asks first.
                (None, Some(app)) if elevated => {
                    match gevrek_system::apps::start_elevated(app) {
                        Ok(_) => log::info!("started {} as administrator", app.name),
                        // (1223: the person said no to Windows' question.)
                        Err(error) if error.raw_os_error() == Some(1223) => {
                            log::info!("{} was not started: the question was answered with no", app.name)
                        }
                        Err(error) => {
                            log::warn!("{} could not be started as administrator: {error}", app.name);
                            problem = Some(format!(
                                "{} could not be started as administrator. An application from the Store cannot be.",
                                app.name
                            ));
                        }
                    }
                }
                // A text that was copied before: on the clipboard again.
                (Some(text), _) => {
                    if let Some(clipboard) = &state.clipboard {
                        clipboard.put(text);
                    }
                }
                (None, Some(app)) => match gevrek_system::apps::start(app) {
                    Ok(Some(process)) => log::info!("started {} as process {process}", app.name),
                    Ok(None) => log::info!("started {}", app.name),
                    Err(error) => log::warn!("{} could not be started: {error}", app.name),
                },
                (None, None) => state
                    .link
                    .send(Request::Dispatch(entry.command.to_string())),
            }
            if let Some(app) = &entry.app {
                let path = launcher::usage_file();
                launcher::Usage::load(&path).bump(app, &path);
            }
            close = true;
        }
        if close {
            state.launcher = None;
            drop(held);
            if let Some(problem) = problem {
                tell(&shared, "Not started", &problem, -1, control);
            }
            return Flow::Close;
        }
        Flow::Continue
    });
    {
        let mut state = state.borrow_mut();
        // Applications are started here, none in a terminal: no word for one.
        let launcher = Launcher::new(state.apps.clone(), "")
            .with(
                state.open.clone(),
                launcher::Usage::load(&launcher::usage_file()),
            )
            // What was copied lately, after a `:`.
            .with_copied(
                state
                    .clipboard
                    .as_ref()
                    .map(|clipboard| clipboard.copied())
                    .unwrap_or_default(),
            );
        state.launcher = Some((id, launcher));
    }
    // What was installed since is there the next time, or in a moment.
    read_apps(state);
}

/// How long a notice of the window manager stays as a notification, in
/// milliseconds; its mark on the panel stays for as long as it holds.
const NOTICE_STAY_MS: i32 = 12_000;

/// What the window manager said since the last time.
fn take_in(state: &Rc<RefCell<State>>, control: &mut Control) {
    let events = state.borrow().link.poll();
    for event in events {
        match &event {
            Event::Runtime { kind, window, .. } => log::trace!("heard {kind} {window:?}"),
            Event::Log { .. } => log::trace!("heard a line of the log"),
            _ => {}
        }
        // The tutorial hears which commands were carried out: some of
        // what it teaches leaves nothing else to look at.
        if let Event::Runtime { kind, fields, .. } = &event {
            if kind == "dispatch.succeeded" {
                let name = fields
                    .iter()
                    .find(|(name, _)| name == "payload")
                    .and_then(|(_, payload)| serde_json::from_str::<serde_json::Value>(payload).ok())
                    .and_then(|payload| payload["command"].as_str().map(str::to_string));
                if let Some(name) = name {
                    let mut state = state.borrow_mut();
                    let state = &mut *state;
                    let todo = state
                        .tutorial
                        .as_mut()
                        .map(|(_, tutoring, _)| tutoring.carried_out(&name, Instant::now()))
                        .unwrap_or_default();
                    tutorial_do(state, todo);
                }
            }
        }
        match event {
            // A key bound to something of the shell's was pressed.
            Event::Runtime { kind, fields, .. } if kind == "shell.command" => {
                let action = fields
                    .iter()
                    .find(|(name, _)| name == "action")
                    .map(|(_, value)| value.as_str());
                if let Some(action) = action {
                    shell_action(state, action, &fields, control);
                }
            }
            Event::Configuration(config, revision) => {
                let (red, green, blue) = config.desktop.appearance.active_border_color;
                {
                    let mut state = state.borrow_mut();
                    state.revision = revision;
                    state.accent = egui::Color32::from_rgb(red, green, blue);
                    state.asked_look = (
                        config.desktop.animations.reduce_motion,
                        config.desktop.appearance.high_contrast,
                    );
                    follow_look(&mut state);
                    let tell = state.link.teller();
                    crate::tutoring::welcome_if_due(&config, &mut state.welcome_asked, tell);
                    let bar = config.desktop.shell.panel.clone();
                    // Said once for each set of them, where the user
                    // looks when something is not there: the log.
                    let out = panel::left_out(&bar);
                    if out != state.said_left_out {
                        if !out.is_empty() {
                            log::warn!(
                                "the panel's modules {out:?} are not drawn: gevrek-shell does not have what they open on Windows yet"
                            );
                        }
                        state.said_left_out = out;
                    }
                    state.bar = Some(bar);
                    state.keys = Some(gevrek_model::shortcuts::shortcuts_of(&config));
                    state.config = Some(config.clone());
                    state.night_light = config.desktop.night_light.enabled;
                    state.show_volume = config.desktop.shell.osd.volume;
                    // Notifications: where, how many at once, how long.
                    let notes = &config.desktop.shell.notifications;
                    state.notes.configure(notes.shown_at_once(), notes.stay());
                    state.notes_corner = notes.corner;
                    state.link.send(Request::State);
                    // The look may be another.
                    for one in state.panels.values() {
                        control.redraw(one.surface);
                    }
                    if let Some((surface, _)) = &state.launcher {
                        control.redraw(*surface);
                    }
                    if let Some(surface) = state.shortcuts {
                        control.redraw(surface);
                    }
                }
                sync_panels(state, control);
            }
            Event::State(wm) => {
                let arisen = {
                    let mut state = state.borrow_mut();
                    let state = &mut *state;
                    state.facts = gevrek_tutorial::Facts::of(&wm);
                    crate::tutoring::tidy_after_a_stop(&wm, &mut state.tidy_asked, state.link.teller());
                    if let Some((surface, ..)) = &state.tutorial {
                        control.redraw(*surface);
                    }
                    state.places = panel::places(&wm);
                    state.rooms = panel::rooms(&wm);
                    state.windows = panel::windows(&wm);
                    state.open = launcher::open_windows(&wm);
                    state.switch_to = crate::switcher::entries(&wm);
                    if let Some(window) = crate::switcher::with_keyboard(&wm) {
                        state.used.note(window);
                    }
                    // For the overview of the workspaces: what is where,
                    // and a picture of a window as it gets and as it
                    // loses the keyboard (it is in sight then; put away,
                    // nobody draws it).
                    state.spaces = crate::overview::displays(&wm);
                    let there: Vec<u64> = state
                        .spaces
                        .iter()
                        .flat_map(|display| &display.spaces)
                        .flat_map(|space| &space.panes)
                        .map(|pane| pane.id)
                        .collect();
                    state.pictures.only(&there);
                    let keyboard = crate::switcher::with_keyboard(&wm);
                    if keyboard != state.had_keyboard {
                        let before = std::mem::replace(&mut state.had_keyboard, keyboard);
                        take_pictures(state, before.into_iter().collect(), Duration::ZERO);
                        take_pictures(
                            state,
                            keyboard.into_iter().collect(),
                            Duration::from_millis(500),
                        );
                    }
                    if let Some((surface, _)) = &state.overview {
                        control.redraw(*surface);
                    }
                    state.views = panel::displays(&wm)
                        .into_iter()
                        .enumerate()
                        .map(|(index, (name, monitor))| {
                            (name, panel::view_at(monitor, Some(index)))
                        })
                        .collect();
                    // What every panel shows.
                    let notices = panel::notices(&wm);
                    // A notice that has arisen is told as a notification
                    // too; its mark on the panel stays while it holds.
                    let arisen = panel::arisen(&notices, &mut state.told);
                    let mode = panel::mode(&wm).to_string();
                    let all = *state.notices != notices || state.mode != mode;
                    if *state.notices != notices {
                        state.notices = Rc::new(notices);
                    }
                    if state.mode != mode {
                        // (Said when asked to: a check reads it.)
                        log::debug!("the panel is told the keys' mode {mode:?}");
                    }
                    state.mode = mode;
                    // Drawn again only where something is different.
                    for (name, one) in &mut state.panels {
                        let view = state.views.get(name).cloned().unwrap_or_default();
                        if all || one.view != view {
                            one.view = view;
                            control.redraw(one.surface);
                        }
                    }
                    arisen
                };
                sync_panels(state, control);
                for notice in arisen {
                    // Longer than the usual: there is more to read.
                    tell(state, &notice.title, &notice.text, NOTICE_STAY_MS, control);
                }
            }
            Event::Runtime { kind, window, .. }
                if panel::concerns(&kind, window, &state.borrow().windows) =>
            {
                log::debug!("asking for the state after {kind}");
                let state = state.borrow();
                state.link.send(Request::State);
                // What the window manager reports follows an event by a
                // tick: asked at once, the answer can be the old one.
                state
                    .link
                    .send(Request::StateAfter(Duration::from_millis(180)));
            }
            // The look follows the configuration.
            Event::Changed { .. } => state.borrow().link.send(Request::Fetch),
            // What the shell itself changed is what runs, from this
            // revision on.
            Event::Applied(revision) => state.borrow_mut().revision = revision,
            // A change of the shell's that was not taken: said, and what
            // runs is fetched again.
            Event::Conflict(why) | Event::ApplyFailed(why) => {
                log::warn!("a change of the shell's was not kept: {why}");
                state.borrow().link.send(Request::Fetch);
                tell(state, "The change was not kept", &why, -1, control);
            }
            // A command of the shell's that the window manager could not
            // carry out: said, not swallowed.
            Event::Dispatched {
                command,
                error: Some(error),
            } => {
                log::warn!("the window manager did not carry out {command}: {error}");
                tell(state, "That did not work", &error, -1, control);
            }
            // No window manager: nobody keeps the panel's room, and there
            // is nothing to show. The panels are back when it is.
            Event::Disconnected(_) => {
                {
                    let mut state = state.borrow_mut();
                    state.places.clear();
                    state.rooms.clear();
                    state.views.clear();
                    state.windows.clear();
                }
                sync_panels(state, control);
            }
            _ => {}
        }
    }
}

/// Bring the panels in line with the window manager's state and the
/// configuration: one on each display the window manager keeps room on
/// while the panel is switched on, at that room.
fn sync_panels(state: &Rc<RefCell<State>>, control: &mut Control) {
    let mut shared = state.borrow_mut();
    let shared = &mut *shared;
    let Some(bar) = shared.bar.clone() else {
        return;
    };
    let wanted = if bar.enabled && shared.with_panel {
        shared.places.clone()
    } else {
        Vec::new()
    };
    // The icons programs give the taskbar are heard of while a panel has
    // a place for them, and no longer.
    let with_tray = !wanted.is_empty() && bar.shows(PanelModule::Tray);
    if with_tray && shared.tray.is_none() {
        let sender = shared.sender.clone();
        shared.tray = gevrek_system::tray_host::Host::start(move |icons| {
            sender.send(Message::Tray(icons));
        });
        if shared.tray.is_none() {
            log::warn!(
                "the tray's icons are not shown: Windows gave no window to hear of them with"
            );
        }
    } else if !with_tray && shared.tray.is_some() {
        shared.tray = None;
        shared.tray_items.clear();
        shared.tray_all.clear();
    }
    // A panel the host closed itself (its display went) is not open.
    shared.panels.retain(|_, one| control.is_open(one.surface));
    let open: Vec<(String, panel::Rect)> = shared
        .panels
        .iter()
        .map(|(name, one)| (name.clone(), one.at))
        .collect();
    let plan = panel::plan(&open, &wanted);
    for name in plan.close {
        if let Some(one) = shared.panels.remove(&name) {
            control.close(one.surface);
        }
    }
    let area = |(x, y, width, height): panel::Rect| Area::new(x, y, width, height);
    for (name, at) in plan.put {
        if let Some(one) = shared.panels.get_mut(&name) {
            one.at = at;
            control.put_at(one.surface, area(at));
        }
    }
    for (name, at) in plan.open {
        // The rectangle is the window manager's; of the place only the
        // name, the level and the keyboard count (`Control::open_at`).
        let place = Place {
            name: shared.name.clone(),
            edge: match bar.edge {
                PanelEdge::Top => Edge::Top,
                PanelEdge::Bottom => Edge::Bottom,
                PanelEdge::Left => Edge::Left,
                PanelEdge::Right => Edge::Right,
            },
            level: Level::Above,
            width: 0,
            height: at.3.max(1) as u32,
            // The window manager keeps the room.
            reserve: false,
            // A click on the panel leaves the keyboard where it is.
            keyboard: Keyboard::Never,
            margin: 0,
            output: None,
        };
        let drawn = Rc::clone(state);
        let display = name.clone();
        let mut fonts = false;
        let mut wheel = crate::panel::Wheel::default();
        // Its hint, from frame to frame.
        let mut hints = crate::panel::Hints::default();
        let surface = control.open_at(place, area(at), move |ui, control| {
            draw_panel(
                &drawn, &display, ui, &mut fonts, &mut wheel, &mut hints, control,
            )
        });
        let view = shared.views.get(&name).cloned().unwrap_or_default();
        shared.panels.insert(name, One { surface, at, view });
    }
}

fn draw_panel(
    shared: &Rc<RefCell<State>>,
    display: &str,
    ui: &mut egui::Ui,
    fonts: &mut bool,
    wheel: &mut crate::panel::Wheel,
    hints: &mut crate::panel::Hints<SurfaceId>,
    control: &mut Control,
) -> Flow {
    let context = ui.ctx().clone();
    let state = shared.borrow();
    if !*fonts {
        theme::install_fonts(&context, state.settings.system_font);
        *fonts = true;
    }
    let (Some(bar), Some(one)) = (state.bar.as_ref(), state.panels.get(display)) else {
        return Flow::Continue;
    };
    let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
    theme::apply(&context, &palette, &state.settings);
    let (surface, view) = (one.surface, one.view.clone());
    let asked = panel::draw(
        ui,
        &panel::Shown {
            bar,
            view: &view,
            accent: state.accent,
            quiet: state.notes.quiet,
            notices: state.notices.len(),
            mode: &state.mode,
            recording: false,
            sharing: false,
            output: display,
            // (Empty unless the panel has a place for them: Windows has
            // its own tray, on its taskbar.)
            tray: &state.tray_items,
            system: &state.system,
            keyboard: None,
        },
        Some(wheel),
    );
    // The word about what the pointer rests on, on a surface of its own
    // that the pointer goes through: under the panel, over it when the
    // panel is at the bottom, beside it when it goes down a side.
    {
        use crate::panel::HintStep;
        let wanted = asked.hint.as_ref().map(|(_, words)| words.as_str());
        match hints.step(wanted, std::time::Instant::now()) {
            HintStep::Nothing => {}
            HintStep::Close(hint) => control.close(hint),
            HintStep::Wait(long) => context.request_repaint_after(long),
            HintStep::Open(before) => {
                if let Some(hint) = before {
                    control.close(hint);
                }
                if let Some((anchor, words)) = asked.hint.clone() {
                    let size = panel::hint_size(ui, &words);
                    let look = bar.clone();
                    let said = words.clone();
                    let mut fonts = false;
                    let system_font = state.settings.system_font;
                    let beside = bar.edge.is_upright();
                    // The hint lies outside the panel, however small what
                    // it is about is: its rectangle is as deep as the panel
                    // (the surface is the panel here).
                    let whole = context.content_rect();
                    let anchor = if beside {
                        egui::Rect::from_x_y_ranges(whole.x_range(), anchor.y_range())
                    } else {
                        egui::Rect::from_x_y_ranges(anchor.x_range(), whole.y_range())
                    };
                    log::debug!("the panel of {display} shows the hint {words:?}");
                    let hint = control.hint(surface, anchor, size, beside, move |ui, _| {
                        if !fonts {
                            theme::install_fonts(ui.ctx(), system_font);
                            fonts = true;
                        }
                        panel::draw_hint(ui, &look, &said);
                        Flow::Continue
                    });
                    hints.shown = Some((hint, words));
                }
            }
        }
    }
    // An icon of the tray was pressed: its program is told, as the
    // taskbar tells it (the menu is the program's own).
    // (The tray's own button opens the list of all icons, below.)
    let tray_list = asked
        .tray
        .as_ref()
        .filter(|(key, _, _)| key == TRAY_MORE)
        .map(|(_, _, anchor)| *anchor);
    if let (Some((key, menu, _)), Some(host), None) = (&asked.tray, &state.tray, tray_list) {
        use gevrek_system::tray_host::Press;
        host.press(key, if *menu { Press::Menu } else { Press::Main });
    }
    if let Some(id) = asked.go {
        state.link.send(Request::Dispatch(
            serde_json::json!({"command": "focus_window", "selector": {"kind": "handle", "value": id}})
                .to_string(),
        ));
    }
    // By the display and the workspace's name, not by its place among
    // the tabs: a place that is stale by the time the window manager
    // hears of it would name another workspace, or make one.
    if let Some(command) = asked.show.and_then(|index| panel::show_tab(&view, index)) {
        state.link.send(Request::Dispatch(command));
    }
    // (Open as the host has it, too: a menu the host put away for a press
    // elsewhere or for Escape is closed before its drawing is let go of,
    // and the next press on its button is to open it, not to close it.)
    let open = state
        .menu
        .as_ref()
        .filter(|(id, _, alive)| alive.strong_count() > 0 && control.is_open(*id))
        .map(|(id, which, _)| (*id, *which));
    let notices = Rc::clone(&state.notices);
    drop(state);
    // The taskbar's own button: whose windows it has. Kept in the
    // configuration, so it is the same after gevrek is started again.
    if let Some(all) = asked.all_tasks {
        let mut state = shared.borrow_mut();
        let state = &mut *state;
        if let Some(bar) = state.bar.as_mut() {
            // (At once here; the configuration that comes back says the same.)
            bar.tasks_of_all_workspaces = all;
        }
        if let Some(mut changed) = state.config.clone() {
            changed.desktop.shell.panel.tasks_of_all_workspaces = all;
            apply_configuration(state, changed);
        }
        redraw_panels(state, control);
    }
    // A right-click on a workspace's tab: its new name is asked for.
    if let Some(tab) = asked.rename.and_then(|index| view.tabs.get(index)) {
        ask_workspace_name(shared, control, view.place.clone(), &tab.name, &tab.label);
    }
    if asked.sound.is_some() || asked.show.is_some() {
        log::debug!(
            "the panel of {display} is asked: sound {:?}, workspace {:?} (the pointer over the sound: {})",
            asked.sound,
            asked.show,
            asked.over_sound
        );
    }
    if let Some(ask) = asked.sound {
        panel_sound(shared, ask, control);
    }
    // The wheel over the brightness.
    if let Some(by) = asked.brightness {
        if let Some(now) = gevrek_system::brightness::change(by) {
            let mut state = shared.borrow_mut();
            state.system.brightness = Some(now.percent());
            redraw_panels(&state, control);
        }
    }
    match asked.pressed {
        Some(PanelModule::Launcher) => toggle_launcher(shared, control),
        Some(PanelModule::Notifications) => toggle_centre(shared, control),
        Some(PanelModule::Power) => toggle_power(shared, control),
        _ => {}
    }
    let wanted = asked
        .layout_menu
        .map(|anchor| (Menu::Layouts, anchor))
        .or(asked.notices.map(|anchor| (Menu::Notices, anchor)))
        .or(asked.sound_menu.map(|anchor| (Menu::Sound, anchor)))
        .or(tray_list.map(|anchor| (Menu::Tray, anchor)));
    // Another workspace is asked for: the menu was for this one.
    if let (Some((id, _)), true, None) = (open, asked.show.is_some(), wanted) {
        control.close(id);
        shared.borrow_mut().menu = None;
    }
    let Some((which, anchor)) = wanted else {
        return Flow::Continue;
    };
    if let Some((id, was)) = open {
        control.close(id);
        shared.borrow_mut().menu = None;
        // A second press on the button puts its menu away.
        if was == which {
            return Flow::Continue;
        }
    }
    let alive = Rc::new(());
    let held = Rc::clone(&alive);
    let state = Rc::clone(shared);
    let mut fonts = false;
    let themed = move |ui: &mut egui::Ui, fonts: &mut bool| {
        let context = ui.ctx().clone();
        let shared = state.borrow();
        if !*fonts {
            theme::install_fonts(&context, shared.settings.system_font);
            *fonts = true;
        }
        let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
        theme::apply(&context, &palette, &shared.settings);
        palette
    };
    let menu = match which {
        Menu::Layouts => {
            let current = if view.floating {
                String::new()
            } else {
                view.layout.clone()
            };
            // Where the display shows another workspace than the one with
            // the keyboard, the choice is for the one shown here: that one
            // gets the keyboard first.
            let first = (!view.focused)
                .then(|| view.tabs.iter().position(|tab| tab.shown))
                .flatten()
                .and_then(|index| panel::show_tab(&view, index));
            let line = Rc::clone(shared);
            control.popup(surface, anchor, panel::layout_menu_size(), move |ui, _| {
                let _held = &held;
                let palette = themed(ui, &mut fonts);
                let Some(layout) = panel::layout_menu(ui, &palette, &current) else {
                    return Flow::Continue;
                };
                let shared = line.borrow();
                if let Some(first) = &first {
                    shared.link.send(Request::Dispatch(first.clone()));
                }
                shared
                    .link
                    .send(Request::Dispatch(panel::set_layout(layout)));
                Flow::Close
            })
        }
        Menu::Notices => {
            let size = panel::notices_size(&notices);
            control.popup(surface, anchor, size, move |ui, _| {
                let _held = &held;
                let palette = themed(ui, &mut fonts);
                panel::notices_list(ui, &palette, &notices);
                Flow::Continue
            })
        }
        Menu::Sound => {
            // (Asked of the system as the menu opens.)
            let outputs = gevrek_system::sound::outputs();
            if outputs.is_empty() {
                return Flow::Continue;
            }
            let size = panel::sound_menu_size(outputs.len());
            control.popup(surface, anchor, size, move |ui, _| {
                let _held = &held;
                let palette = themed(ui, &mut fonts);
                let Some(id) = panel::sound_menu(ui, &palette, &outputs) else {
                    return Flow::Continue;
                };
                // The system tells how loud the new device is (`watch`).
                if !gevrek_system::sound::choose_output(id) {
                    log::warn!("the device to play on was not changed");
                }
                Flow::Close
            })
        }
        Menu::Tray => {
            let count = shared.borrow().tray_all.len();
            let line = Rc::clone(shared);
            // The pictures this list has made of the icons, by their keys.
            let mut pictures: HashMap<String, (u64, egui::TextureHandle)> = HashMap::new();
            control.popup(
                surface,
                anchor,
                tray_list_size(count),
                move |ui, control| {
                    let _held = &held;
                    let palette = themed(ui, &mut fonts);
                    let mut state = line.borrow_mut();
                    let state = &mut *state;
                    match tray_list_draw(
                        ui,
                        &palette,
                        &state.tray_all,
                        &state.tray_on_panel,
                        &mut pictures,
                    ) {
                        TrayAsk::Nothing => Flow::Continue,
                        // On the panel, or off it: kept, and the panels drawn again.
                        TrayAsk::Stand(name, on) => {
                            if on {
                                state.tray_on_panel.insert(name);
                            } else {
                                state.tray_on_panel.remove(&name);
                            }
                            keep_tray_choice(&state.tray_on_panel);
                            tray_for_panel(state);
                            redraw_panels(state, control);
                            Flow::Continue
                        }
                        // Pressed in the list as on the panel.
                        TrayAsk::Press(key, menu) => {
                            use gevrek_system::tray_host::Press;
                            if let Some(host) = &state.tray {
                                host.press(&key, if menu { Press::Menu } else { Press::Main });
                            }
                            Flow::Close
                        }
                    }
                },
            )
        }
    };
    shared.borrow_mut().menu = Some((menu, which, Rc::downgrade(&alive)));
    Flow::Continue
}
