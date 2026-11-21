//! gevrek-shell: what a desktop has around its windows.
//!
//! One program for all of it, started with the session. It listens to the
//! window manager: a key bound to a shell action (`{"command": "shell",
//! "action": "launcher"}`, Super+D in the standard keys) arrives here as
//! an event and opens the surface that does it.
//!
//! The panel is the same on every system ([`panel`]: what it shows, which
//! events change that, the drawing of its modules), on the surfaces of
//! the system's host (`gevrek_shellkit::host`). So are the launcher
//! ([`launcher`]: what it lists and how it is drawn), the notifications
//! with their history ([`notifications`]: the stack and its drawing), the
//! on-screen display ([`osd`]) and the power menu ([`power`]). What
//! differs is where they come from and who carries them out: on Linux
//! programs send notifications over the session's bus and the login
//! service ends the session; on Windows the notifications are gevrek's
//! own and the system's calls end the session. Linux has the clipboard,
//! screenshots, the portal and the authentication agent besides (the
//! `linux` half below); Windows has the list of shortcuts as a surface
//! ([`shortcuts`], [`windows`]).

// Each system's half uses its part of the panel's code (who keeps the
// panel's room differs between them), and the tests use all of it: what
// one half leaves unused is not dead.
#[cfg(any(target_os = "linux", windows))]
#[allow(dead_code)]
mod panel;

#[cfg(target_os = "linux")]
mod activate;
#[cfg(target_os = "linux")]
mod clipboard;
// The control centre is the same on both; what it asks is carried out by
// each system's half.
#[cfg(any(target_os = "linux", windows))]
#[cfg_attr(windows, allow(dead_code))]
mod control;
// The tutorial's card. (Drawn here; put on a surface by each system's
// half as that is written.)
#[cfg(any(target_os = "linux", windows))]
#[allow(dead_code)]
mod coach;
#[cfg(any(target_os = "linux", windows))]
#[allow(dead_code)]
mod tutoring;
mod frost;
#[cfg(target_os = "linux")]
mod inhibit;
#[cfg(any(target_os = "linux", windows))]
mod launcher;
#[cfg(target_os = "linux")]
mod media;
// (On Windows: the keyboard's own keys for what plays.)
#[cfg(windows)]
#[path = "media_keys.rs"]
mod media;
// The stack is the same on both; the service on the bus is Linux's.
#[cfg(target_os = "linux")]
mod app_shortcuts;
#[cfg(any(target_os = "linux", windows))]
#[allow(dead_code)]
mod notifications;
#[cfg(any(target_os = "linux", windows))]
#[allow(dead_code)]
mod osd;
#[cfg(target_os = "linux")]
mod polkit;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(any(target_os = "linux", windows))]
mod power;
#[cfg(target_os = "linux")]
mod record;
#[cfg(target_os = "linux")]
mod remote;
#[cfg(target_os = "linux")]
mod screencast;
#[cfg(target_os = "linux")]
mod screenshot;
#[cfg(target_os = "linux")]
mod sleep;
#[cfg(any(target_os = "linux", windows))]
mod switcher;
#[cfg(target_os = "linux")]
mod thumbs;
// What the tray shows is the same wherever it is drawn; the tray on the
// bus is Linux's.
#[cfg(any(target_os = "linux", windows))]
#[allow(dead_code)]
mod tray;
// Which icons of the tray stand on the panel: the same on both systems.
#[cfg(any(target_os = "linux", windows))]
mod tray_choice;
// Shown by the shell on Windows; on Linux the settings program's window
// is still the list, and only the tests use this.
#[cfg(any(target_os = "linux", windows))]
#[cfg_attr(not(windows), allow(dead_code))]
mod shortcuts;
// The overview of every workspace, where the shell draws it (a
// compositor shows the workspaces themselves).
// The zones of the desktop: containers that hold its icons.
#[cfg(any(target_os = "linux", windows))]
mod zones;
#[cfg(windows)]
mod overview;
#[cfg(windows)]
mod windows;

#[cfg(not(any(target_os = "linux", windows)))]
fn main() {
    eprintln!("gevrek-shell runs on Linux and on Windows.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::launcher::Launcher;
    use crate::notifications::{FromBus, Gone, Stack, ToBus};
    use crate::osd::Osd;
    use clap::Parser;
    use gevrek_link::{Event, Link, Request};
    use gevrek_model::settings::PanelModule;
    use gevrek_shellkit::layer::{Control, Shell, SurfaceId};
    use gevrek_shellkit::{Edge, Flow, Keyboard, Level, Place};
    use gevrek_ui::theme::{self, icons};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

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

    /// What reaches the shell from outside its surfaces.
    enum Message {
        /// The line to the window manager has news.
        Line,
        /// The zones of the desktop have something to do: their backend
        /// has news, or their state is due to be written.
        Zones,
        /// A program sent a notification, or took one back.
        Bus(FromBus),
        /// polkit asks for the user's word, or takes a request back.
        Agent(crate::polkit::FromAgent),
        /// The system's check of the user for a request (known by its
        /// cookie) has ended: what polkit's helper said.
        Verdict(String, crate::polkit::Verdict),
        /// The system's check asks the user something for a request:
        /// its question, whether what is typed may be seen, and where
        /// the answer goes (`None`: the user took the request back).
        AuthAsks {
            cookie: String,
            question: String,
            seen: bool,
            answer: std::sync::mpsc::Sender<Option<String>>,
        },
        /// The system's check says something for a request (a finger to
        /// be put on a reader, a mistake).
        AuthSays {
            cookie: String,
            text: String,
        },
        /// A request was refused on the screen: show the next that waits.
        NextRequest,
        /// The tray's icons changed, or a menu that was asked for is there.
        Tray(crate::tray::FromTray),
        /// The sound, the battery or the network is another than it was.
        System(crate::panel::SystemView),
        /// An application asks something through the portal.
        Portal(crate::portal::Asked),
        /// The pictures of the windows, for the switcher.
        Thumbs(Vec<crate::thumbs::Thumb>),
        /// The devices to play on, read again for the control centre.
        Mixer(gevrek_system::sound::Mixer),
        Wifi(Option<gevrek_system::wifi::Wifi>),
        /// The system is being asked for the wireless networks in reach
        /// (or no longer is).
        WifiLooking(bool),
        /// What a player has on, read for the control centre.
        Playing(Option<crate::media::Playing>),
        /// The machine's power profiles, read again after one was chosen.
        Profiles(Option<gevrek_system::power_profile::Profiles>),
        /// A recording has ended: its file and how many pictures it has,
        /// or why it broke off.
        Recorded(Result<(std::path::PathBuf, u64), String>),
        /// What was shared with an application is there no more (its
        /// window was closed, its display taken away): the session's name.
        ShareEnded(String),
        /// The machine is about to sleep: the screen is to be locked.
        LockForSleep,
        /// Bluetooth, read again for the control centre.
        Bluetooth(Option<gevrek_system::bluetooth::Bluetooth>),
        /// The wallpaper, blurred, for the frosted surfaces: the file it
        /// was read from, and the picture (none: it could not be read).
        Frost(std::path::PathBuf, Option<egui::ColorImage>),
        /// The display as it was when a frosted surface was opened,
        /// blurred (none: no picture of it could be had).
        Scene(u64, Option<egui::ColorImage>),
    }

    /// A request to share the screen that is on the screen: the user
    /// chooses what, or refuses.
    /// What a part of the frozen picture of a display is chosen for.
    #[derive(Default)]
    enum RegionFor {
        /// Kept as a screenshot (the key's).
        #[default]
        Keep,
        /// Filmed from then on.
        Film,
        /// Written to a file for the application that asked through the
        /// portal; a press without a drag takes the whole display.
        Picture(async_channel::Sender<Option<std::path::PathBuf>>),
        /// Not a part but one place: its colour, for the application
        /// that asked.
        Colour(async_channel::Sender<Option<(f64, f64, f64)>>),
    }

    struct Share {
        surface: SurfaceId,
        ask: crate::portal::ShareAsk,
        sources: Vec<crate::screencast::Source>,
        /// The previews, given to the drawing once each.
        pictures: Vec<Option<egui::TextureHandle>>,
        chosen: usize,
        /// The cards that are marked, where more than one may be shared.
        marked: std::collections::BTreeSet<usize>,
        /// The choice is to be remembered for the application.
        remember: bool,
        problem: String,
    }

    /// A request for the user's word that is on the screen.
    struct Auth {
        surface: SurfaceId,
        ask: crate::polkit::Ask,
        /// What is typed into the field.
        password: String,
        /// The system's check is at work and asks nothing at the moment.
        checking: bool,
        /// What the system's check asks now: its question, whether what
        /// is typed may be seen, and where the answer goes.
        asked: Option<(String, bool, std::sync::mpsc::Sender<Option<String>>)>,
        /// The system's check asked the user something since it was
        /// last begun (one that says no without asking is not begun
        /// again by itself: it would say no for ever).
        was_asked: bool,
        /// What the system's check said last (a finger to be put on a
        /// reader).
        said: String,
        /// What went wrong with the last try.
        problem: String,
    }

    /// What a panel can open below itself.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Menu {
        Layouts,
        Notices,
        Tray,
        /// The list of every icon of the tray, under the tray's button.
        TrayList,
        Sound,
        /// The windows a button of the taskbar stands for (by the first
        /// of them: another button's list is another menu).
        Windows(u64),
        /// What can be done with a window of the taskbar (the other
        /// button on its button), by the window.
        TaskActions(u64),
    }

    /// What the surfaces share.
    struct State {
        link: Link,
        launcher: Option<(SurfaceId, Launcher)>,
        /// The window switcher, while it is up.
        switcher: Option<(SurfaceId, crate::switcher::Switcher)>,
        /// The windows in the order they last had the keyboard.
        used: crate::switcher::Used,
        /// The panel of each display, by the display's name, and what
        /// its place was made from.
        panels: std::collections::HashMap<String, SurfaceId>,
        panel_made: Option<(gevrek_model::settings::PanelEdge, u32, u32, bool)>,
        /// The menu of a panel that is open, which one it is, and what
        /// tells that it still is (its drawing holds the other end).
        menu: Option<(SurfaceId, Menu, std::rc::Weak<()>)>,
        bar: Option<gevrek_model::settings::PanelProfile>,
        /// The tray, while the shell is it; its icons; and the icon whose
        /// menu was asked of its program and is waited for: on which
        /// display's panel, and next to which rectangle there.
        tray: Option<crate::tray::Tray>,
        /// The program that carries icons of the older kind over to the
        /// tray, while it runs (`desktop.shell.panel.older_tray_icons`).
        older_icons: Option<crate::tray::Older>,
        tray_items: Rc<Vec<crate::tray::Item>>,
        tray_wait: Option<(String, String, egui::Rect)>,
        /// The menu of a tray's icon that is open: whose it is, and the
        /// menu, which is put anew when its program fills a submenu.
        tray_menu: Option<(String, Rc<RefCell<crate::tray::Node>>)>,
        /// Every icon of the tray with its lasting name, and the names
        /// of those the person put on the panel (`tray_choice`).
        tray_all: Vec<(crate::tray::Item, String)>,
        tray_on_panel: std::collections::HashSet<String>,
        /// The sound, the battery and the network, as last looked at.
        system: Rc<crate::panel::SystemView>,
        outputs: Vec<String>,
        /// What the window manager has now.
        wm: Option<serde_json::Value>,
        /// The windows that state lists: what happens to another one is
        /// no reason to ask for the state again.
        windows: std::collections::HashSet<u64>,
        /// What the window manager has to tell, and which of it was told
        /// as a notification when it arose.
        notices: Rc<Vec<crate::panel::Notice>>,
        told: std::collections::HashSet<String>,
        /// The keys' mode when it is not the usual one.
        mode: String,
        /// The shell draws the panel (not with `--no-panel`).
        with_panel: bool,
        /// The keeper of the clipboard, where the compositor lets one be.
        clipboard: Option<crate::clipboard::Clipboard>,
        /// gevrek's portal backend, while it answers on the bus.
        portal: Option<crate::portal::Portal>,
        /// The notifications that came through the portal, by the key
        /// their application knows them by: to replace and to withdraw.
        portal_notes: std::collections::HashMap<String, u32>,
        /// The same by their number: the application, its name for the
        /// notification, and what a press on it is in its words.
        portal_ids: std::collections::HashMap<u32, (String, String, Option<String>)>,
        /// The installed applications, kept between two openings of
        /// the launcher.
        apps: gevrek_system::apps::Kept,
        /// The film of a display that is being taken.
        recording: Option<crate::record::Recording>,
        /// What is shared with applications, by the portal's session.
        casts: std::collections::HashMap<String, Vec<crate::screencast::Cast>>,
        /// The chooser of what to share, while it is shown.
        share: Option<Share>,
        /// An application asks to control the desktop, and the user has
        /// not said yet: the question's surface, and the request.
        control_ask: Option<(SurfaceId, crate::portal::ControlAsk)>,
        /// An application asks for shortcuts, and the user has not said
        /// yet: the question's surface, the request, and what is to
        /// become of each.
        shortcut_ask: Option<(
            SurfaceId,
            crate::portal::ShortcutsAsk,
            crate::app_shortcuts::Planned,
        )>,
        /// The configuration the window manager runs, as it last came,
        /// with its revision.
        running: Option<(gevrek_model::config::Config, u64)>,
        /// Keeps the screen awake while an application asks through the
        /// portal; made when the first one asks.
        awake: Option<crate::inhibit::Awake>,
        /// A wallpaper an application asked for, until the running
        /// configuration has come to be changed.
        wanted_wallpaper: Option<std::path::PathBuf>,
        /// Whether the person is to be welcomed was asked (once a run).
        welcome_asked: bool,
        /// Whether a tutorial that was stopped left something was asked.
        tidy_asked: bool,
        /// The tutorial, while it is taken: its card's surface, how far
        /// it is, and its practice windows.
        tutorial: Option<(SurfaceId, crate::tutoring::Tutoring, Vec<std::process::Child>)>,
        /// What the tutorial knows of the desktop.
        facts: gevrek_tutorial::Facts,
        /// The request for a password that is shown, and those waiting.
        auth: Option<Auth>,
        auth_waiting: Vec<crate::polkit::Ask>,
        /// The agent's connection to the system's bus, kept while it serves.
        _agent: Option<zbus::blocking::Connection>,
        /// To hand a verdict from the checking thread back to the shell.
        verdicts: Option<gevrek_shellkit::layer::Sender<Message>>,
        /// A picture of a display that a part is being chosen of: its
        /// surface, the picture, and where the drag began.
        region: Option<(
            SurfaceId,
            Rc<crate::screenshot::Picture>,
            Option<egui::Pos2>,
        )>,
        /// What the part that is being chosen is for; and the display
        /// it is chosen on.
        region_for: RegionFor,
        region_output: Option<String>,
        /// The notification centre, while it is open.
        centre: Option<(SurfaceId, crate::frost::Coming)>,
        /// The control centre, while it is up: its surface, its sliders
        /// while they are held, the devices to play on, and how far it
        /// has come (or gone).
        control_centre: Option<(
            SurfaceId,
            crate::control::Held,
            gevrek_system::sound::Mixer,
            crate::frost::Coming,
        )>,
        /// Bluetooth, as read for the control centre.
        bluetooth: Option<gevrek_system::bluetooth::Bluetooth>,
        /// The wireless networks, as read for the control centre, and
        /// the one that is being joined.
        wifi: Option<gevrek_system::wifi::Wifi>,
        wifi_joining: Option<String>,
        wifi_looking: bool,
        /// The look of the larger surfaces: frosted glass or plain.
        glass: gevrek_model::settings::GlassProfile,
        /// They slide and fade (the desktop's animations are on).
        moving: bool,
        /// The wallpaper blurred, for the glass: a number that is another
        /// for another picture, and the picture.
        frost: Option<(u64, std::sync::Arc<egui::ColorImage>)>,
        /// The wallpaper's file the glass was read from, or is being.
        frost_of: Option<std::path::PathBuf>,
        /// The display as it was when the frosted surface that is open
        /// was opened, blurred: what its glass shows, with the windows.
        frost_scene: Option<(u64, std::sync::Arc<egui::ColorImage>)>,
        /// The number of the last such picture that was asked for.
        frost_asked: u64,
        /// What a player has on, as read for the control centre.
        playing: Option<crate::media::Playing>,
        /// The machine's power profiles, as read when the control centre
        /// came up and after one was chosen there.
        power_profiles: Option<gevrek_system::power_profile::Profiles>,
        /// The night light is switched on (the configuration says so).
        night_light: bool,
        /// The power menu: its surface, the choice that waits for its
        /// second press, and what went wrong with the last one.
        power: Option<(SurfaceId, Option<crate::power::Choice>, String)>,
        /// The box that asks for a workspace's new name while it is
        /// open: its surface, the workspace's name, what is typed, and
        /// what is wrong with it.
        rename: Option<(SurfaceId, String, String, String)>,
        /// What the on-screen display shows, and until when.
        osd: Option<(SurfaceId, Osd, Instant)>,
        notes: Stack,
        notes_surface: Option<SurfaceId>,
        /// The corner notifications are shown in.
        notes_corner: gevrek_model::settings::NotificationCorner,
        /// A sound when a notification arrives.
        notes_sound: bool,
        /// Where pictures of the screen go.
        screenshots: gevrek_model::settings::ScreenshotsProfile,
        /// A change of the volume is shown (`desktop.shell.osd.volume`).
        show_volume: bool,
        /// The configuration asks for the lock before sleep.
        lock_before_sleep: std::sync::Arc<std::sync::atomic::AtomicBool>,
        /// To tell the senders of notifications what became of them.
        to_bus: Option<std::sync::mpsc::Sender<ToBus>>,
        settings: gevrek_ui::Settings,
        accent: egui::Color32,
        /// The configuration's own accent, and the wallpaper's where the
        /// desktop takes its accent from that.
        own_accent: egui::Color32,
        /// The colours the preferences' file asks for (the desktop's high
        /// contrast takes their place while it is on).
        own_preset: gevrek_ui::theme::Preset,
        wallpaper_accent: Option<egui::Color32>,
        terminal: String,
        /// The zones of the desktop, once `desktop.zones.enabled` was set.
        zones: Option<crate::zones::surfaces::Running>,
    }

    pub fn run() -> anyhow::Result<()> {
        // The message-bus library tells of every message it reads at the
        // level other things tell of what they do at: a session's log had
        // a screenful a second of it. Unless it is asked for by name.
        gevrek_logging::Logging::new("gevrek-shell", log::LevelFilter::Warn)
            .quiet("zbus", log::LevelFilter::Warn)
            .init();
        let args = Args::parse();
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
            launcher: None,
            switcher: None,
            used: Default::default(),
            panels: Default::default(),
            panel_made: None,
            menu: None,
            bar: None,
            tray: None,
            tray_items: Rc::new(Vec::new()),
            tray_wait: None,
            older_icons: None,
            tray_menu: None,
            tray_all: Vec::new(),
            tray_on_panel: crate::tray_choice::read_tray_choice(),
            system: Rc::new(Default::default()),
            outputs: Vec::new(),
            wm: None,
            windows: Default::default(),
            notices: Rc::new(Vec::new()),
            told: Default::default(),
            mode: String::new(),
            with_panel: !args.no_panel,
            clipboard: None,
            portal: None,
            portal_notes: Default::default(),
            portal_ids: Default::default(),
            wanted_wallpaper: None,
            welcome_asked: false,
            tidy_asked: false,
            tutorial: None,
            facts: gevrek_tutorial::Facts::default(),
            awake: None,
            recording: None,
            region_for: RegionFor::Keep,
            region_output: None,
            casts: std::collections::HashMap::new(),
            share: None,
            control_ask: None,
            shortcut_ask: None,
            running: None,
            apps: Default::default(),
            auth: None,
            auth_waiting: Vec::new(),
            _agent: None,
            verdicts: None,
            centre: None,
            control_centre: None,
            power_profiles: None,
            playing: None,
            bluetooth: None,
            wifi: None,
            wifi_joining: None,
            wifi_looking: false,
            glass: Default::default(),
            moving: true,
            frost: None,
            frost_of: None,
            frost_scene: None,
            frost_asked: 0,
            night_light: false,
            region: None,
            power: None,
            rename: None,
            osd: None,
            notes: Stack::default(),
            notes_surface: None,
            notes_corner: Default::default(),
            notes_sound: false,
            screenshots: Default::default(),
            show_volume: true,
            lock_before_sleep: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            to_bus: None,
            settings: gevrek_ui::Settings::load(&appearance),
            accent: egui::Color32::from_rgb(64, 224, 208),
            own_accent: egui::Color32::from_rgb(64, 224, 208),
            own_preset: gevrek_ui::Settings::load(&appearance).preset,
            wallpaper_accent: None,
            terminal: gevrek_model::examples::default_terminal().to_string(),
            zones: None,
        }));
        // The notification service; without the session's bus, or when
        // another service has the name, the shell runs without it.
        let bus_sender = shell.sender();
        match crate::notifications::serve(move |message| {
            bus_sender.send(Message::Bus(message));
        }) {
            Ok(to_bus) => {
                log::debug!("the shell is the notification service");
                state.borrow_mut().to_bus = Some(to_bus)
            }
            Err(error) => log::warn!("no notifications: {error}"),
        }
        // What is copied is kept when its program closes, and remembered.
        match crate::clipboard::start() {
            Ok(clipboard) => {
                log::debug!("the clipboard is kept");
                state.borrow_mut().clipboard = Some(clipboard)
            }
            Err(error) => log::warn!("the clipboard is not kept: {error}"),
        }
        // The portal backend: what applications are told of the look.
        {
            let mut shared = state.borrow_mut();
            let look = look_of(&shared);
            let portal_sender = shell.sender();
            match crate::portal::Portal::serve(look, move |asked| {
                portal_sender.send(Message::Portal(asked));
            }) {
                Ok(portal) => {
                    log::debug!("the shell is the portal backend");
                    shared.portal = Some(portal)
                }
                Err(error) => log::warn!("no portal backend: {error}"),
            }
        }
        // The authentication agent; without polkit, or when the session
        // has an agent already, the shell runs without being it.
        let agent_sender = shell.sender();
        match crate::polkit::serve(move |message| {
            agent_sender.send(Message::Agent(message));
        }) {
            Ok(connection) => {
                log::debug!("the shell is the authentication agent");
                state.borrow_mut()._agent = Some(connection)
            }
            Err(error) => log::warn!("not the authentication agent: {error}"),
        }
        // The tray; without the session's bus, or when another tray is
        // there, the shell runs without being it.
        if !args.no_panel {
            let tray_sender = shell.sender();
            match crate::tray::serve(move |message| {
                tray_sender.send(Message::Tray(message));
            }) {
                Ok(tray) => {
                    log::debug!("the shell is the tray");
                    state.borrow_mut().tray = Some(tray)
                }
                Err(error) => log::warn!("no tray: {error}"),
            }
        }
        // The sound, the battery and the network for the panel: looked at
        // every few seconds off the drawing thread (asking costs the start
        // of a program), and told when something is another than it was.
        if !args.no_panel {
            let system_sender = shell.sender();
            std::thread::spawn(move || {
                let mut known = None;
                loop {
                    let now = crate::panel::SystemView::read();
                    if known.as_ref() != Some(&now) {
                        known = Some(now.clone());
                        system_sender.send(Message::System(now));
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            });
        }
        // The screen is locked before the machine sleeps (where logind
        // says when that is, and the configuration wants it).
        {
            let sleep_sender = shell.sender();
            let wanted = std::sync::Arc::clone(&state.borrow().lock_before_sleep);
            match crate::sleep::watch(wanted, move || {
                sleep_sender.send(Message::LockForSleep);
            }) {
                Ok(()) => log::debug!("the shell hears when the machine is about to sleep"),
                Err(error) => log::info!("not locking before sleep: {error}"),
            }
        }
        state.borrow_mut().verdicts = Some(shell.sender());
        // This shell is the one that runs the zones of the desktop: what
        // is asked of them through the window manager comes here. Told
        // how they are at once, which is: not there, until the
        // configuration is read.
        {
            let state = state.borrow();
            state.link.host_zones();
            state.link.publish_zones(Default::default());
        }
        // `--launcher` goes through the same door as the key does.
        let mut open_first = args.launcher;
        if open_first {
            shell.sender().send(Message::Line);
        }
        let listening = Rc::clone(&state);
        let displays = Rc::clone(&state);
        shell.run_with_outputs(
            false,
            move |message, control| {
                if std::mem::take(&mut open_first) {
                    toggle_launcher(&listening, control);
                }
                match message {
                    Message::Line => take_in(&listening, control),
                    Message::Zones => zones_woken(&listening, control),
                    Message::Bus(message) => from_bus(&listening, message, control),
                    Message::Agent(message) => from_agent(&listening, message, control),
                    Message::Portal(asked) => from_portal(&listening, asked, control),
                    Message::ShareEnded(session) => {
                        let mut state = listening.borrow_mut();
                        if state.casts.remove(&session).is_some() {
                            log::debug!("a sharing of the screen ended");
                            if let Some(portal) = &state.portal {
                                portal.share_closed(&session);
                            }
                        }
                    }
                    Message::Recorded(result) => match result {
                        Ok((file, pictures)) => {
                            log::debug!(
                                "the recording is written to {} ({pictures} pictures)",
                                file.display()
                            );
                            tell(
                            &listening,
                            "Recording saved",
                            &format!(
                                "{} ({} s)",
                                file.display(),
                                pictures / u64::from(crate::record::RATE)
                            ),
                            control,
                        )
                        }
                        Err(why) => {
                            log::warn!("the recording was not saved: {why}");
                            tell(&listening, "The recording was not saved", &why, control)
                        }
                    },
                    Message::Playing(playing) => {
                        let mut state = listening.borrow_mut();
                        state.playing = playing;
                        if let Some((id, ..)) = &state.control_centre {
                            control.redraw(*id);
                        }
                    }
                    Message::Profiles(profiles) => {
                        let mut state = listening.borrow_mut();
                        state.power_profiles = profiles;
                        if let Some((id, ..)) = &state.control_centre {
                            control.redraw(*id);
                        }
                    }
                    Message::Wifi(wifi) => {
                        let mut state = listening.borrow_mut();
                        state.wifi = wifi;
                        state.wifi_joining = None;
                        if let Some((id, ..)) = &state.control_centre {
                            control.redraw(*id);
                        }
                    }
                    Message::WifiLooking(looking) => {
                        let mut state = listening.borrow_mut();
                        state.wifi_looking = looking;
                        if let Some((id, ..)) = &state.control_centre {
                            control.redraw(*id);
                        }
                    }
                    Message::Bluetooth(bluetooth) => {
                        let mut state = listening.borrow_mut();
                        state.bluetooth = bluetooth;
                        if let Some((id, ..)) = &state.control_centre {
                            control.redraw(*id);
                        }
                    }
                    Message::Frost(file, picture) => {
                        let mut state = listening.borrow_mut();
                        // (Of the wallpaper that is wanted now, not of
                        // one that was changed again meanwhile.)
                        if state.frost_of.as_ref() == Some(&file) {
                            let stamp = state.frost.as_ref().map_or(1, |(stamp, _)| stamp + 1);
                            state.frost =
                                picture.map(|picture| (stamp, std::sync::Arc::new(picture)));
                            control.redraw_all();
                        }
                    }
                    Message::Scene(asked, picture) => {
                        let mut state = listening.borrow_mut();
                        // (Of the surface that is open now, not of one
                        // that was closed and opened again meanwhile.)
                        if state.frost_asked == asked {
                            state.frost_scene = picture
                                .map(|picture| (1_000_000 + asked, std::sync::Arc::new(picture)));
                            control.redraw_all();
                        }
                    }
                    Message::Mixer(mixer) => {
                        let mut state = listening.borrow_mut();
                        if let Some((id, _, known, _)) = state.control_centre.as_mut() {
                            *known = mixer;
                            control.redraw(*id);
                        }
                    }
                    Message::Thumbs(thumbs) => {
                        let mut state = listening.borrow_mut();
                        if let Some((id, switcher)) = state.switcher.as_mut() {
                            switcher.thumbs =
                                thumbs.into_iter().map(|thumb| (thumb, None)).collect();
                            control.redraw(*id);
                        }
                    }
                    Message::Verdict(cookie, verdict) => {
                        auth_verdict(&listening, &cookie, verdict, control)
                    }
                    Message::AuthAsks {
                        cookie,
                        question,
                        seen,
                        answer,
                    } => {
                        let mut state = listening.borrow_mut();
                        match state.auth.as_mut().filter(|auth| auth.ask.cookie == cookie) {
                            Some(auth) => {
                                auth.asked = Some((question, seen, answer));
                                auth.was_asked = true;
                                auth.checking = false;
                                auth.password.clear();
                                control.redraw(auth.surface);
                            }
                            // (A request that is not on the screen any
                            // more: taken back.)
                            None => {
                                log::debug!("polkit asked about a request that is not shown any more");
                                let _ = answer.send(None);
                            }
                        }
                    }
                    Message::AuthSays { cookie, text } => {
                        let mut state = listening.borrow_mut();
                        if let Some(auth) =
                            state.auth.as_mut().filter(|auth| auth.ask.cookie == cookie)
                        {
                            auth.said = text;
                            control.redraw(auth.surface);
                        }
                    }
                    Message::NextRequest => next_auth(&listening, control),
                    Message::Tray(message) => from_tray(&listening, message, control),
                    Message::LockForSleep => {
                        log::debug!("the machine is about to sleep: the screen is locked first");
                        let command = serde_json::json!({"command": "shell", "action": "lock"});
                        listening
                            .borrow()
                            .link
                            .send(Request::Dispatch(command.to_string()));
                    }
                    Message::System(system) => {
                        let mut state = listening.borrow_mut();
                        state.system = Rc::new(system);
                        if let Some((id, ..)) = &state.control_centre {
                            control.redraw(*id);
                        }
                        for id in state.panels.values() {
                            control.redraw(*id);
                        }
                    }
                }
            },
            move |names, control| {
                log::debug!("the displays are {names:?}");
                displays.borrow_mut().outputs = names.to_vec();
                sync_panels(&displays, control);
            },
        )?;
        Ok(())
    }

    /// What applications are told of the desktop's look: dark or light as
    /// gevrek's own programs are, and the colour around the focused window.
    fn look_of(state: &State) -> crate::portal::Look {
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        let part = |value: u8| f64::from(value) / 255.0;
        crate::portal::Look {
            reduce_motion: state.settings.reduce_motion,
            contrast: state.settings.preset.is_high_contrast(),
            dark: palette.dark,
            accent: (
                part(state.accent.r()),
                part(state.accent.g()),
                part(state.accent.b()),
            ),
        }
    }

    /// What the window manager said since the last time.
    fn take_in(state: &Rc<RefCell<State>>, control: &mut Control) {
        // What was asked of the zones (a key, `gevrek zones ...`).
        let asked = state.borrow().link.poll_zones();
        for heard in asked {
            if let gevrek_link::ZonesHeard::Asked(ask) = heard {
                let state = state.borrow();
                crate::zones::surfaces::asked(&state.zones, &state.link, ask);
            }
        }
        let events = state.borrow().link.poll();
        for event in events {
            // The tutorial hears which commands were carried out: some of
            // what it teaches leaves nothing else to look at.
            if let Event::Runtime { kind, fields, .. } = &event {
                if kind == "dispatch.succeeded" {
                    let name = fields
                        .iter()
                        .find(|(name, _)| name == "payload")
                        .and_then(|(_, payload)| {
                            serde_json::from_str::<serde_json::Value>(payload).ok()
                        })
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
                // A press on a window or the bare desktop: the control
                // centre and the notifications go, as they came.
                Event::Runtime { kind, .. } if kind == "desktop.pressed" => {
                    let mut state = state.borrow_mut();
                    let now = Instant::now();
                    if let Some((id, coming)) = state.centre.as_mut() {
                        coming.leave(now);
                        control.redraw(*id);
                    }
                    if let Some((id, _, _, coming)) = state.control_centre.as_mut() {
                        coming.leave(now);
                        control.redraw(*id);
                    }
                }
                // A display is shown to others, or no longer: the zones
                // cover what is not for other eyes.
                Event::Runtime { kind, fields, .. } if kind == "screen.shared" => {
                    if let Some(zones) = &state.borrow().zones {
                        zones.screen_shared(&fields);
                    }
                }
                Event::Runtime { kind, fields, .. } if kind == "shell.command" => {
                    let action = fields
                        .iter()
                        .find(|(name, _)| name == "action")
                        .map(|(_, value)| value.as_str());
                    log::debug!("the window manager asks the shell for {}", action.unwrap_or("nothing"));
                    match action {
                        Some("launcher") => toggle_launcher(state, control),
                        Some("welcome") => {
                            crate::tutoring::open_welcome(state.borrow().link.teller())
                        }
                        Some("tutorial") => tutorial_start(state, control),
                        Some(pressed @ ("tutorial_next" | "tutorial_back" | "tutorial_leave")) => {
                            let input = match pressed {
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
                        Some("window_switcher") => toggle_switcher(state, control),
                        Some("power_menu") | Some("logout") => toggle_power(state, control),
                        Some("notification_center") => toggle_centre(state, control),
                        Some("control_center") => toggle_control(state, control),
                        Some("do_not_disturb") => {
                            let quiet = {
                                let mut state = state.borrow_mut();
                                state.notes.quiet = !state.notes.quiet;
                                state.notes.quiet
                            };
                            let said = if quiet {
                                Osd::words(icons::BELL_SLASH, "Do not disturb")
                            } else {
                                Osd::words(icons::BELL, "Notifications are shown again")
                            };
                            show_osd(state, said, control);
                        }
                        Some(
                            shot
                            @ ("screenshot_screen" | "screenshot_window" | "screenshot_region"),
                        ) => screenshot(state, shot, control),
                        Some("record_screen") => toggle_recording(state, false, control),
                        Some("record_window") => toggle_recording(state, true, control),
                        // A part chosen by dragging: chosen as a
                        // screenshot's is, and filmed from then on (the
                        // same key, or the mark on the panel, ends it).
                        Some("record_region") => {
                            if state.borrow().recording.is_some() {
                                toggle_recording(state, false, control);
                            } else {
                                state.borrow_mut().region_for = RegionFor::Film;
                                screenshot(state, "screenshot_region", control);
                                // (No display to choose on: nothing waits.)
                                if state.borrow().region.is_none() {
                                    state.borrow_mut().region_for = RegionFor::Keep;
                                }
                            }
                        }
                        // Said with a command (`gevrek notify`,
                        // `gevrek shell osd`): shown as it is said.
                        Some(said @ ("notify" | "osd")) => {
                            let field = |wanted: &str| {
                                fields
                                    .iter()
                                    .find(|(name, _)| name == wanted)
                                    .map(|(_, value)| value.as_str())
                            };
                            if said == "notify" {
                                let title = field("title").unwrap_or_default();
                                tell(state, title, field("text").unwrap_or_default(), control);
                            } else {
                                let osd =
                                    Osd::said(field("text").unwrap_or_default(), field("level"));
                                show_osd(state, osd, control);
                            }
                        }
                        // The keys of an application's own shortcut: the
                        // application is told.
                        Some("app_shortcut") => {
                            let field = |wanted: &str| {
                                fields
                                    .iter()
                                    .find(|(name, _)| name == wanted)
                                    .map_or("", |(_, value)| value.as_str())
                            };
                            if let Some(portal) = &state.borrow().portal {
                                portal.shortcut(
                                    field("app"),
                                    field("id"),
                                    field("released") == "true",
                                );
                            }
                        }
                        Some(other) => key_action(state, other, control),
                        None => {}
                    }
                }
                Event::Configuration(config, revision) => {
                    log::debug!("the configuration of revision {revision} is taken in");
                    state.borrow_mut().running = Some(((*config).clone(), revision));
                    {
                        let mut state = state.borrow_mut();
                        let tell = state.link.teller();
                        crate::tutoring::welcome_if_due(&config, &mut state.welcome_asked, tell);
                    }
                    // A wallpaper asked for through the portal: written
                    // into the configuration that just came, and applied.
                    let wanted = state.borrow_mut().wanted_wallpaper.take();
                    if let Some(file) = wanted {
                        let mut changed = config.clone();
                        changed.desktop.wallpaper.image = file.to_string_lossy().into_owned();
                        // (Shown, not one of a folder's in turn.)
                        changed.desktop.wallpaper.slideshow.folder.clear();
                        state.borrow().link.send(Request::Apply {
                            config: changed,
                            base_revision: revision,
                            client_id: "gevrek-shell".to_string(),
                        });
                    }
                    let (red, green, blue) = config.desktop.appearance.active_border_color;
                    {
                        let mut state = state.borrow_mut();
                        state.own_accent = egui::Color32::from_rgb(red, green, blue);
                        // (The wallpaper's colour, where the window
                        // manager says the desktop has it, comes first.)
                        state.accent = state.wallpaper_accent.unwrap_or(state.own_accent);
                        state.bar = Some(config.desktop.shell.panel.clone());
                        // The icons of older programs, carried over to the
                        // tray or no longer, as the configuration has it now.
                        let wanted = crate::tray::older_wanted(
                            config.desktop.shell.panel.older_tray_icons,
                            state.tray.is_some(),
                            std::env::var("DISPLAY").ok().as_deref(),
                            crate::tray::Older::found(),
                        );
                        let runs = state.older_icons.as_mut().is_some_and(|older| older.runs());
                        if wanted && !runs {
                            log::debug!("the icons of older programs are carried over to the tray");
                            state.older_icons = crate::tray::Older::start();
                        } else if !wanted && state.older_icons.is_some() {
                            log::debug!("the icons of older programs are no longer carried over");
                            state.older_icons = None;
                        }
                        // Kept still when the desktop is: the shell's own
                        // surfaces, and what applications are told.
                        state.settings.reduce_motion = config.desktop.animations.reduce_motion;
                        // High contrast for the desktop: the shell's own
                        // surfaces in their high-contrast colours, else
                        // as the preferences' file says.
                        state.settings.preset = if config.desktop.appearance.high_contrast {
                            gevrek_ui::theme::Preset::HighContrastDark
                        } else {
                            state.own_preset
                        };
                        // Notifications: where, how many, how long.
                        let notes = &config.desktop.shell.notifications;
                        state.notes.configure(notes.shown_at_once(), notes.stay());
                        state.notes_corner = notes.corner;
                        state.notes_sound = notes.sound;
                        state.night_light = config.desktop.night_light.enabled;
                        // The glass of the larger surfaces: the wallpaper
                        // blurred, read beside the shell when it is another.
                        state.glass = config.desktop.shell.glass.clone();
                        state.moving = config.desktop.animations.enabled;
                        let wallpaper = config
                            .desktop
                            .shell
                            .glass
                            .enabled
                            .then(|| config.desktop.wallpaper.of("").0)
                            .flatten();
                        if wallpaper != state.frost_of {
                            state.frost_of = wallpaper.clone();
                            match (wallpaper, state.verdicts.clone()) {
                                (Some(file), Some(sender)) => {
                                    std::thread::spawn(move || {
                                        let picture = crate::frost::frosted(&file);
                                        sender.send(Message::Frost(file, picture));
                                    });
                                }
                                _ => state.frost = None,
                            }
                        }
                        state.screenshots = config.desktop.shell.screenshots.clone();
                        state.show_volume = config.desktop.shell.osd.volume;
                        state.lock_before_sleep.store(
                            config.desktop.session.lock_before_sleep,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        // Applications follow the accent too.
                        if let Some(portal) = &state.portal {
                            portal.set(look_of(&state));
                        }
                        state.link.send(Request::State);
                    }
                    sync_panels(state, control);
                    zones_configured(state, control);
                    // Whatever is open is drawn by the new settings at
                    // once (the glass of the control centre, a colour).
                    control.redraw_all();
                }
                Event::State(wm) => {
                    let arisen = {
                        let mut state = state.borrow_mut();
                        let state = &mut *state;
                        // The zones are on the displays the state names.
                        if let Some(zones) = &state.zones {
                            zones.told(&wm);
                        }
                        state.facts = gevrek_tutorial::Facts::of(&wm);
                        crate::tutoring::tidy_after_a_stop(
                            &wm,
                            &mut state.tidy_asked,
                            state.link.teller(),
                        );
                        if let Some((surface, ..)) = &state.tutorial {
                            control.redraw(*surface);
                        }
                        state.windows = crate::panel::windows(&wm);
                        let notices = crate::panel::notices(&wm);
                        // A notice that has arisen is told as a
                        // notification too; its mark on the panel stays
                        // while it holds.
                        let arisen = crate::panel::arisen(&notices, &mut state.told);
                        if *state.notices != notices {
                            state.notices = Rc::new(notices);
                        }
                        state.mode = crate::panel::mode(&wm).to_string();
                        if let Some(window) = crate::switcher::with_keyboard(&wm) {
                            state.used.note(window);
                        }
                        // The accent the desktop has from its wallpaper.
                        let from_wallpaper = crate::panel::accent(&wm)
                            .map(|(red, green, blue)| egui::Color32::from_rgb(red, green, blue));
                        if from_wallpaper != state.wallpaper_accent {
                            state.wallpaper_accent = from_wallpaper;
                            state.accent = from_wallpaper.unwrap_or(state.own_accent);
                            if let Some(portal) = &state.portal {
                                portal.set(look_of(state));
                            }
                            control.redraw_all();
                        }
                        // (The Background portal's account of who has windows.)
                        if let Some(portal) = &state.portal {
                            portal.set_apps(crate::portal::apps_of(&wm));
                        }
                        state.wm = Some(wm);
                        for id in state.panels.values() {
                            control.redraw(*id);
                        }
                        arisen
                    };
                    for notice in arisen {
                        tell(state, &notice.title, &notice.text, control);
                    }
                }
                // What a panel shows may have changed. (Not after every
                // event of a window: one that is dragged tells of each
                // place it passes, and the panel shows none of them.)
                Event::Runtime { kind, window, .. }
                    if crate::panel::concerns(&kind, window, &state.borrow().windows) =>
                {
                    let state = state.borrow();
                    state.link.send(Request::State);
                    // (Again a little later: the window manager's state
                    // may be told before it has taken the event in.)
                    for after in [180, 400] {
                        state
                            .link
                            .send(Request::StateAfter(std::time::Duration::from_millis(after)));
                    }
                }
                // The look follows the configuration.
                Event::Changed { .. } => state.borrow().link.send(Request::Fetch),
                // What the shell itself changed is what runs, from this
                // revision on.
                // And what it made of the state is asked for at once (the
                // window manager publishes it before it answers).
                Event::Applied(revision) => {
                    let mut state = state.borrow_mut();
                    if let Some((_, known)) = state.running.as_mut() {
                        *known = revision;
                    }
                    state.link.send(Request::State);
                }
                // A change of the shell's that was not taken: said, and
                // what runs is fetched again.
                Event::Conflict(why) | Event::ApplyFailed(why) => {
                    log::warn!("a change of the shell's was not kept: {why}");
                    state.borrow().link.send(Request::Fetch);
                    tell(state, "The change was not kept", &why, control);
                }
                // A command of the shell's that the window manager could
                // not carry out: said, not swallowed.
                Event::Dispatched {
                    command,
                    error: Some(error),
                } => {
                    log::warn!("the window manager did not carry out {command}: {error}");
                    tell(state, "That did not work", &error, control);
                }
                Event::Connected(peer) => {
                    log::debug!(
                        "connected to the window manager {} at {} ({}, {})",
                        peer.version,
                        peer.address,
                        peer.backend,
                        peer.platform
                    );
                }
                Event::Disconnected(why) => {
                    log::debug!("the line to the window manager is down: {why}");
                }
                _ => {}
            }
        }
    }

    /// Bring the panels in line with the displays and the configuration:
    /// one on each display while the bar is switched on, at the place the
    /// configuration gives it.
    fn sync_panels(state: &Rc<RefCell<State>>, control: &mut Control) {
        let mut shared = state.borrow_mut();
        let wanted = shared
            .bar
            .as_ref()
            .filter(|bar| bar.enabled && shared.with_panel)
            .map(crate::panel::geometry);
        // Another place, or switched off: the surfaces that are there go.
        if shared.panel_made != wanted {
            if !shared.panels.is_empty() {
                log::debug!(
                    "the panels are closed: {}",
                    if wanted.is_some() { "they go to another place" } else { "the panel is switched off" }
                );
            }
            for (_, id) in shared.panels.drain() {
                control.close(id);
            }
            shared.panel_made = wanted;
        }
        // A display that is gone takes its panel along.
        let outputs = shared.outputs.clone();
        let gone: Vec<String> = shared
            .panels
            .keys()
            .filter(|name| !outputs.contains(name))
            .cloned()
            .collect();
        for name in gone {
            if let Some(id) = shared.panels.remove(&name) {
                log::debug!("the panel of {name} is closed: the display is gone");
                control.close(id);
            }
        }
        let Some((edge, height, margin, floating)) = wanted else {
            return;
        };
        for name in outputs {
            // (A display may have no panel of its own:
            // `desktop.shell.panel.displays`.)
            let here = shared
                .bar
                .as_ref()
                .is_some_and(|bar| bar.on(&name).is_some());
            if !here {
                if let Some(id) = shared.panels.remove(&name) {
                    log::debug!("the panel of {name} is closed: the display is to have none");
                    control.close(id);
                }
                continue;
            }
            if shared.panels.contains_key(&name) {
                continue;
            }
            let place = Place {
                name: "gevrek-panel".to_string(),
                edge: match edge {
                    gevrek_model::settings::PanelEdge::Top => Edge::Top,
                    gevrek_model::settings::PanelEdge::Bottom => Edge::Bottom,
                    gevrek_model::settings::PanelEdge::Left => Edge::Left,
                    gevrek_model::settings::PanelEdge::Right => Edge::Right,
                },
                level: Level::Above,
                // (Down a side, `height` is how wide the panel is, and
                // it is as high as the display.)
                width: if edge.is_upright() { height } else { 0 },
                height: if edge.is_upright() { 0 } else { height },
                // Windows keep clear of it, unless it floats over them.
                reserve: !floating,
                keyboard: Keyboard::Never,
                margin: margin as i32,
                output: Some(name.clone()),
            };
            log::debug!("a panel is opened on {name}");
            let drawn = Rc::clone(state);
            let output = name.clone();
            let mut fonts = false;
            // What the wheel has turned over this panel and not yet
            // been counted for.
            let mut wheel = crate::panel::Wheel::default();
            // Its hint, and the parts of its surface the pointer reaches,
            // as they were told to the compositor the last time.
            let mut hints = crate::panel::Hints::default();
            let mut reach: Vec<egui::Rect> = Vec::new();
            let id = control.open(place, move |ui, control| {
                draw_panel(
                    &drawn, &output, ui, &mut fonts, &mut wheel, &mut hints, &mut reach, control,
                )
            });
            shared.panels.insert(name, id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_panel(
        shared: &Rc<RefCell<State>>,
        output: &str,
        ui: &mut egui::Ui,
        fonts: &mut bool,
        wheel: &mut crate::panel::Wheel,
        hints: &mut crate::panel::Hints<SurfaceId>,
        reach: &mut Vec<egui::Rect>,
        control: &mut Control,
    ) -> Flow {
        let context = ui.ctx().clone();
        let state = shared.borrow();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        // (As it is on this display.)
        let Some(bar) = state.bar.as_ref().and_then(|bar| bar.on(output)) else {
            return Flow::Continue;
        };
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let view = state
            .wm
            .as_ref()
            .and_then(|wm| crate::panel::view(wm, output))
            .unwrap_or_default();
        let keyboard = state.wm.as_ref().and_then(crate::panel::keyboard_layout);
        let asked = crate::panel::draw(
            ui,
            &crate::panel::Shown {
                bar: &bar,
                view: &view,
                accent: state.accent,
                quiet: state.notes.quiet,
                notices: state.notices.len(),
                mode: &state.mode,
                recording: state.recording.is_some(),
                sharing: !state.casts.is_empty(),
                output,
                tray: &state.tray_items,
                system: &state.system,
                keyboard: keyboard.as_ref(),
            },
            Some(wheel),
        );
        // The parts of the surface that are the panel: the pointer
        // reaches those, and what is under the rest gets its clicks.
        if !asked.reach.is_empty() && asked.reach != *reach {
            *reach = asked.reach.clone();
            if let Some(panel) = state.panels.get(output) {
                control.reach(*panel, reach.clone());
            }
        }
        // The word about what the pointer rests on, on a surface of its
        // own: under the panel, or over it when the panel is at the
        // bottom (the compositor finds the side that has room).
        {
            use crate::panel::HintStep;
            // (While a menu of the panel is open nothing is said about
            // what the pointer rests on: the word would lie over the menu.)
            let menu_open = state
                .menu
                .as_ref()
                .is_some_and(|(_, _, alive)| alive.strong_count() > 0);
            let wanted = asked
                .hint
                .as_ref()
                .filter(|_| !menu_open)
                .map(|(_, words)| words.as_str());
            match hints.step(wanted, std::time::Instant::now()) {
                HintStep::Nothing => {}
                HintStep::Close(surface) => control.close(surface),
                HintStep::Wait(long) => context.request_repaint_after(long),
                HintStep::Open(before) => {
                    if let Some(surface) = before {
                        control.close(surface);
                    }
                    if let (Some((anchor, words)), Some(panel)) =
                        (asked.hint.clone(), state.panels.get(output))
                    {
                        let size = crate::panel::hint_size(ui, &words);
                        let look = bar.clone();
                        let said = words.clone();
                        let mut fonts = false;
                        let system_font = state.settings.system_font;
                        let beside = bar.edge.is_upright();
                        let surface = control.hint(*panel, anchor, size, beside, move |ui, _| {
                            if !fonts {
                                theme::install_fonts(ui.ctx(), system_font);
                                fonts = true;
                            }
                            crate::panel::draw_hint(ui, &look, &said);
                            Flow::Continue
                        });
                        hints.shown = Some((surface, words));
                    }
                }
            }
        }
        // Louder, quieter or muted: done off this thread, and what the
        // sound is afterwards comes back as the others' news does.
        if let (Some(ask), Some(sender)) = (asked.sound, state.verdicts.clone()) {
            std::thread::spawn(move || {
                use gevrek_system::sound::{self, Device};
                match ask {
                    crate::panel::SoundAsk::Step(by) => sound::change(Device::Speaker, by),
                    crate::panel::SoundAsk::Mute => sound::toggle_mute(Device::Speaker),
                };
                sender.send(Message::System(crate::panel::SystemView::read()));
            });
        }
        // The wheel over the brightness.
        if let (Some(by), Some(sender)) = (asked.brightness, state.verdicts.clone()) {
            std::thread::spawn(move || {
                gevrek_system::brightness::change(by);
                sender.send(Message::System(crate::panel::SystemView::read()));
            });
        }
        // The recording's mark: the recording ends (as its key ends it).
        let end_recording = asked.end_recording;
        let end_sharing = asked.end_sharing;
        // The keyboard's layout: on to the next one, or back.
        if let Some(next) = asked.keyboard {
            state
                .link
                .send(Request::Dispatch(crate::panel::switch_keyboard_layout(
                    next,
                )));
        }
        // A button of the taskbar: to that window.
        if let Some(id) = asked.go {
            state.link.send(Request::Dispatch(
                serde_json::json!({"command": "focus_window", "selector": {"kind": "handle", "value": id}})
                    .to_string(),
            ));
        }
        // The taskbar's own button: whose windows it has. Kept in the
        // configuration, so it is the same after gevrek is started again.
        if let Some(all) = asked.all_tasks {
            if let Some((config, revision)) = state.running.clone() {
                let mut changed = config;
                changed.desktop.shell.panel.tasks_of_all_workspaces = all;
                state.link.send(Request::Apply {
                    config: Box::new(changed),
                    base_revision: revision,
                    client_id: "gevrek-shell".to_string(),
                });
            }
        }
        // By the display and the workspace's name there, not by its place
        // among the tabs: a place that is stale by the time the window
        // manager hears of it would name another workspace, or make one.
        let show = asked
            .show
            .and_then(|index| crate::panel::show_tab(&view, index));
        // A right-click on a workspace's tab: its new name is asked for.
        let rename = asked
            .rename
            .and_then(|index| view.tabs.get(index))
            .map(|tab| (view.place.clone(), tab.name.clone(), tab.label.clone()));
        let wanted = asked
            .layout_menu
            .map(|anchor| (Menu::Layouts, anchor))
            .or(asked.notices.map(|anchor| (Menu::Notices, anchor)))
            .or(asked.sound_menu.map(|anchor| (Menu::Sound, anchor)))
            .or(asked.task_menu.as_ref().map(|(anchor, windows)| {
                (Menu::Windows(windows.first().copied().unwrap_or(0)), *anchor)
            }))
            .or(asked
                .task_actions
                .map(|(anchor, window)| (Menu::TaskActions(window), anchor)));
        // (The windows a button of the taskbar lists, as they are now.)
        let listed = asked
            .task_menu
            .as_ref()
            .map(|(_, windows)| crate::panel::tasks_of_ids(&view, windows))
            .unwrap_or_default();
        // (The window the other button was pressed on, as it is now, and
        // what each line of its menu sends.)
        let acted = asked.task_actions.and_then(|(_, window)| {
            let task = crate::panel::tasks_of_ids(&view, &[window]).into_iter().next()?;
            let commands = crate::panel::TaskAction::ALL
                .map(|action| (action, action.command(&view, &task)));
            Some((task, commands))
        });
        let pressed = asked.pressed;
        let tray = asked.tray;
        // The wheel and the middle button on an icon of the tray.
        if let Some(host) = &state.tray {
            if let Some((key, notches)) = asked.tray_wheel.clone() {
                host.ask(crate::tray::Ask::Scroll {
                    key,
                    notches,
                    sideways: false,
                });
            }
            if let Some(key) = asked.tray_middle.clone() {
                host.ask(crate::tray::Ask::Middle { key });
            }
        }
        let notices = Rc::clone(&state.notices);
        if let Some(command) = &show {
            state.link.send(Request::Dispatch(command.clone()));
        }
        let panel = state.panels.get(output).copied();
        let open = state
            .menu
            .as_ref()
            .filter(|(_, _, alive)| alive.strong_count() > 0)
            .map(|(id, which, _)| (*id, *which));
        drop(state);
        if end_recording {
            toggle_recording(shared, false, control);
        }
        if end_sharing {
            end_sharing_all(shared);
        }
        if let Some((place, name, label)) = rename {
            ask_workspace_name(shared, control, output, place, &name, &label);
        }
        match pressed {
            Some(PanelModule::Launcher) => toggle_launcher(shared, control),
            Some(PanelModule::Notifications) => toggle_centre(shared, control),
            Some(PanelModule::Power) => toggle_power(shared, control),
            Some(PanelModule::Network | PanelModule::ControlCentre) => {
                toggle_control(shared, control)
            }
            _ => {}
        }
        // Another workspace is asked for: the menu was for this one.
        if let (Some((id, _)), true, None) = (open, show.is_some(), wanted) {
            control.close(id);
            shared.borrow_mut().menu = None;
        }
        if let Some((key, menu, anchor)) = tray {
            // The tray's own button: the list of every icon, under it; a
            // second press puts the list away.
            if key == crate::tray_choice::TRAY_MORE {
                if let Some((id, was)) = open {
                    control.close(id);
                    shared.borrow_mut().menu = None;
                    if was == Menu::TrayList {
                        return Flow::Continue;
                    }
                }
                if let Some(panel) = panel {
                    open_tray_list(shared, panel, output, anchor, control);
                }
                return Flow::Continue;
            }
            // A menu that is open goes, whichever it is; a second press
            // on an icon puts its menu away.
            if let Some((id, was)) = open {
                control.close(id);
                shared.borrow_mut().menu = None;
                if menu && was == Menu::Tray {
                    return Flow::Continue;
                }
            }
            let mut state = shared.borrow_mut();
            let ask = if menu {
                // The menu is asked of the program; it opens when the
                // answer is there (`from_tray`).
                state.tray_wait = Some((output.to_string(), key.clone(), anchor));
                crate::tray::Ask::Menu { key }
            } else {
                crate::tray::Ask::Activate { key }
            };
            if let Some(tray) = &state.tray {
                tray.ask(ask);
            }
            return Flow::Continue;
        }
        let (Some((which, anchor)), Some(panel)) = (wanted, panel) else {
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
        let menu = match which {
            Menu::Layouts => {
                // Where the display shows another workspace than the one
                // with the keyboard, the choice is for the one shown here.
                // That one gets the keyboard first, asked for by the
                // display and its place there (see the tabs above).
                let first = (!view.focused)
                    .then(|| view.tabs.iter().position(|tab| tab.shown))
                    .flatten()
                    .and_then(|index| crate::panel::show_tab(&view, index));
                open_layout_menu(
                    shared,
                    panel,
                    anchor,
                    view.layout.clone(),
                    first,
                    Rc::clone(&alive),
                    control,
                )
            }
            Menu::Notices => {
                open_notices(shared, panel, anchor, notices, Rc::clone(&alive), control)
            }
            Menu::Sound => open_sound_menu(shared, panel, anchor, Rc::clone(&alive), control),
            Menu::Windows(_) => {
                if listed.is_empty() {
                    return Flow::Continue;
                }
                open_task_menu(shared, panel, anchor, listed, Rc::clone(&alive), control)
            }
            Menu::TaskActions(_) => {
                let Some((task, commands)) = acted else {
                    return Flow::Continue;
                };
                open_task_actions(shared, panel, anchor, task, commands, Rc::clone(&alive), control)
            }
            // Opened when its program has answered, not from here.
            Menu::Tray | Menu::TrayList => return Flow::Continue,
        };
        shared.borrow_mut().menu = Some((menu, which, Rc::downgrade(&alive)));
        Flow::Continue
    }

    /// What the panel shows of the tray (see `tray_choice::for_panel`).
    fn tray_for_panel(state: &mut State) {
        // (The colour of the panel's words, as light as they are.)
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        let colour = (palette.text.r(), palette.text.g(), palette.text.b());
        state.tray_items = Rc::new(crate::tray_choice::for_panel(
            &state.tray_all,
            &state.tray_on_panel,
            colour,
        ));
    }

    /// The list of every icon of the tray, under the tray's own button:
    /// a switch for whether an icon stands on the panel, and a press on
    /// an icon's line is a press on the icon.
    fn open_tray_list(
        shared: &Rc<RefCell<State>>,
        panel: SurfaceId,
        output: &str,
        anchor: egui::Rect,
        control: &mut Control,
    ) {
        use crate::tray_choice::TrayAsk;
        let size = crate::tray_choice::tray_list_size(shared.borrow().tray_all.len());
        let alive = Rc::new(());
        let held = Rc::clone(&alive);
        let state = Rc::clone(shared);
        let output = output.to_string();
        let mut fonts = false;
        let mut pictures = std::collections::HashMap::new();
        let menu = control.popup(panel, anchor, size, move |ui, control| {
            let _held = &held;
            let context = ui.ctx().clone();
            let mut shared = state.borrow_mut();
            let shared = &mut *shared;
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            match crate::tray_choice::tray_list_draw(
                ui,
                &palette,
                &shared.tray_all,
                &shared.tray_on_panel,
                &mut pictures,
            ) {
                TrayAsk::Nothing => Flow::Continue,
                TrayAsk::Stand(name, on) => {
                    if on {
                        shared.tray_on_panel.insert(name);
                    } else {
                        shared.tray_on_panel.remove(&name);
                    }
                    crate::tray_choice::keep_tray_choice(&shared.tray_on_panel);
                    tray_for_panel(shared);
                    for id in shared.panels.values() {
                        control.redraw(*id);
                    }
                    context.request_repaint();
                    Flow::Continue
                }
                TrayAsk::Press(key, menu) => {
                    let ask = if menu {
                        // Its menu opens where the list was.
                        shared.tray_wait = Some((output.clone(), key.clone(), anchor));
                        crate::tray::Ask::Menu { key }
                    } else {
                        crate::tray::Ask::Activate { key }
                    };
                    if let Some(tray) = &shared.tray {
                        tray.ask(ask);
                    }
                    shared.menu = None;
                    Flow::Close
                }
            }
        });
        shared.borrow_mut().menu = Some((menu, Menu::TrayList, Rc::downgrade(&alive)));
    }

    /// The tray's icons changed, or the menu that was asked for is there:
    /// it opens under its icon, and a press on a line is sent to the
    /// icon's program.
    fn from_tray(
        shared: &Rc<RefCell<State>>,
        message: crate::tray::FromTray,
        control: &mut Control,
    ) {
        let (key, menu) = match message {
            crate::tray::FromTray::Items(items) => {
                let mut state = shared.borrow_mut();
                // A menu is not waited for from a program that is gone.
                let waited = state.tray_wait.take();
                state.tray_wait =
                    waited.filter(|(_, key, _)| items.iter().any(|item| item.key == *key));
                // Every icon is kept; on the panel stand those the person
                // put there, and the tray's own button for the list.
                state.tray_all = items
                    .into_iter()
                    .map(|item| {
                        let name = item.name.clone();
                        (item, name)
                    })
                    .collect();
                tray_for_panel(&mut state);
                for id in state.panels.values() {
                    control.redraw(*id);
                }
                // (The list of the icons, where it is open, shows them.)
                if let Some((id, Menu::TrayList, _)) = &state.menu {
                    control.redraw(*id);
                }
                return;
            }
            crate::tray::FromTray::Menu { key, menu } => (key, menu),
        };
        // The menu that is open, read again (a submenu was opened and
        // its program may have filled it): shown as it is now.
        {
            let state = shared.borrow();
            if let (Some((id, Menu::Tray, _)), Some((open, kept))) = (&state.menu, &state.tray_menu)
            {
                if *open == key && state.tray_wait.is_none() {
                    *kept.borrow_mut() = menu;
                    control.redraw(*id);
                    return;
                }
            }
        }
        let waited = {
            let mut state = shared.borrow_mut();
            match state.tray_wait.take() {
                Some(waited) if waited.1 == key => waited,
                // Another one was asked for since.
                other => {
                    state.tray_wait = other;
                    return;
                }
            }
        };
        let (output, _, anchor) = waited;
        let Some(panel) = shared.borrow().panels.get(&output).copied() else {
            return;
        };
        if crate::tray::lines(&menu).is_empty() {
            return;
        }
        // Another menu that was opened meanwhile gives way.
        if let Some((id, _, _)) = shared.borrow_mut().menu.take() {
            control.close(id);
        }
        let size = crate::panel::tray_menu_size(&menu);
        let menu = Rc::new(RefCell::new(menu));
        let kept = Rc::clone(&menu);
        let mut path: Vec<i32> = Vec::new();
        let opened = key.clone();
        let alive = Rc::new(());
        let held = Rc::clone(&alive);
        let state = Rc::clone(shared);
        let mut fonts = false;
        let surface = control.popup(panel, anchor, size, move |ui, _| {
            let _held = &held;
            let context = ui.ctx().clone();
            let shared = state.borrow();
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            match crate::panel::tray_menu(ui, &palette, &menu.borrow(), &mut path) {
                None => Flow::Continue,
                // A submenu: its program is told, and the menu stays.
                Some(crate::panel::TrayMenuAsk::Open(id)) => {
                    if let Some(tray) = &shared.tray {
                        tray.ask(crate::tray::Ask::Open {
                            key: key.clone(),
                            id,
                        });
                    }
                    Flow::Continue
                }
                Some(crate::panel::TrayMenuAsk::Press(id)) => {
                    if let Some(tray) = &shared.tray {
                        tray.ask(crate::tray::Ask::Press {
                            key: key.clone(),
                            id,
                        });
                    }
                    Flow::Close
                }
            }
        });
        let mut state = shared.borrow_mut();
        state.menu = Some((surface, Menu::Tray, Rc::downgrade(&alive)));
        state.tray_menu = Some((opened, kept));
    }

    /// What the window manager has to tell, in full, under its mark on a
    /// panel: there until a press elsewhere or on the mark again.
    fn open_notices(
        shared: &Rc<RefCell<State>>,
        panel: SurfaceId,
        anchor: egui::Rect,
        notices: Rc<Vec<crate::panel::Notice>>,
        alive: Rc<()>,
        control: &mut Control,
    ) -> SurfaceId {
        let size = crate::panel::notices_size(&notices);
        let state = Rc::clone(shared);
        let mut fonts = false;
        control.popup(panel, anchor, size, move |ui, _| {
            let _held = &alive;
            let context = ui.ctx().clone();
            let shared = state.borrow();
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            crate::panel::notices_list(ui, &palette, &notices);
            Flow::Continue
        })
    }

    /// The menu under a panel's sound: the devices to play on; a press
    /// on one makes it the one in use and puts the menu away.
    fn open_sound_menu(
        shared: &Rc<RefCell<State>>,
        panel: SurfaceId,
        anchor: egui::Rect,
        alive: Rc<()>,
        control: &mut Control,
    ) -> SurfaceId {
        // (Asked of the sound server as the menu opens.)
        let outputs = gevrek_system::sound::outputs();
        let size = crate::panel::sound_menu_size(outputs.len());
        let state = Rc::clone(shared);
        let mut fonts = false;
        control.popup(panel, anchor, size, move |ui, _| {
            let _held = &alive;
            let context = ui.ctx().clone();
            let shared = state.borrow();
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            let Some(id) = crate::panel::sound_menu(ui, &palette, &outputs) else {
                return Flow::Continue;
            };
            if let Some(sender) = shared.verdicts.clone() {
                std::thread::spawn(move || {
                    gevrek_system::sound::choose_output(id);
                    sender.send(Message::System(crate::panel::SystemView::read()));
                });
            }
            Flow::Close
        })
    }

    /// The list under a button of the taskbar that stands for several
    /// windows: a press on one goes to it and puts the list away.
    fn open_task_menu(
        shared: &Rc<RefCell<State>>,
        panel: SurfaceId,
        anchor: egui::Rect,
        tasks: Vec<crate::panel::Task>,
        alive: Rc<()>,
        control: &mut Control,
    ) -> SurfaceId {
        let size = crate::panel::task_menu_size(tasks.len());
        let state = Rc::clone(shared);
        let mut fonts = false;
        control.popup(panel, anchor, size, move |ui, _| {
            let _held = &alive;
            let context = ui.ctx().clone();
            let shared = state.borrow();
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            let Some(window) = crate::panel::task_menu(ui, &palette, &tasks) else {
                return Flow::Continue;
            };
            shared
                .link
                .send(Request::Dispatch(crate::panel::go_to_window(window)));
            Flow::Close
        })
    }

    /// The menu of a window of the taskbar (the other button on its
    /// button): a press on a line sends its command and puts the menu
    /// away.
    fn open_task_actions(
        shared: &Rc<RefCell<State>>,
        panel: SurfaceId,
        anchor: egui::Rect,
        task: crate::panel::Task,
        commands: [(crate::panel::TaskAction, Option<String>); 2],
        alive: Rc<()>,
        control: &mut Control,
    ) -> SurfaceId {
        let size = crate::panel::task_actions_size();
        let state = Rc::clone(shared);
        let mut fonts = false;
        control.popup(panel, anchor, size, move |ui, _| {
            let _held = &alive;
            let context = ui.ctx().clone();
            let shared = state.borrow();
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            let Some(action) = crate::panel::task_actions(ui, &palette, &task) else {
                return Flow::Continue;
            };
            let command = commands
                .iter()
                .find(|(each, _)| *each == action)
                .and_then(|(_, command)| command.clone());
            if let Some(command) = command {
                shared.link.send(Request::Dispatch(command));
            }
            Flow::Close
        })
    }

    /// The menu under a panel's layout: every layout, the one in use
    /// marked; a press on one sets it and puts the menu away.
    fn open_layout_menu(
        shared: &Rc<RefCell<State>>,
        panel: SurfaceId,
        anchor: egui::Rect,
        current: String,
        first: Option<String>,
        alive: Rc<()>,
        control: &mut Control,
    ) -> SurfaceId {
        let size = crate::panel::layout_menu_size();
        let state = Rc::clone(shared);
        let mut fonts = false;
        control.popup(panel, anchor, size, move |ui, _| {
            let _held = &alive;
            let context = ui.ctx().clone();
            let shared = state.borrow();
            if !fonts {
                theme::install_fonts(&context, shared.settings.system_font);
                fonts = true;
            }
            let palette = shared.settings.resolve(egui::Theme::Dark, shared.accent);
            theme::apply(&context, &palette, &shared.settings);
            // (Escape puts a menu away, as a press elsewhere does.)
            if crate::panel::menu_escape(&context) {
                return Flow::Close;
            }
            let Some(layout) = crate::panel::layout_menu(ui, &palette, &current) else {
                return Flow::Continue;
            };
            if let Some(first) = &first {
                shared.link.send(Request::Dispatch(first.clone()));
            }
            shared
                .link
                .send(Request::Dispatch(crate::panel::set_layout(layout)));
            Flow::Close
        })
    }

    /// Everything that is shared ends (the mark on the panel was
    /// pressed): the streams stop, and the portal is told of each.
    fn end_sharing_all(state: &Rc<RefCell<State>>) {
        let mut state = state.borrow_mut();
        let sessions: Vec<String> = state.casts.keys().cloned().collect();
        state.casts.clear();
        if let Some(portal) = &state.portal {
            for session in sessions {
                portal.share_closed(&session);
            }
        }
    }

    /// The chooser of what to share: so many cards beside one another,
    /// each this wide, with a picture this high above its name.
    const SHARE_COLUMNS: usize = 3;
    const SHARE_CARD: (f32, f32) = (236.0, 133.0);
    const SHARE_NAME: f32 = 42.0;

    /// The size of the chooser for this many things to choose from: as
    /// high as its rows of cards, up to three of them (more are scrolled).
    fn share_size(count: usize) -> (u32, u32) {
        let rows = count.div_ceil(SHARE_COLUMNS).clamp(1, 3) as f32;
        let width = 36.0 + SHARE_COLUMNS as f32 * (SHARE_CARD.0 + 10.0) + 6.0;
        let height = 150.0 + rows * (SHARE_CARD.1 + SHARE_NAME + 10.0);
        (width as u32, height as u32)
    }

    /// Where the choice goes from `chosen` by an arrow key among `count`
    /// cards: `by` one to the side, or a row up or down.
    fn share_step(chosen: usize, count: usize, by: isize) -> usize {
        let to = chosen as isize + by;
        if to < 0 || to >= count as isize {
            chosen
        } else {
            to as usize
        }
    }

    /// The configuration asks for displays to be shared in buffers of
    /// the graphics card.
    fn share_in_card(state: &State) -> bool {
        state
            .running
            .as_ref()
            .is_some_and(|(config, _)| config.desktop.platform.wayland.share_card_buffers)
    }

    /// Begin to share these things for a session: a stream for each, or
    /// none at all when one of them cannot be had.
    fn begin_share(
        sources: &[&crate::screencast::Source],
        pointer: bool,
        beside: bool,
        session: &str,
        in_card: bool,
        sender: Option<gevrek_shellkit::layer::Sender<Message>>,
    ) -> Result<(Vec<crate::portal::Shared>, Vec<crate::screencast::Cast>), String> {
        let mut told = Vec::new();
        let mut casts = Vec::new();
        for source in sources {
            let (ended, sender) = (session.to_string(), sender.clone());
            let cast =
                crate::screencast::start(&source.what, pointer, beside, in_card, move || {
                    if let Some(sender) = sender {
                        sender.send(Message::ShareEnded(ended));
                    }
                })
                .map_err(|error| format!("{error:#}"))?;
            told.push(crate::portal::Shared {
                node: cast.node,
                size: cast.size,
                display: matches!(source.what, crate::screencast::What::Output(_)),
                output: match &source.what {
                    crate::screencast::What::Output(name) => Some(name.clone()),
                    crate::screencast::What::Window(_) => None,
                },
            });
            casts.push(cast);
        }
        Ok((told, casts))
    }

    /// The displays a choice of these things would be remembered as:
    /// only displays are (a window is another one the next time).
    fn share_remembered(sources: &[&crate::screencast::Source]) -> Option<Vec<String>> {
        sources
            .iter()
            .map(|source| match &source.what {
                crate::screencast::What::Output(name) => Some(name.clone()),
                crate::screencast::What::Window(_) => None,
            })
            .collect()
    }

    /// An application asks for the screen: show what can be shared.
    fn open_share(state: &Rc<RefCell<State>>, ask: crate::portal::ShareAsk, control: &mut Control) {
        use crate::screencast::What;
        // One request at a time; a second one while the user chooses is
        // refused.
        if state.borrow().share.is_some() {
            log::debug!("a request to share the screen is refused: another is being chosen");
            let _ = ask.answer.send_blocking(None);
            return;
        }
        // (With a picture of each, taken now: the chooser is in none.)
        let sources: Vec<crate::screencast::Source> = match crate::screencast::sources() {
            Ok(all) => all
                .into_iter()
                .filter(|source| match source.what {
                    What::Output(_) => ask.displays,
                    What::Window(_) => ask.windows,
                })
                // (The shell's own surfaces are no windows; a window
                // without a title has nothing to be known by.)
                .filter(|source| !source.label.is_empty())
                .collect(),
            Err(error) => {
                log::warn!("nothing can be shared: {error:#}");
                Vec::new()
            }
        };
        if sources.is_empty() {
            log::debug!("a request to share the screen is refused: there is nothing to share");
            let _ = ask.answer.send_blocking(None);
            tell(
                state,
                "No screen sharing",
                "There is nothing that could be shared.",
                control,
            );
            return;
        }
        // A choice that was remembered for this application: the same
        // displays again, unasked, while every one of them is there.
        let again: Option<Vec<&crate::screencast::Source>> = (!ask.remembered.is_empty())
            .then(|| {
                ask.remembered
                    .iter()
                    .map(|name| {
                        sources
                            .iter()
                            .find(|source| source.what == What::Output(name.clone()))
                    })
                    .collect()
            })
            .flatten();
        if let Some(again) = again {
            let sender = state.borrow().verdicts.clone();
            let in_card = share_in_card(&state.borrow());
            if let Ok((streams, casts)) = begin_share(
                &again,
                ask.pointer,
                ask.beside,
                &ask.session,
                in_card,
                sender,
            ) {
                log::debug!("{} is given {} streams of the screen", ask.app, streams.len());
                let _ = ask.answer.send_blocking(Some(crate::portal::Answer {
                    streams,
                    remember: Some(ask.remembered.clone()),
                }));
                state.borrow_mut().casts.insert(ask.session.clone(), casts);
                // (The mark on the panel says that it is shared.)
                let state = state.borrow();
                for id in state.panels.values() {
                    control.redraw(*id);
                }
                return;
            }
        }
        let (width, height) = share_size(sources.len());
        let place = Place {
            name: "gevrek-share".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width,
            height,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: None,
        };
        let drawn = Rc::clone(state);
        let mut fonts = false;
        let surface = control.open(place, move |ui, _| draw_share(&drawn, ui, &mut fonts));
        state.borrow_mut().share = Some(Share {
            surface,
            ask,
            marked: std::collections::BTreeSet::new(),
            remember: false,
            pictures: vec![None; sources.len()],
            sources,
            chosen: 0,
            problem: String::new(),
        });
    }

    fn draw_share(shared: &Rc<RefCell<State>>, ui: &mut egui::Ui, fonts: &mut bool) -> Flow {
        use crate::screencast::What;
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let sender = state.verdicts.clone();
        let in_card = share_in_card(&state);
        let Some(share) = state.share.as_mut() else {
            return Flow::Close;
        };
        let pressed =
            |key| context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        let mut refuse = pressed(egui::Key::Escape);
        let mut accept = pressed(egui::Key::Enter);
        let count = share.sources.len();
        let mut moved = false;
        if share.ask.several && pressed(egui::Key::Space) && !share.marked.remove(&share.chosen) {
            share.marked.insert(share.chosen);
        }
        // (The key for what the box at the bottom says.)
        if share.ask.may_remember && pressed(egui::Key::R) {
            share.remember = !share.remember;
        }
        for (key, by) in [
            (egui::Key::ArrowRight, 1),
            (egui::Key::ArrowLeft, -1),
            (egui::Key::ArrowDown, SHARE_COLUMNS as isize),
            (egui::Key::ArrowUp, -(SHARE_COLUMNS as isize)),
        ] {
            if pressed(key) {
                share.chosen = share_step(share.chosen, count, by);
                moved = true;
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                    .corner_radius(14)
                    .inner_margin(18),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(icons::SCREENCAST)
                            .size(22.0)
                            .color(palette.accent_fg()),
                    );
                    ui.label(
                        egui::RichText::new("Share the screen")
                            .size(17.0)
                            .strong()
                            .color(palette.text),
                    );
                });
                ui.add_space(4.0);
                let who = if share.ask.app.is_empty() {
                    "An application".to_string()
                } else {
                    share.ask.app.clone()
                };
                ui.label(
                    egui::RichText::new(format!("{who} asks to see what is on the screen."))
                        .color(palette.text),
                );
                ui.add_space(8.0);
                let room = (ui.available_height() - 46.0).max(SHARE_CARD.1);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                        ui.horizontal_wrapped(|ui| {
                            for index in 0..count {
                                let source = &share.sources[index];
                                let (card, response) = ui.allocate_exact_size(
                                    egui::vec2(SHARE_CARD.0, SHARE_CARD.1 + SHARE_NAME),
                                    egui::Sense::click(),
                                );
                                let chosen = share.chosen == index;
                                let painter = ui.painter();
                                painter.rect_filled(
                                    card,
                                    10,
                                    if chosen {
                                        palette.accent.gamma_multiply(0.22)
                                    } else if response.hovered() {
                                        palette.text.gamma_multiply(0.08)
                                    } else {
                                        palette.text.gamma_multiply(0.04)
                                    },
                                );
                                painter.rect_stroke(
                                    card,
                                    10,
                                    egui::Stroke::new(
                                        if chosen { 2.0 } else { 1.0 },
                                        if chosen {
                                            palette.accent
                                        } else {
                                            palette.border
                                        },
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                                // The picture, whole, in the card's upper part.
                                let frame = egui::Rect::from_min_size(
                                    card.min + egui::vec2(8.0, 8.0),
                                    egui::vec2(SHARE_CARD.0 - 16.0, SHARE_CARD.1 - 12.0),
                                );
                                let icon = match source.what {
                                    What::Output(_) => icons::MONITOR,
                                    What::Window(_) => icons::APP_WINDOW,
                                };
                                match &source.preview {
                                    Some(preview) if preview.width > 0 && preview.height > 0 => {
                                        let texture =
                                            share.pictures[index].get_or_insert_with(|| {
                                                let image =
                                                    egui::ColorImage::from_rgba_unmultiplied(
                                                        [
                                                            preview.width as usize,
                                                            preview.height as usize,
                                                        ],
                                                        &preview.rgba,
                                                    );
                                                context.load_texture(
                                                    format!("share-{index}"),
                                                    image,
                                                    egui::TextureOptions::LINEAR,
                                                )
                                            });
                                        let by = (frame.width() / preview.width as f32)
                                            .min(frame.height() / preview.height as f32);
                                        let place = egui::Rect::from_center_size(
                                            frame.center(),
                                            egui::vec2(
                                                preview.width as f32 * by,
                                                preview.height as f32 * by,
                                            ),
                                        );
                                        let whole = egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        );
                                        painter.image(
                                            texture.id(),
                                            place,
                                            whole,
                                            egui::Color32::WHITE,
                                        );
                                    }
                                    // No picture could be taken: its sign.
                                    _ => {
                                        painter.text(
                                            frame.center(),
                                            egui::Align2::CENTER_CENTER,
                                            icon,
                                            egui::FontId::proportional(40.0),
                                            palette.muted,
                                        );
                                    }
                                }
                                // Its name, and a window's program under it.
                                let words = egui::Rect::from_min_max(
                                    egui::pos2(card.left() + 10.0, frame.bottom() + 2.0),
                                    egui::pos2(card.right() - 10.0, card.bottom() - 4.0),
                                );
                                let cut = painter.with_clip_rect(words);
                                cut.text(
                                    words.left_top() + egui::vec2(0.0, 2.0),
                                    egui::Align2::LEFT_TOP,
                                    format!("{icon}  {}", source.label),
                                    egui::FontId::proportional(13.5),
                                    palette.text,
                                );
                                if !source.detail.is_empty() {
                                    cut.text(
                                        words.left_top() + egui::vec2(0.0, 21.0),
                                        egui::Align2::LEFT_TOP,
                                        &source.detail,
                                        egui::FontId::proportional(11.5),
                                        palette.muted,
                                    );
                                }
                                if chosen && moved {
                                    ui.scroll_to_rect(card, None);
                                }
                                // Marked to be shared, where several may be.
                                if share.marked.contains(&index) {
                                    let badge = card.right_top() + egui::vec2(-18.0, 18.0);
                                    painter.circle_filled(badge, 11.0, palette.accent);
                                    painter.text(
                                        badge,
                                        egui::Align2::CENTER_CENTER,
                                        icons::CHECK,
                                        egui::FontId::proportional(13.0),
                                        theme::readable(egui::Color32::BLACK, palette.accent, 4.5),
                                    );
                                }
                                if response.clicked() {
                                    share.chosen = index;
                                    if share.ask.several && !share.marked.remove(&index) {
                                        share.marked.insert(index);
                                    }
                                }
                                if response.double_clicked() {
                                    accept = true;
                                }
                            }
                        });
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if !share.problem.is_empty() {
                        ui.label(egui::RichText::new(&share.problem).color(palette.error));
                    } else if share.ask.may_remember {
                        // (Only displays are remembered: said where a
                        // window is among what is chosen.)
                        ui.checkbox(&mut share.remember, "Do not ask this application again (R)")
                            .on_hover_text("Holds for displays; a window is asked for every time.");
                    } else if share.ask.several {
                        ui.label(
                            egui::RichText::new("Several can be chosen: Space marks one.")
                                .color(palette.muted),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.button("Share").clicked() {
                            accept = true;
                        }
                        if ui.button("Refuse").clicked() {
                            refuse = true;
                        }
                    });
                });
            });
        if refuse {
            if let Some(share) = state.share.take() {
                log::debug!("the screen is not shared with {}: refused", share.ask.app);
                let _ = share.ask.answer.send_blocking(None);
            }
            return Flow::Close;
        }
        if accept {
            // What is marked, or the one that is chosen.
            let which: Vec<usize> = if share.marked.is_empty() {
                vec![share.chosen]
            } else {
                share.marked.iter().copied().collect()
            };
            let chosen: Vec<&crate::screencast::Source> = which
                .iter()
                .filter_map(|index| share.sources.get(*index))
                .collect();
            if chosen.is_empty() {
                return Flow::Continue;
            }
            let session = share.ask.session.clone();
            match begin_share(
                &chosen,
                share.ask.pointer,
                share.ask.beside,
                &session,
                in_card,
                sender,
            ) {
                Ok((streams, casts)) => {
                    let remember = share.remember.then(|| share_remembered(&chosen)).flatten();
                    if let Some(share) = state.share.take() {
                        let _ = share
                            .ask
                            .answer
                            .send_blocking(Some(crate::portal::Answer { streams, remember }));
                    }
                    state.casts.insert(session, casts);
                    return Flow::Close;
                }
                Err(why) => {
                    log::warn!("not shared: {why}");
                    share.problem = why;
                }
            }
        }
        Flow::Continue
    }

    /// What an application that asks to control the desktop would do,
    /// in words, by the devices it asks for.
    fn control_words(devices: u32) -> &'static str {
        use crate::portal::{KEYBOARD, POINTER};
        match (devices & POINTER != 0, devices & KEYBOARD != 0) {
            (true, false) => "move the pointer and press its buttons",
            (false, true) => "type on the keyboard",
            _ => "move the pointer and type on the keyboard",
        }
    }

    /// An application asks (through the portal) to control the desktop:
    /// the user is asked, every time. One question at a time; a second
    /// one meanwhile is refused.
    fn open_control(
        state: &Rc<RefCell<State>>,
        ask: crate::portal::ControlAsk,
        control: &mut Control,
    ) {
        if state.borrow().control_ask.is_some() {
            log::debug!("a request to control the desktop is refused: another is asked");
            let _ = ask.answer.send_blocking(false);
            return;
        }
        log::debug!("the question whether the desktop may be controlled is opened");
        let place = Place {
            name: "gevrek-control".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width: 460,
            height: 190,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: None,
        };
        let drawn = Rc::clone(state);
        let mut fonts = false;
        let surface = control.open(place, move |ui, _| draw_control(&drawn, ui, &mut fonts));
        state.borrow_mut().control_ask = Some((surface, ask));
    }

    /// The question: Escape (or its button) refuses; only the button
    /// that says so allows, no key does.
    fn draw_control(shared: &Rc<RefCell<State>>, ui: &mut egui::Ui, fonts: &mut bool) -> Flow {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let Some((_, ask)) = state.control_ask.as_ref() else {
            return Flow::Close;
        };
        let mut said: Option<bool> = context
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            .then_some(false);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                    .corner_radius(14)
                    .inner_margin(18),
            )
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Control of the desktop")
                        .size(17.0)
                        .strong()
                        .color(palette.text),
                );
                ui.add_space(6.0);
                let who = if ask.app.is_empty() {
                    "An application".to_string()
                } else {
                    ask.app.clone()
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{who} asks to {}, as if it sat at this machine.",
                        control_words(ask.devices)
                    ))
                    .color(palette.text),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("It can do so until it ends, or is closed.")
                        .color(palette.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                    if ui.button("Allow").clicked() {
                        said = Some(true);
                    }
                    if ui.button("Refuse").clicked() {
                        said = Some(false);
                    }
                });
            });
        let Some(said) = said else {
            return Flow::Continue;
        };
        if let Some((_, ask)) = state.control_ask.take() {
            log::debug!(
                "{} may {}control the desktop",
                ask.app,
                if said { "" } else { "not " }
            );
            let _ = ask.answer.send_blocking(said);
        }
        Flow::Close
    }

    /// An application asks (through the portal) for keys of its own.
    /// Those it has already are answered at once; for new ones the user
    /// is asked, and allowed they are added to the running
    /// configuration as bindings.
    fn open_shortcuts(
        state: &Rc<RefCell<State>>,
        ask: crate::portal::ShortcutsAsk,
        control: &mut Control,
    ) {
        use crate::app_shortcuts::Plan;
        let planned = state.borrow().running.as_ref().map(|(config, _)| {
            crate::app_shortcuts::plan(&config.live_bindings(), &ask.app, &ask.wanted)
        });
        // (No configuration known, or a question is up already: refused.)
        let Some(planned) = planned.filter(|_| state.borrow().shortcut_ask.is_none()) else {
            log::debug!("shortcuts for {} are refused: no configuration, or another question is up", ask.app);
            let _ = ask.answer.send_blocking(None);
            return;
        };
        if !planned
            .iter()
            .any(|(_, plan)| matches!(plan, Plan::Bind(_)))
        {
            let bound =
                state.borrow().running.as_ref().map(|(config, _)| {
                    crate::app_shortcuts::bound(&config.desktop.bindings, &ask.app)
                });
            log::debug!("{} asks for no new shortcut: told what it has", ask.app);
            let _ = ask.answer.send_blocking(Some(bound.unwrap_or_default()));
            return;
        }
        log::debug!("the question of {}'s shortcuts is opened", ask.app);
        let place = Place {
            name: "gevrek-shortcuts".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width: 520,
            height: (200 + 28 * planned.len().min(12)) as u32,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: None,
        };
        let drawn = Rc::clone(state);
        let mut fonts = false;
        let surface = control.open(place, move |ui, _| draw_shortcuts(&drawn, ui, &mut fonts));
        state.borrow_mut().shortcut_ask = Some((surface, ask, planned));
    }

    /// The question: Escape (or its button) refuses; only the button
    /// that says so allows.
    fn draw_shortcuts(shared: &Rc<RefCell<State>>, ui: &mut egui::Ui, fonts: &mut bool) -> Flow {
        use crate::app_shortcuts::Plan;
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let Some((_, ask, planned)) = state.shortcut_ask.as_ref() else {
            return Flow::Close;
        };
        let mut said: Option<bool> = context
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            .then_some(false);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                    .corner_radius(14)
                    .inner_margin(18),
            )
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Keys for an application")
                        .size(17.0)
                        .strong()
                        .color(palette.text),
                );
                ui.add_space(6.0);
                let who = if ask.app.is_empty() {
                    "An application"
                } else {
                    ask.app.as_str()
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{who} asks for keys that work whatever window has the keyboard:"
                    ))
                    .color(palette.text),
                );
                ui.add_space(6.0);
                for (wanted, plan) in planned.iter().take(12) {
                    ui.horizontal(|ui| {
                        let what = if wanted.description.trim().is_empty() {
                            &wanted.id
                        } else {
                            &wanted.description
                        };
                        ui.label(egui::RichText::new(what).color(palette.text));
                        match plan {
                            Plan::Bind(chord) => {
                                ui.label(
                                    egui::RichText::new(chord.words())
                                        .monospace()
                                        .color(palette.accent_fg()),
                                );
                            }
                            Plan::Bound(keys) => {
                                ui.label(
                                    egui::RichText::new(format!("{keys} (it has them)"))
                                        .monospace()
                                        .color(palette.muted),
                                );
                            }
                            Plan::Not(why) => {
                                ui.label(
                                    egui::RichText::new(format!("no keys: {why}"))
                                        .color(palette.muted),
                                );
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "They are bindings like any other afterwards: changed or taken out in gevrek studio.",
                    )
                    .size(12.0)
                    .color(palette.faint),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                    if ui.button("Allow").clicked() {
                        said = Some(true);
                    }
                    if ui.button("Refuse").clicked() {
                        said = Some(false);
                    }
                });
            });
        let Some(said) = said else {
            return Flow::Continue;
        };
        let Some((_, ask, planned)) = state.shortcut_ask.take() else {
            return Flow::Close;
        };
        let Some((config, revision)) = state.running.clone().filter(|_| said) else {
            log::debug!("the shortcuts of {} are refused", ask.app);
            let _ = ask.answer.send_blocking(None);
            return Flow::Close;
        };
        let mut changed = config;
        for (wanted, plan) in &planned {
            if let Plan::Bind(chord) = plan {
                changed
                    .desktop
                    .bindings
                    .extend(crate::app_shortcuts::bindings_of(&ask.app, wanted, chord));
            }
        }
        let bound = crate::app_shortcuts::bound(&changed.desktop.bindings, &ask.app);
        state.link.send(Request::Apply {
            config: Box::new(changed),
            base_revision: revision,
            client_id: "gevrek-shell".to_string(),
        });
        log::debug!("{} is given {} shortcuts", ask.app, bound.len());
        let _ = ask.answer.send_blocking(Some(bound));
        Flow::Close
    }

    /// polkit asks for the user's word, or takes a request back.
    fn from_agent(
        state: &Rc<RefCell<State>>,
        message: crate::polkit::FromAgent,
        control: &mut Control,
    ) {
        match message {
            crate::polkit::FromAgent::Begin(ask) => {
                state.borrow_mut().auth_waiting.push(ask);
                next_auth(state, control);
            }
            crate::polkit::FromAgent::Cancel(cookie) => {
                let mut shared = state.borrow_mut();
                shared.auth_waiting.retain(|ask| ask.cookie != cookie);
                if shared
                    .auth
                    .as_ref()
                    .is_some_and(|auth| auth.ask.cookie == cookie)
                {
                    if let Some(auth) = shared.auth.take() {
                        control.close(auth.surface);
                    }
                }
                drop(shared);
                next_auth(state, control);
            }
        }
    }

    /// Show the next request that waits, when none is shown.
    fn next_auth(state: &Rc<RefCell<State>>, control: &mut Control) {
        let mut shared = state.borrow_mut();
        if shared.auth.is_some() || shared.auth_waiting.is_empty() {
            return;
        }
        let ask = shared.auth_waiting.remove(0);
        log::debug!("the authentication dialog is opened for {}", ask.user);
        let place = Place {
            name: "gevrek-authentication".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width: 520,
            height: 236,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: None,
        };
        let drawn = Rc::clone(state);
        let mut fonts = false;
        let surface = control.open(place, move |ui, _| draw_auth(&drawn, ui, &mut fonts));
        let (user, cookie) = (ask.user.clone(), ask.cookie.clone());
        shared.auth = Some(Auth {
            surface,
            ask,
            password: String::new(),
            checking: true,
            asked: None,
            was_asked: false,
            said: String::new(),
            problem: String::new(),
        });
        let sender = shared.verdicts.clone();
        drop(shared);
        auth_check(state, user, cookie, sender);
    }

    /// Have the system check the user for the request that is shown: on
    /// a thread of its own, which asks the dialog what the system asks
    /// and tells it what the system says.
    fn auth_check(
        state: &Rc<RefCell<State>>,
        user: String,
        cookie: String,
        sender: Option<gevrek_shellkit::layer::Sender<Message>>,
    ) {
        let Some(sender) = sender else {
            return;
        };
        let (asks, says, ends) = (sender.clone(), sender.clone(), sender);
        let (asked_for, said_for, ended_for) = (cookie.clone(), cookie.clone(), cookie.clone());
        let spawned = std::thread::Builder::new()
            .name("gevrek-auth-check".to_string())
            .spawn(move || {
                let verdict = crate::polkit::check(
                    &user,
                    &cookie,
                    |question, seen| {
                        let (answer, typed) = std::sync::mpsc::channel();
                        asks.send(Message::AuthAsks {
                            cookie: asked_for.clone(),
                            question: question.to_string(),
                            seen,
                            answer,
                        });
                        typed.recv().ok().flatten()
                    },
                    |text| {
                        says.send(Message::AuthSays {
                            cookie: said_for.clone(),
                            text: text.to_string(),
                        });
                    },
                );
                ends.send(Message::Verdict(ended_for, verdict));
            });
        if spawned.is_err() {
            if let Some(auth) = state.borrow_mut().auth.as_mut() {
                auth.checking = false;
                auth.problem = "The check could not be started.".to_string();
            }
        }
    }

    /// polkit's helper answered the password that was given.
    fn auth_verdict(
        state: &Rc<RefCell<State>>,
        cookie: &str,
        verdict: crate::polkit::Verdict,
        control: &mut Control,
    ) {
        let again = {
            let mut shared = state.borrow_mut();
            let sender = shared.verdicts.clone();
            // (The verdict of a request that is not shown any more is
            // nobody's.)
            let Some(auth) = shared
                .auth
                .as_mut()
                .filter(|auth| auth.ask.cookie == cookie)
            else {
                return;
            };
            auth.checking = false;
            auth.asked = None;
            match verdict {
                crate::polkit::Verdict::Yes => {
                    log::debug!("the authentication of {} held", auth.ask.user);
                    let _ = auth.ask.answer.send_blocking(true);
                    let surface = auth.surface;
                    shared.auth = None;
                    control.close(surface);
                    None
                }
                // Not right: said, and the system asks once more.
                crate::polkit::Verdict::No(said) => {
                    auth.problem = if said.is_empty() {
                        "That was not right.".to_string()
                    } else {
                        said
                    };
                    auth.said.clear();
                    control.redraw(auth.surface);
                    if std::mem::take(&mut auth.was_asked) {
                        auth.checking = true;
                        Some((auth.ask.user.clone(), auth.ask.cookie.clone(), sender))
                    } else {
                        None
                    }
                }
                crate::polkit::Verdict::Refused => None,
                crate::polkit::Verdict::Broken(why) => {
                    auth.problem = why;
                    control.redraw(auth.surface);
                    None
                }
            }
        };
        if let Some((user, cookie, sender)) = again {
            auth_check(state, user, cookie, sender);
        }
        next_auth(state, control);
    }

    fn draw_auth(shared: &Rc<RefCell<State>>, ui: &mut egui::Ui, fonts: &mut bool) -> Flow {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let Some(auth) = state.auth.as_mut() else {
            return Flow::Close;
        };
        let mut submit = false;
        let mut dismiss =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border_strong()))
                    .corner_radius(14)
                    .inner_margin(18),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(icons::SHIELD_CHECK)
                            .size(22.0)
                            .color(palette.accent_fg()),
                    );
                    ui.label(
                        egui::RichText::new("Your word is needed")
                            .size(17.0)
                            .strong()
                            .color(palette.text),
                    );
                });
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(&auth.ask.message).color(palette.text))
                        .wrap(),
                );
                ui.label(
                    egui::RichText::new(&auth.ask.action)
                        .size(11.5)
                        .monospace()
                        .color(palette.faint),
                );
                ui.add_space(10.0);
                // What the system asks, in its words (its usual
                // question for a password is said with whose it is).
                let (question, seen) = match &auth.asked {
                    Some((question, seen, _)) => (
                        crate::polkit::question_shown(question, &auth.ask.user),
                        *seen,
                    ),
                    None => (format!("Password of {}", auth.ask.user), false),
                };
                let asking = auth.asked.is_some();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(question).color(palette.muted));
                    let field_id = egui::Id::unique("gevrek authentication password");
                    if asking {
                        ui.memory_mut(|memory| memory.request_focus(field_id));
                    }
                    ui.add_enabled(
                        asking,
                        egui::TextEdit::singleline(&mut auth.password)
                            .id(field_id)
                            .password(!seen)
                            .desired_width(220.0),
                    );
                });
                if asking
                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    && !auth.password.is_empty()
                {
                    submit = true;
                }
                ui.add_space(6.0);
                if !auth.said.is_empty() {
                    // (What the system says meanwhile: a finger to be
                    // put on a reader.)
                    ui.label(egui::RichText::new(&auth.said).color(palette.text));
                } else if auth.checking && auth.problem.is_empty() {
                    ui.label(egui::RichText::new("Checking…").color(palette.muted));
                } else if !auth.problem.is_empty() {
                    ui.label(egui::RichText::new(&auth.problem).color(palette.error));
                } else {
                    ui.label(
                        egui::RichText::new("Enter gives it · Esc refuses")
                            .size(12.0)
                            .color(palette.faint),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Refuse").clicked() {
                        dismiss = true;
                    }
                });
            });
        if submit {
            // The answer goes to the system's check, which goes on.
            if let Some((_, _, answer)) = auth.asked.take() {
                auth.checking = true;
                auth.problem.clear();
                auth.said.clear();
                // (What was typed goes to the check alone, never to the log.)
                log::debug!("the authentication dialog's answer goes to the system's check");
                let _ = answer.send(Some(std::mem::take(&mut auth.password)));
            }
        }
        if dismiss {
            // (Taken back, whatever the system's check is at: it is told
            // at its next question, and its verdict is nobody's.)
            log::debug!("the authentication dialog is dismissed");
            if let Some((_, _, answer)) = auth.asked.take() {
                let _ = answer.send(None);
            }
            let _ = auth.ask.answer.send_blocking(false);
            state.auth = None;
            if let Some(shell) = &state.verdicts {
                shell.send(Message::NextRequest);
            }
            return Flow::Close;
        }
        Flow::Continue
    }

    /// A display's name, a window's place on it (logical pixels from the
    /// display's corner) and the display's size, each where it is known.
    type FocusedPlace = (
        Option<String>,
        Option<(f32, f32, f32, f32)>,
        Option<(f32, f32)>,
    );

    /// The display that has the keyboard's window, and that window's
    /// place on it (logical pixels from the display's corner).
    fn focused_place(state: &State) -> FocusedPlace {
        let Some(wm) = &state.wm else {
            return (None, None, None);
        };
        let monitors = wm["monitors"].as_array().cloned().unwrap_or_default();
        let Some(monitor) = monitors
            .iter()
            .find(|monitor| monitor["is_focused"].as_bool().unwrap_or(false))
            .or(monitors.first())
        else {
            return (None, None, None);
        };
        let name = monitor["connector"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| monitor["name"].as_str())
            .map(str::to_string);
        let number = |value: &serde_json::Value| value.as_f64().unwrap_or(0.0) as f32;
        let corner = (
            number(&monitor["position"]["x"]),
            number(&monitor["position"]["y"]),
        );
        let size = (
            number(&monitor["size"]["width"]),
            number(&monitor["size"]["height"]),
        );
        let window = monitor["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|workspace| workspace["id"] == monitor["workspace"]["id"])
            .flat_map(|workspace| {
                workspace["detailed_windows"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .find(|window| window["is_focused"].as_bool().unwrap_or(false))
            .map(|window| {
                (
                    number(&window["position"]["x"]) - corner.0,
                    number(&window["position"]["y"]) - corner.1,
                    number(&window["size"]["width"]),
                    number(&window["size"]["height"]),
                )
            });
        (name, window, Some(size))
    }

    /// Say something of the shell's own as a notification.
    fn tell(state: &Rc<RefCell<State>>, summary: &str, body: &str, control: &mut Control) {
        state
            .borrow_mut()
            .notes
            .tell("gevrek", summary, body, 5000, Instant::now());
        show_notes(state, control);
    }

    /// Write a picture to a file and say where it is.
    fn keep(
        state: &Rc<RefCell<State>>,
        picture: &crate::screenshot::Picture,
        control: &mut Control,
    ) {
        // On the clipboard too, to be pasted at once (where the clipboard
        // is kept by the shell).
        let wanted = state.borrow().screenshots.clone();
        let copied = wanted.clipboard
            && match (&state.borrow().clipboard, crate::screenshot::png(picture)) {
                (Some(clipboard), Ok(png)) => {
                    clipboard.put_picture(png);
                    true
                }
                _ => false,
            };
        // Only for pasting, where the configuration says so (and the
        // clipboard took it: a picture is never lost for that).
        if !wanted.save && copied {
            return tell(
                state,
                "Screenshot copied",
                "It is on the clipboard.",
                control,
            );
        }
        let folder = crate::screenshot::folder_of(&wanted.folder);
        match crate::screenshot::save(picture, &folder) {
            Ok(path) => tell(
                state,
                if copied {
                    "Screenshot saved and copied"
                } else {
                    "Screenshot saved"
                },
                &path.display().to_string(),
                control,
            ),
            Err(error) => tell(
                state,
                "The screenshot was not saved",
                &format!("{error:#}"),
                control,
            ),
        }
    }

    /// The recording's key: begin a film of the display with the
    /// keyboard (or, with `window`, of the place the window with the
    /// keyboard has now), or end the one that is going on and say where
    /// it is.
    fn toggle_recording(state: &Rc<RefCell<State>>, window: bool, control: &mut Control) {
        let going = state.borrow_mut().recording.take();
        if let Some(recording) = going {
            log::debug!("the recording ends: asked again");
            for id in state.borrow().panels.values() {
                control.redraw(*id);
            }
            // The file is finished beside the shell: it waits for nothing.
            if let Some(sender) = state.borrow().verdicts.clone() {
                if let Err(error) = std::thread::Builder::new()
                    .name("gevrek-record-end".to_string())
                    .spawn(move || {
                        sender.send(Message::Recorded(recording.finish()));
                    })
                {
                    log::warn!("the recording is not finished: no thread for it: {error}");
                }
            }
            return show_osd(state, Osd::words(icons::STOP, "Recording ended"), control);
        }
        let (output, ..) = focused_place(&state.borrow());
        let (of, words) = if window {
            // The window by its own pictures: it is followed wherever
            // it is moved to.
            let Some((title, app_id)) = state.borrow().wm.as_ref().and_then(focused_window) else {
                return tell(
                    state,
                    "No recording",
                    "No window has the keyboard to be recorded.",
                    control,
                );
            };
            (
                crate::record::Of::Window { title, app_id },
                "Recording the window: the same key ends it",
            )
        } else {
            (
                crate::record::Of::Display { output, part: None },
                "Recording the screen: the same key ends it",
            )
        };
        begin_recording(state, of, words, control);
    }

    /// The title and the program of the window that has the keyboard.
    fn focused_window(wm: &serde_json::Value) -> Option<(String, String)> {
        wm["monitors"]
            .as_array()?
            .iter()
            .flat_map(|monitor| monitor["workspace"]["detailed_windows"].as_array())
            .flatten()
            .find(|window| window["is_focused"].as_bool().unwrap_or(false))
            .map(|window| {
                let said = |key: &str| window[key].as_str().unwrap_or_default().to_string();
                (said("title"), said("app_id"))
            })
    }

    /// Begin a film of something, and say so.
    fn begin_recording(
        state: &Rc<RefCell<State>>,
        of: crate::record::Of,
        words: &str,
        control: &mut Control,
    ) {
        let (folder, sound) = {
            let state = state.borrow();
            (
                crate::screenshot::folder_of(&state.screenshots.folder),
                state.screenshots.record_sound,
            )
        };
        match crate::record::start(of, &folder, sound) {
            Ok(recording) => {
                // (Asked for and not to be had: said, and the film is
                // taken without it.)
                let silent = sound && !recording.sound;
                {
                    let mut state = state.borrow_mut();
                    state.recording = Some(recording);
                    // (Its mark comes on the panels.)
                    for id in state.panels.values() {
                        control.redraw(*id);
                    }
                }
                if silent {
                    tell(
                        state,
                        "Recording without sound",
                        "GStreamer cannot listen to the sound server here (its pulsesrc is missing).",
                        control,
                    );
                }
                show_osd(state, Osd::words(icons::RECORD, words), control);
            }
            Err(error) => tell(state, "No recording", &format!("{error:#}"), control),
        }
    }

    /// A screenshot key: the display, the focused window, or a part
    /// chosen by dragging.
    fn screenshot(state: &Rc<RefCell<State>>, which: &str, control: &mut Control) {
        let (output, window, logical) = focused_place(&state.borrow());
        log::debug!("a screenshot ({which}) of {}", output.as_deref().unwrap_or("the display"));
        let picture = match crate::screenshot::take(output.as_deref()) {
            Ok(picture) => picture,
            Err(error) => {
                log::debug!("no picture of the screen: {error:#}");
                return tell(
                    state,
                    "No picture of the screen",
                    &format!("{error:#}"),
                    control,
                )
            }
        };
        // The picture is in the display's own pixels; places are logical.
        let scale = logical
            .filter(|size| size.0 > 0.0)
            .map_or(1.0, |size| picture.width as f32 / size.0);
        match which {
            "screenshot_window" => {
                let part = window.and_then(|(x, y, width, height)| {
                    let device = |value: f32| (value * scale).round().max(0.0) as u32;
                    crate::screenshot::crop(
                        &picture,
                        (device(x), device(y), device(width), device(height)),
                    )
                });
                match part {
                    Some(part) => keep(state, &part, control),
                    None => tell(
                        state,
                        "No window has the keyboard",
                        "The whole display was not taken instead.",
                        control,
                    ),
                }
            }
            "screenshot_region" => {
                if state.borrow().region.is_some() {
                    return;
                }
                state.borrow_mut().region_output = output.clone();
                let place = Place {
                    name: "gevrek-screenshot".to_string(),
                    edge: Edge::Fill,
                    level: Level::Overlay,
                    width: 0,
                    height: 0,
                    reserve: false,
                    keyboard: Keyboard::Exclusive,
                    margin: 0,
                    output,
                };
                let picture = Rc::new(picture);
                let shared = Rc::clone(state);
                let mut texture: Option<egui::TextureHandle> = None;
                let id = control.open(place, move |ui, control| {
                    draw_region(&shared, ui, control, &mut texture)
                });
                state.borrow_mut().region = Some((id, picture, None));
            }
            _ => keep(state, &picture, control),
        }
    }

    /// Write a picture an application asked for through the portal to
    /// a file of gevrek's own (under the cache), for the portal to hand
    /// on. `None` where it could not be written.
    fn hand_over(picture: &crate::screenshot::Picture) -> Option<std::path::PathBuf> {
        let folder = gevrek_model::paths::cache_dir().join("portal");
        match crate::screenshot::save(picture, &folder) {
            Ok(file) => Some(file),
            Err(error) => {
                log::warn!("the picture an application asked for was not written: {error:#}");
                None
            }
        }
    }

    /// An application asks (through the portal) for a picture of the
    /// screen: the display that has the keyboard at once, or what the
    /// user chooses of it.
    fn picture_asked(
        state: &Rc<RefCell<State>>,
        ask: crate::portal::PictureAsk,
        control: &mut Control,
    ) {
        if !ask.interactive {
            let (output, ..) = focused_place(&state.borrow());
            let file = match crate::screenshot::take(output.as_deref()) {
                Ok(picture) => hand_over(&picture),
                Err(error) => {
                    log::warn!("no picture of the screen for an application: {error:#}");
                    None
                }
            };
            log::debug!(
                "an application asked for a picture of the screen: {}",
                if file.is_some() { "given" } else { "none" }
            );
            let _ = ask.answer.send_blocking(file);
            return;
        }
        choose_for(state, RegionFor::Picture(ask.answer), control);
    }

    /// The frozen picture of the display is put up for this purpose;
    /// where it cannot be (one is up already, no picture is to be had)
    /// whoever asked is told that nothing was chosen.
    fn choose_for(state: &Rc<RefCell<State>>, purpose: RegionFor, control: &mut Control) {
        let refuse = |purpose: RegionFor| match purpose {
            RegionFor::Picture(answer) => {
                log::debug!("an application's picture of the screen is refused: one is chosen already, or none can be had");
                let _ = answer.send_blocking(None);
            }
            RegionFor::Colour(answer) => {
                log::debug!("an application's colour is refused: a part is chosen already, or no picture can be had");
                let _ = answer.send_blocking(None);
            }
            _ => {}
        };
        if state.borrow().region.is_some() {
            return refuse(purpose);
        }
        state.borrow_mut().region_for = purpose;
        screenshot(state, "screenshot_region", control);
        if state.borrow().region.is_none() {
            let purpose = std::mem::take(&mut state.borrow_mut().region_for);
            refuse(purpose);
        }
    }

    /// The frozen picture of the display, to drag a part out of.
    fn draw_region(
        shared: &Rc<RefCell<State>>,
        ui: &mut egui::Ui,
        control: &mut Control,
        texture: &mut Option<egui::TextureHandle>,
    ) -> Flow {
        let context = ui.ctx().clone();
        let (picture, began, accent) = {
            let state = shared.borrow();
            let Some((_, picture, began)) = &state.region else {
                return Flow::Close;
            };
            (Rc::clone(picture), *began, state.accent)
        };
        let shown = texture.get_or_insert_with(|| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [picture.width as usize, picture.height as usize],
                &picture.pixels,
            );
            context.load_texture("gevrek screenshot", image, egui::TextureOptions::LINEAR)
        });
        let screen = ui.max_rect();
        let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let painter = ui.painter();
        let for_colour = matches!(shared.borrow().region_for, RegionFor::Colour(_));
        // (A colour is picked from the picture as it is, not dimmed.)
        let light = if for_colour { 255 } else { 110 };
        painter.image(shown.id(), screen, whole, egui::Color32::from_gray(light));
        context.set_cursor_icon(egui::CursorIcon::Crosshair);
        // The picture's pixel under a place on the surface.
        let pixel_at = |at: egui::Pos2| -> Option<[u8; 3]> {
            let x = ((at.x / screen.width()) * picture.width as f32) as i64;
            let y = ((at.y / screen.height()) * picture.height as f32) as i64;
            if x < 0 || y < 0 || x >= i64::from(picture.width) || y >= i64::from(picture.height) {
                return None;
            }
            let index = (y as usize * picture.width as usize + x as usize) * 4;
            picture
                .pixels
                .get(index..index + 3)
                .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        };
        let response = ui.interact(screen, ui.scope_id().with("region"), egui::Sense::drag());
        let pointer = response.interact_pointer_pos();
        if response.drag_started() {
            if let (Some(at), Some(region)) = (pointer, shared.borrow_mut().region.as_mut()) {
                region.2 = Some(at);
            }
        }
        let chosen = began
            .zip(pointer.or_else(|| context.pointer_latest_pos()))
            .map(|(from, to)| egui::Rect::from_two_pos(from, to))
            .filter(|_| !for_colour);
        if for_colour {
            // The colour under the pointer, shown beside it.
            if let Some((at, [red, green, blue])) = context
                .pointer_latest_pos()
                .and_then(|at| pixel_at(at).map(|pixel| (at, pixel)))
            {
                let swatch =
                    egui::Rect::from_min_size(at + egui::vec2(18.0, 18.0), egui::vec2(44.0, 44.0));
                painter.rect_filled(swatch, 6.0, egui::Color32::from_rgb(red, green, blue));
                painter.rect_stroke(
                    swatch,
                    6.0,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
            painter.text(
                screen.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Press on the colour to take · Esc leaves",
                egui::FontId::proportional(16.0),
                egui::Color32::WHITE,
            );
        } else if let Some(part) = chosen {
            // The chosen part at its full light, with an edge.
            let uv = egui::Rect::from_min_max(
                egui::pos2(part.min.x / screen.width(), part.min.y / screen.height()),
                egui::pos2(part.max.x / screen.width(), part.max.y / screen.height()),
            );
            painter.image(shown.id(), part, uv, egui::Color32::WHITE);
            painter.rect_stroke(
                part,
                0.0,
                egui::Stroke::new(1.5, accent),
                egui::StrokeKind::Outside,
            );
        } else {
            painter.text(
                screen.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                match shared.borrow().region_for {
                    RegionFor::Film => "Drag over the part to record · Esc leaves",
                    RegionFor::Picture(_) => {
                        "Drag over the part to hand over, or press for all of it · Esc leaves"
                    }
                    _ => "Drag over the part to keep · Esc leaves",
                },
                egui::FontId::proportional(16.0),
                egui::Color32::WHITE,
            );
        }
        let mut done =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut taken = None;
        let mut pixels = None;
        // Let go where it was pressed: no part, but a place.
        let mut pressed_at = None;
        // (A press let go at once may never have been a drag to the
        // toolkit: the button going up ends it all the same.)
        let let_go =
            response.drag_stopped() || context.input(|input| input.pointer.primary_released());
        if let_go {
            if chosen.is_none_or(|part| part.width() < 3.0 && part.height() < 3.0) {
                pressed_at = pointer.or_else(|| context.pointer_latest_pos());
            }
            if let Some(part) = chosen.filter(|part| part.width() >= 3.0 && part.height() >= 3.0) {
                let across = picture.width as f32 / screen.width();
                let down = picture.height as f32 / screen.height();
                let part = (
                    (part.min.x * across).round().max(0.0) as u32,
                    (part.min.y * down).round().max(0.0) as u32,
                    (part.width() * across).round() as u32,
                    (part.height() * down).round() as u32,
                );
                pixels = Some(part);
                taken = crate::screenshot::crop(&picture, part);
            }
            done = true;
        }
        if done {
            let (purpose, output) = {
                let mut state = shared.borrow_mut();
                state.region = None;
                (
                    std::mem::take(&mut state.region_for),
                    state.region_output.take(),
                )
            };
            match (purpose, pixels, taken) {
                // Chosen to be filmed: from now on, that part.
                (RegionFor::Film, Some(part), _) => begin_recording(
                    shared,
                    crate::record::Of::Display {
                        output,
                        part: Some(part),
                    },
                    "Recording the part: the same key ends it",
                    control,
                ),
                (RegionFor::Keep, _, Some(part)) => keep(shared, &part, control),
                // For an application: the part, or all of the display
                // where it was only pressed on; nothing where it was left.
                (RegionFor::Picture(answer), _, taken) => {
                    let wanted = taken.or_else(|| {
                        pressed_at.map(|_| crate::screenshot::Picture {
                            width: picture.width,
                            height: picture.height,
                            pixels: picture.pixels.clone(),
                        })
                    });
                    let file = wanted.and_then(|picture| hand_over(&picture));
                    log::debug!(
                        "an application's picture of the screen: {}",
                        if file.is_some() { "given" } else { "left by the person" }
                    );
                    let _ = answer.send_blocking(file);
                }
                (RegionFor::Colour(answer), _, _) => {
                    let colour = pressed_at.and_then(pixel_at).map(|[red, green, blue]| {
                        (
                            f64::from(red) / 255.0,
                            f64::from(green) / 255.0,
                            f64::from(blue) / 255.0,
                        )
                    });
                    log::debug!(
                        "an application's colour: {}",
                        if colour.is_some() { "picked" } else { "left by the person" }
                    );
                    let _ = answer.send_blocking(colour);
                }
                _ => {}
            }
            return Flow::Close;
        }
        Flow::Continue
    }

    /// A frosted surface is about to open: the display is taken as it is
    /// now (beside what draws) for its glass to show, windows and all.
    /// Until that has come the glass shows the wallpaper.
    fn glass_scene(state: &Rc<RefCell<State>>) {
        let mut state = state.borrow_mut();
        // (Another frosted surface is open: its picture serves, and has
        // not this one in it.)
        if !state.glass.enabled || state.centre.is_some() || state.control_centre.is_some() {
            return;
        }
        state.frost_scene = None;
        state.frost_asked += 1;
        let asked = state.frost_asked;
        let (output, ..) = focused_place(&state);
        let Some(sender) = state.verdicts.clone() else {
            return;
        };
        std::thread::spawn(move || {
            let picture = crate::screenshot::take(output.as_deref())
                .ok()
                .and_then(|taken| {
                    crate::frost::frosted_scene(taken.width, taken.height, taken.pixels)
                });
            sender.send(Message::Scene(asked, picture));
        });
    }

    /// Open the notification centre (what arrived, to look at again), or
    /// close it when it is open.
    fn toggle_centre(state: &Rc<RefCell<State>>, control: &mut Control) {
        // Open: it goes (the way it came), and is closed when it is gone.
        if let Some((id, coming)) = state.borrow_mut().centre.as_mut() {
            log::debug!("the notification centre goes: asked again");
            coming.leave(Instant::now());
            control.redraw(*id);
            return;
        }
        log::debug!("the notification centre is opened");
        glass_scene(state);
        let place = Place {
            name: "gevrek-notification-centre".to_string(),
            edge: Edge::Right,
            level: Level::Overlay,
            width: 400,
            height: 0,
            reserve: false,
            // (It does not take the keyboard from the window that has
            // it: it has the keyboard when something in it is pressed.)
            keyboard: Keyboard::OnDemand,
            margin: 12,
            output: None,
        };
        let shared = Rc::clone(state);
        let mut fonts = false;
        let id = control.open(place, move |ui, _| {
            let context = ui.ctx().clone();
            let mut state = shared.borrow_mut();
            if !fonts {
                theme::install_fonts(&context, state.settings.system_font);
                fonts = true;
            }
            let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
            theme::apply(&context, &palette, &state.settings);
            let escape = context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let now = Instant::now();
            let Some((_, coming)) = state.centre else {
                return Flow::Close;
            };
            if coming.gone(now, state.moving) {
                state.centre = None;
                return Flow::Close;
            }
            let glass = crate::frost::texture(
                &context,
                state.frost_scene.as_ref().or(state.frost.as_ref()),
            );
            let display = state
                .wm
                .as_ref()
                .and_then(crate::frost::display_with_keyboard)
                .unwrap_or(egui::Vec2::ZERO);
            let look = crate::frost::Look {
                glass: glass.as_ref().filter(|_| state.glass.enabled),
                tint: state.glass.tint,
                // (At the right edge, a margin in from it and from the top.)
                origin: egui::pos2(display.x - 12.0 - 400.0, 12.0),
                display,
                shown: coming.shown(now, state.moving),
                from_right: true,
            };
            let asked = crate::notifications::draw_centre(
                ui,
                &palette,
                &state.notes,
                now,
                state.notes.quiet,
                &look,
            );
            let close = escape || asked.close;
            state.notes.quiet = asked.quiet;
            if asked.forget {
                state.notes.forget_history();
            }
            if close {
                if let Some((_, coming)) = state.centre.as_mut() {
                    coming.leave(now);
                }
                context.request_repaint();
            }
            Flow::Continue
        });
        state.borrow_mut().centre = Some((id, crate::frost::Coming::new(Instant::now())));
    }

    /// Open the control centre, or close it when it is open.
    fn toggle_control(state: &Rc<RefCell<State>>, control: &mut Control) {
        // Open: it goes (the way it came), and is closed when it is gone.
        if let Some((id, _, _, coming)) = state.borrow_mut().control_centre.as_mut() {
            log::debug!("the control centre goes: asked again");
            coming.leave(Instant::now());
            control.redraw(*id);
            return;
        }
        log::debug!("the control centre is opened");
        glass_scene(state);
        let place = Place {
            name: "gevrek-control-centre".to_string(),
            edge: Edge::Right,
            level: Level::Overlay,
            width: crate::control::WIDTH,
            height: 0,
            reserve: false,
            // (It does not take the keyboard from the window that has
            // it: it has the keyboard when something in it is pressed.)
            keyboard: Keyboard::OnDemand,
            margin: 12,
            output: None,
        };
        let shared = Rc::clone(state);
        let mut fonts = false;
        let id = control.open(place, move |ui, control| {
            let context = ui.ctx().clone();
            let asked = {
                let mut state = shared.borrow_mut();
                let state = &mut *state;
                if !fonts {
                    theme::install_fonts(&context, state.settings.system_font);
                    fonts = true;
                }
                let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
                theme::apply(&context, &palette, &state.settings);
                let escape = context
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                let now = Instant::now();
                let moving = state.moving;
                let glass = crate::frost::texture(
                    &context,
                    state.frost_scene.as_ref().or(state.frost.as_ref()),
                );
                let display = state
                    .wm
                    .as_ref()
                    .and_then(crate::frost::display_with_keyboard)
                    .unwrap_or(egui::Vec2::ZERO);
                let Some((_, held, mixer, coming)) = state.control_centre.as_mut() else {
                    return Flow::Close;
                };
                if coming.gone(now, moving) {
                    state.control_centre = None;
                    return Flow::Close;
                }
                let look = crate::frost::Look {
                    glass: glass.as_ref().filter(|_| state.glass.enabled),
                    tint: state.glass.tint,
                    // (At the right edge, a margin in from it and from the top.)
                    origin: egui::pos2(display.x - 12.0 - crate::control::WIDTH as f32, 12.0),
                    display,
                    shown: coming.shown(now, moving),
                    from_right: true,
                };
                let facts = crate::control::Facts {
                    volume: state.system.volume,
                    outputs: mixer.outputs.clone(),
                    inputs: mixer.inputs.clone(),
                    streams: mixer.streams.clone(),
                    brightness: state.system.brightness,
                    quiet: state.notes.quiet,
                    night_light: state.night_light,
                    recording: state.recording.is_some(),
                    profiles: state.power_profiles.clone(),
                    network: state.system.network.clone(),
                    playing: state.playing.clone(),
                    bluetooth: state.bluetooth.clone(),
                    wifi: state.wifi.clone(),
                    joining: state.wifi_joining.clone(),
                    wifi_looking: state.wifi_looking,
                };
                let mut asked = crate::control::draw(ui, &palette, &facts, held, &look);
                if escape {
                    asked.push(crate::control::Ask::Close);
                }
                asked
            };
            let mut close = false;
            for ask in asked {
                use crate::control::Ask;
                use gevrek_system::sound::{self, Device};
                // What takes a moment is done beside the shell, which is
                // told how things stand afterwards.
                let beside = |work: Box<dyn FnOnce() + Send>| {
                    if let Some(sender) = shared.borrow().verdicts.clone() {
                        std::thread::spawn(move || {
                            work();
                            sender.send(Message::System(crate::panel::SystemView::read()));
                        });
                    }
                };
                match ask {
                    Ask::Close => close = true,
                    Ask::Volume(step) => beside(Box::new(move || {
                        sound::change(Device::Speaker, step);
                    })),
                    Ask::Mute => beside(Box::new(|| {
                        sound::toggle_mute(Device::Speaker);
                    })),
                    Ask::Output(_) | Ask::Input(_) | Ask::Stream(..) | Ask::StreamMute(_) => {
                        if let Some(sender) = shared.borrow().verdicts.clone() {
                            std::thread::spawn(move || {
                                match ask {
                                    Ask::Output(device) => sound::choose_output(device),
                                    Ask::Input(device) => sound::choose_input(device),
                                    Ask::Stream(id, percent) => sound::set_stream(id, percent),
                                    Ask::StreamMute(id) => sound::toggle_stream_mute(id),
                                    _ => false,
                                };
                                sender.send(Message::Mixer(sound::mixer()));
                                sender.send(Message::System(crate::panel::SystemView::read()));
                            });
                        }
                    }
                    Ask::Brightness(step) => beside(Box::new(move || {
                        gevrek_system::brightness::change(step);
                    })),
                    Ask::Quiet(quiet) => shared.borrow_mut().notes.quiet = quiet,
                    Ask::NightLight => {
                        let mut state = shared.borrow_mut();
                        // (Shown at once; the configuration says the same
                        // when it comes.)
                        state.night_light = !state.night_light;
                        let command =
                            serde_json::json!({"command": "shell", "action": "night_light"});
                        state.link.send(Request::Dispatch(command.to_string()));
                        state.link.send(Request::Fetch);
                    }
                    Ask::Media(what) => {
                        let sender = shared.borrow().verdicts.clone();
                        std::thread::spawn(move || {
                            crate::media::tell(what);
                            // (The player has taken the key in a moment.)
                            std::thread::sleep(std::time::Duration::from_millis(400));
                            if let Some(sender) = sender {
                                sender.send(Message::Playing(crate::media::playing()));
                            }
                        });
                    }
                    Ask::Record => toggle_recording(&shared, false, control),
                    Ask::Lock => {
                        let command = serde_json::json!({"command": "shell", "action": "lock"});
                        shared
                            .borrow()
                            .link
                            .send(Request::Dispatch(command.to_string()));
                        close = true;
                    }
                    Ask::PowerMenu => {
                        close = true;
                        toggle_power(&shared, control);
                    }
                    Ask::Settings => {
                        close = true;
                        crate::control::open_studio("bar");
                    }
                    Ask::Wifi(on) => {
                        if let Some(sender) = shared.borrow().verdicts.clone() {
                            std::thread::spawn(move || {
                                gevrek_system::wifi::set_enabled(on);
                                if on {
                                    // (A radio that has just come on has
                                    // heard nothing yet.)
                                    look_for_networks(&sender);
                                } else {
                                    sender.send(Message::Wifi(gevrek_system::wifi::read()));
                                }
                                sender.send(Message::System(crate::panel::SystemView::read()));
                            });
                        }
                    }
                    Ask::WifiJoin {
                        name,
                        known,
                        password,
                    } => {
                        let sender = {
                            let mut state = shared.borrow_mut();
                            state.wifi_joining = Some(name.clone());
                            state.verdicts.clone()
                        };
                        if let Some(sender) = sender {
                            std::thread::spawn(move || {
                                if !gevrek_system::wifi::join(&name, known, password.as_deref()) {
                                    log::warn!("the wireless network {name} was not joined");
                                }
                                sender.send(Message::Wifi(gevrek_system::wifi::read()));
                                sender.send(Message::System(crate::panel::SystemView::read()));
                            });
                        }
                        context.request_repaint();
                    }
                    Ask::Bluetooth(on) => {
                        if let Some(sender) = shared.borrow().verdicts.clone() {
                            std::thread::spawn(move || {
                                gevrek_system::bluetooth::set_power(on);
                                // (The radio takes a moment to follow:
                                // told as it stands until it has.)
                                for _ in 0..12 {
                                    let now = gevrek_system::bluetooth::read();
                                    let there =
                                        now.as_ref().is_none_or(|radio| radio.powered == on);
                                    sender.send(Message::Bluetooth(now));
                                    if there {
                                        break;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                }
                            });
                        }
                    }
                    Ask::BluetoothDevice(address, on) => {
                        if let Some(sender) = shared.borrow().verdicts.clone() {
                            std::thread::spawn(move || {
                                gevrek_system::bluetooth::connect(&address, on);
                                sender.send(Message::Bluetooth(gevrek_system::bluetooth::read()));
                            });
                        }
                    }
                    Ask::Profile(profile) => {
                        if let Some(sender) = shared.borrow().verdicts.clone() {
                            std::thread::spawn(move || {
                                gevrek_system::power_profile::choose(&profile);
                                sender
                                    .send(Message::Profiles(gevrek_system::power_profile::read()));
                            });
                        }
                    }
                }
            }
            if close {
                // It goes; the surface is closed when it is gone.
                if let Some((_, _, _, coming)) = shared.borrow_mut().control_centre.as_mut() {
                    coming.leave(Instant::now());
                }
                context.request_repaint();
            }
            Flow::Continue
        });
        // The devices to play on, as the sound server lists them now,
        // and the machine's power profiles. (The programs that play and
        // the devices to record from come a moment later: each program
        // is asked for its loudness.)
        let outputs = gevrek_system::sound::Mixer {
            outputs: gevrek_system::sound::outputs(),
            ..Default::default()
        };
        let profiles = gevrek_system::power_profile::read();
        // What is playing, and how bluetooth stands, are asked beside
        // the shell.
        if let Some(sender) = state.borrow().verdicts.clone() {
            std::thread::spawn(move || {
                sender.send(Message::Mixer(gevrek_system::sound::mixer()));
                sender.send(Message::Playing(crate::media::playing()));
                sender.send(Message::Bluetooth(gevrek_system::bluetooth::read()));
                // (Last: looking for networks may take a few seconds.)
                let wifi = gevrek_system::wifi::read();
                let none_heard = wifi
                    .as_ref()
                    .is_some_and(|wifi| wifi.enabled && wifi.networks.is_empty());
                sender.send(Message::Wifi(wifi));
                if none_heard {
                    look_for_networks(&sender);
                }
            });
        }
        let mut state = state.borrow_mut();
        state.power_profiles = profiles;
        state.control_centre = Some((
            id,
            Default::default(),
            outputs,
            crate::frost::Coming::new(Instant::now()),
        ));
    }

    /// Ask the system to look for wireless networks and tell how the
    /// wireless side stands until some are heard (a radio that has just
    /// come on, or a list that was never asked for, has none at first),
    /// for a dozen seconds at most.
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
            std::thread::sleep(std::time::Duration::from_millis(1500));
            // (Asked again half way: the first asking may have come
            // before the radio was ready to look.)
            if round == 3 {
                gevrek_system::wifi::rescan();
            }
        }
        sender.send(Message::WifiLooking(false));
    }

    /// The running configuration is another: the zones of the desktop
    /// are started, told, or put away, as it says
    /// (`desktop.zones.enabled`).
    fn zones_configured(state: &Rc<RefCell<State>>, control: &mut Control) {
        let mut shared = state.borrow_mut();
        let shared = &mut *shared;
        let Some(config) = shared.running.as_ref().map(|(config, _)| config.desktop.zones.clone()) else {
            return;
        };
        let Some(sender) = shared.verdicts.clone() else {
            return;
        };
        let look = Rc::clone(state);
        crate::zones::surfaces::Running::configured(
            &mut shared.zones,
            &config,
            control,
            move || {
                sender.send(Message::Zones);
            },
            move || {
                Rc::new(move |ui: &egui::Ui, fonts: &mut bool| {
                    let state = look.borrow();
                    let context = ui.ctx();
                    if !*fonts {
                        theme::install_fonts(context, state.settings.system_font);
                        *fonts = true;
                    }
                    let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
                    theme::apply(context, &palette, &state.settings);
                    (palette, state.settings)
                })
            },
            // What an item opens is started by the window manager, as
            // what the launcher starts is: no child of the shell.
            || {
                let tell = shared.link.teller();
                crate::zones::Starter {
                    dispatch: Box::new(move |command| tell(Request::Dispatch(command))),
                    terminal: shared.terminal.clone(),
                }
            },
            // (Nothing draws desktop icons here but the zones.)
            false,
        );
    }

    /// The zones asked to be looked at. What they changed of the
    /// configuration (a zone made, renamed, rolled up) is written as the
    /// shell writes any change of its own.
    fn zones_woken(state: &Rc<RefCell<State>>, control: &mut Control) {
        let changed = {
            let state = state.borrow();
            let Some(zones) = &state.zones else {
                return;
            };
            let changed = zones.woken(control);
            zones.publish(&state.link);
            changed
        };
        let Some(changed) = changed else {
            return;
        };
        let mut state = state.borrow_mut();
        if let Some((mut config, _)) = state.running.clone() {
            config.desktop.zones = changed;
            apply_configuration(&mut state, config);
        }
    }

    /// Open the power menu, or close it when it is open.
    fn toggle_power(state: &Rc<RefCell<State>>, control: &mut Control) {
        if let Some((id, ..)) = state.borrow_mut().power.take() {
            log::debug!("the power menu is closed: asked again");
            control.close(id);
            return;
        }
        log::debug!("the power menu is opened");
        let place = Place {
            name: "gevrek-power".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width: crate::power::SIZE.0,
            height: crate::power::SIZE.1,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: None,
        };
        let shared = Rc::clone(state);
        let mut fonts = false;
        let id = control.open(place, move |ui, _| {
            let context = ui.ctx().clone();
            let mut state = shared.borrow_mut();
            if !fonts {
                theme::install_fonts(&context, state.settings.system_font);
                fonts = true;
            }
            let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
            theme::apply(&context, &palette, &state.settings);
            let Some((_, armed, problem)) = state.power.clone() else {
                return Flow::Close;
            };
            let mut close = context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let chosen = crate::power::draw(ui, &palette, armed, &problem);
            if let Some(choice) = chosen {
                match crate::power::step(choice, armed) {
                    crate::power::Step::CarryOut(choice) => {
                        let done = match choice.command() {
                            Some(command) => {
                                state.link.send(Request::Dispatch(command.to_string()));
                                Ok(())
                            }
                            None => crate::power::ask_machine(choice),
                        };
                        match done {
                            Ok(()) => close = true,
                            Err(why) => {
                                if let Some(power) = state.power.as_mut() {
                                    power.1 = None;
                                    power.2 = why;
                                }
                            }
                        }
                    }
                    crate::power::Step::Arm(choice) => {
                        if let Some(power) = state.power.as_mut() {
                            power.1 = Some(choice);
                            power.2.clear();
                        }
                    }
                }
                context.request_repaint();
            }
            if close {
                state.power = None;
                return Flow::Close;
            }
            Flow::Continue
        });
        state.borrow_mut().power = Some((id, None, String::new()));
    }

    /// Give the window manager a configuration the shell changed (a
    /// workspace's title, what the taskbar shows). It is the shell's own
    /// from this moment, so that a second change made before the window
    /// manager has answered builds on the first; the revision it answers
    /// with is taken in `Event::Applied`, and a change it refuses is told
    /// and the running configuration fetched again.
    fn apply_configuration(state: &mut State, changed: gevrek_model::config::Config) {
        let revision = state.running.as_ref().map_or(0, |(_, revision)| *revision);
        state.link.send(Request::Apply {
            config: Box::new(changed.clone()),
            base_revision: revision,
            client_id: "gevrek-shell".to_string(),
        });
        state.running = Some((changed, revision));
    }

    /// Ask for a workspace's new name, in a small box on the display of
    /// the panel whose tab was pressed. Enter gives the workspace of that
    /// display the name as its title, and leaves the workspaces of that
    /// name on the other displays as they are (`panel::renaming`): it is
    /// written into the configuration, which the window manager takes in
    /// at once and has again when it is started anew. A name that cannot
    /// be is said in the box, which stays.
    ///
    /// `display` is the output the box is put on; `place` the display as
    /// the window manager and the configuration name it, `name` what the
    /// workspace is known by there, `label` what it is shown by now.
    fn ask_workspace_name(
        state: &Rc<RefCell<State>>,
        control: &mut Control,
        display: &str,
        place: gevrek_model::settings::DisplayRef,
        name: &str,
        label: &str,
    ) {
        if let Some((id, ..)) = state.borrow_mut().rename.take() {
            control.close(id);
        }
        let surface = Place {
            name: "gevrek-rename".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width: crate::panel::RENAME_SIZE.0,
            height: crate::panel::RENAME_SIZE.1,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: Some(display.to_string()),
        };
        let key = name.to_string();
        let shared = Rc::clone(state);
        let mut fonts = false;
        let id = control.open(surface, move |ui, _| {
            let context = ui.ctx().clone();
            let mut state = shared.borrow_mut();
            let state = &mut *state;
            if !fonts {
                theme::install_fonts(&context, state.settings.system_font);
                fonts = true;
            }
            let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
            theme::apply(&context, &palette, &state.settings);
            let Some((_, old, mut name, mut problem)) = state.rename.clone() else {
                return Flow::Close;
            };
            let mut close = context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let before = name.clone();
            match crate::panel::rename_box(ui, &palette, &old, &mut name, &problem) {
                Some(true) => {
                    let running = state.running.as_ref().map(|(config, _)| config);
                    match crate::panel::renaming(running, &place, &key, &name) {
                        crate::panel::Renaming::Nothing => close = true,
                        crate::panel::Renaming::Refused(why) => problem = why,
                        crate::panel::Renaming::Apply(changed) => {
                            log::info!(
                                "workspace {key:?} of {} is shown as {:?} from now on",
                                place.name(),
                                name.trim()
                            );
                            apply_configuration(state, *changed);
                            state
                                .link
                                .send(Request::StateAfter(std::time::Duration::from_millis(120)));
                            close = true;
                        }
                    }
                }
                Some(false) => close = true,
                None => {
                    if name != before {
                        problem.clear();
                    }
                }
            }
            if close {
                state.rename = None;
                return Flow::Close;
            }
            if let Some(rename) = state.rename.as_mut() {
                rename.2 = name;
                rename.3 = problem;
            }
            Flow::Continue
        });
        state.borrow_mut().rename =
            Some((id, label.to_string(), label.to_string(), String::new()));
    }

    /// How long the on-screen display stays after the last key.
    const OSD_STAY: std::time::Duration = crate::osd::STAY;

    /// A key of the keyboard's top row: do it, and say what was done.
    fn key_action(state: &Rc<RefCell<State>>, action: &str, control: &mut Control) {
        use gevrek_system::sound::{self, Device};
        let level_of = Osd::sound;
        let player =
            |ask: crate::media::Ask, icon: &'static str, words: &str| match crate::media::tell(ask)
            {
                Some(name) => Osd::words(icon, format!("{words}: {name}")),
                None => Osd::words(icon, "Nothing is playing"),
            };
        let osd = match action {
            "volume_up" => level_of(Device::Speaker, sound::change(Device::Speaker, 5)),
            "volume_down" => level_of(Device::Speaker, sound::change(Device::Speaker, -5)),
            "volume_mute" => level_of(Device::Speaker, sound::toggle_mute(Device::Speaker)),
            "mic_mute" => level_of(Device::Microphone, sound::toggle_mute(Device::Microphone)),
            "media_play_pause" => player(
                crate::media::Ask::PlayPause,
                icons::PLAY_PAUSE,
                "Play or pause",
            ),
            "media_next" => player(crate::media::Ask::Next, icons::SKIP_FORWARD, "Next"),
            "media_previous" => player(crate::media::Ask::Previous, icons::SKIP_BACK, "Previous"),
            "media_stop" => player(crate::media::Ask::Stop, icons::STOP, "Stop"),
            "brightness_up" => Osd::brightness(gevrek_system::brightness::change(5)),
            "brightness_down" => Osd::brightness(gevrek_system::brightness::change(-5)),
            // What the shell does not do yet is left alone, without a word.
            _ => return,
        };
        // (The volume is changed either way; shown unless the
        // configuration leaves that to something else.)
        if action.starts_with("volume_") && !state.borrow().show_volume {
            return;
        }
        show_osd(state, osd, control);
    }

    /// Say something at the bottom of the display for a moment.
    fn show_osd(state: &Rc<RefCell<State>>, osd: Osd, control: &mut Control) {
        let until = Instant::now() + OSD_STAY;
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
                log::debug!("the on-screen display is opened");
                let place = Place {
                    name: "gevrek-osd".to_string(),
                    edge: Edge::Bottom,
                    level: Level::Overlay,
                    width: crate::osd::SIZE.0,
                    height: crate::osd::SIZE.1,
                    reserve: false,
                    keyboard: Keyboard::Never,
                    margin: crate::osd::MARGIN,
                    output: None,
                };
                let shared = Rc::clone(state);
                let mut fonts = false;
                let id = control.open(place, move |ui, _| draw_osd(&shared, ui, &mut fonts));
                state.borrow_mut().osd = Some((id, osd, until));
            }
        }
    }

    fn draw_osd(shared: &Rc<RefCell<State>>, ui: &mut egui::Ui, fonts: &mut bool) -> Flow {
        let context = ui.ctx().clone();
        let mut state = shared.borrow_mut();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let Some((_, osd, until)) = state.osd.clone() else {
            return Flow::Close;
        };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            state.osd = None;
            return Flow::Close;
        }
        context.request_repaint_after(left + std::time::Duration::from_millis(20));
        crate::osd::draw(ui, &palette, &osd);
        Flow::Continue
    }

    use crate::notifications::{height as notes_height, NOTE_WIDTH};

    /// An application asks something through the portal.
    fn from_portal(state: &Rc<RefCell<State>>, asked: crate::portal::Asked, control: &mut Control) {
        use crate::portal::Asked;
        match asked {
            // A notification: on the same cards as any other. Sent again
            // under its key, it takes the place of the one before.
            Asked::Notify {
                key,
                app,
                title,
                body,
                urgency,
                buttons,
                default_action,
                picture,
            } => {
                let replaces = state.borrow().portal_notes.get(&key).copied().unwrap_or(0);
                // As the notification service takes them: a key, its
                // words, the next key; a press on the card is "default".
                let mut actions = Vec::new();
                if default_action.is_some() {
                    actions.extend(["default".to_string(), String::new()]);
                }
                for (action, label) in buttons {
                    actions.extend([action, label]);
                }
                let (answer, number) = std::sync::mpsc::channel();
                from_bus(
                    state,
                    FromBus::Notify {
                        app: app.clone(),
                        replaces,
                        summary: title,
                        body,
                        actions,
                        urgency,
                        expire_ms: -1,
                        picture,
                        silent: false,
                        sound: None,
                        answer,
                    },
                    control,
                );
                if let Ok(id) = number.try_recv() {
                    let mut state = state.borrow_mut();
                    // (Not without end: an application that never takes
                    // its notifications back.)
                    if state.portal_notes.len() > 500 {
                        state.portal_notes.clear();
                        state.portal_ids.clear();
                    }
                    // The application's own name for it is what follows
                    // its id in the key.
                    let name = key
                        .split_once('\n')
                        .map_or("", |(_, name)| name)
                        .to_string();
                    state.portal_notes.insert(key, id);
                    state.portal_ids.insert(id, (app, name, default_action));
                }
            }
            Asked::Withdraw(key) => {
                let known = state.borrow_mut().portal_notes.remove(&key);
                if let Some(id) = known {
                    state.borrow_mut().portal_ids.remove(&id);
                    from_bus(state, FromBus::Close(id), control);
                }
            }
            // The screen kept awake, by whoever asks.
            Asked::Awake(asker, asks) => {
                let mut state = state.borrow_mut();
                if state.awake.is_none() && asks {
                    match crate::inhibit::start() {
                        Ok(awake) => state.awake = Some(awake),
                        Err(error) => log::warn!("the screen is not kept awake: {error}"),
                    }
                }
                if let Some(awake) = state.awake.as_mut() {
                    awake.set(&asker, asks);
                }
            }
            // Screen sharing: the user chooses what, or refuses.
            Asked::Share(ask) => open_share(state, ask, control),
            // A picture of the screen, or a colour picked from it.
            Asked::Picture(ask) => picture_asked(state, ask, control),
            Asked::Colour(ask) => choose_for(state, RegionFor::Colour(ask.answer), control),
            // The pointer and the keyboard: the user allows it, or not.
            Asked::Control(ask) => open_control(state, ask, control),
            // An application's own keys.
            Asked::Shortcuts(ask) => open_shortcuts(state, ask, control),
            Asked::ShortcutsList(ask) => {
                let bound = state.borrow().running.as_ref().map(|(config, _)| {
                    crate::app_shortcuts::bound(&config.desktop.bindings, &ask.app)
                });
                let _ = ask.answer.send_blocking(Some(bound.unwrap_or_default()));
            }
            Asked::ShortcutsConfigure => {
                let studio = std::env::current_exe()
                    .ok()
                    .and_then(|own| Some(own.parent()?.join("gevrek-studio")))
                    .filter(|studio| studio.exists())
                    .unwrap_or_else(|| "gevrek-studio".into());
                if let Err(error) = std::process::Command::new(studio)
                    .args(["--page", "bindings"])
                    .spawn()
                {
                    log::warn!("the bindings could not be opened: {error}");
                }
            }
            // Its application is done with it.
            Asked::ShareEnd(session) => {
                let mut state = state.borrow_mut();
                state.casts.remove(&session);
                if state
                    .share
                    .as_ref()
                    .is_some_and(|share| share.ask.session == session)
                {
                    if let Some(share) = state.share.take() {
                        let _ = share.ask.answer.send_blocking(None);
                        control.close(share.surface);
                    }
                }
            }
            // The wallpaper: the running configuration is asked for, and
            // changed when it comes (`Event::Configuration`).
            Asked::Wallpaper(file) => {
                let mut state = state.borrow_mut();
                state.wanted_wallpaper = Some(file);
                state.link.send(Request::Fetch);
            }
        }
    }

    /// A notification arrived, or its sender took one back.
    fn from_bus(state: &Rc<RefCell<State>>, message: FromBus, control: &mut Control) {
        {
            let mut state = state.borrow_mut();
            match message {
                FromBus::Notify {
                    app,
                    replaces,
                    summary,
                    body,
                    actions,
                    urgency,
                    expire_ms,
                    picture,
                    silent,
                    sound,
                    answer,
                } => {
                    if crate::notifications::is_heard(
                        state.notes_sound,
                        silent,
                        state.notes.quiet,
                        urgency,
                    ) {
                        gevrek_system::sound::play_event(sound.as_deref());
                    }
                    let id = state.notes.push(
                        &app,
                        replaces,
                        &summary,
                        &body,
                        &actions,
                        urgency,
                        expire_ms,
                        Instant::now(),
                    );
                    let _ = answer.send(id);
                    // (After the answer: reading an icon takes a moment.)
                    if let Some(pixels) = picture.and_then(crate::notifications::pixels_of) {
                        state.notes.set_picture(id, pixels);
                    }
                }
                FromBus::Close(id) => {
                    if state.notes.remove(id) {
                        if let Some(to_bus) = &state.to_bus {
                            let _ = to_bus.send(ToBus::Closed(id, Gone::Closed));
                        }
                    }
                }
            }
        }
        show_notes(state, control);
    }

    /// Bring the notifications' surface in line with the stack: open it,
    /// give it the size for what is shown, draw it again.
    fn show_notes(state: &Rc<RefCell<State>>, control: &mut Control) {
        let (surface, height, empty, corner) = {
            let state = state.borrow();
            (
                state.notes_surface,
                notes_height(&state.notes),
                state.notes.is_empty(),
                state.notes_corner,
            )
        };
        match surface {
            Some(id) if control.is_open(id) => {
                if !empty {
                    control.resize(id, NOTE_WIDTH, height);
                }
                control.redraw(id);
            }
            _ if empty => {}
            _ => {
                log::debug!("the notifications' surface is opened");
                let place = Place {
                    name: "gevrek-notifications".to_string(),
                    edge: {
                        use gevrek_model::settings::NotificationCorner as Corner;
                        match corner {
                            Corner::TopRight => Edge::TopRight,
                            Corner::TopLeft => Edge::TopLeft,
                            Corner::BottomRight => Edge::BottomRight,
                            Corner::BottomLeft => Edge::BottomLeft,
                        }
                    },
                    level: Level::Overlay,
                    width: NOTE_WIDTH,
                    height,
                    reserve: false,
                    keyboard: Keyboard::Never,
                    margin: 12,
                    output: None,
                };
                let shared = Rc::clone(state);
                let mut fonts = false;
                let id = control.open(place, move |ui, control| {
                    draw_notes(&shared, ui, control, &mut fonts)
                });
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
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let now = Instant::now();
        let before = state.notes.shown().len();
        // Those whose time is over go, and their senders are told.
        for id in state.notes.expire(now) {
            if let Some(to_bus) = &state.to_bus {
                let _ = to_bus.send(ToBus::Closed(id, Gone::Expired));
            }
        }
        let gone = crate::notifications::draw_cards(ui, &palette, &state.notes);
        let dismissed = gone.is_some();
        if let Some((id, action)) = gone {
            state.notes.remove(id);
            // One that came through the portal: its application is told
            // there, in its own words.
            if let Some((app, name, default)) = state.portal_ids.remove(&id) {
                state.portal_notes.retain(|_, known| *known != id);
                let chosen = match action.as_deref() {
                    Some("default") => default,
                    other => other.map(str::to_string),
                };
                if let (Some(chosen), Some(portal)) = (chosen, &state.portal) {
                    portal.chose(&app, &name, &chosen);
                }
            } else if let Some(to_bus) = &state.to_bus {
                if let Some(key) = action {
                    let _ = to_bus.send(ToBus::Action(id, key));
                }
                let _ = to_bus.send(ToBus::Closed(id, Gone::Dismissed));
            }
        }
        if state.notes.is_empty() {
            state.notes_surface = None;
            return Flow::Close;
        }
        if state.notes.shown().len() != before || dismissed {
            if let Some(id) = state.notes_surface {
                control.resize(id, NOTE_WIDTH, notes_height(&state.notes));
            }
            context.request_repaint();
        }
        // Drawn again when the next one's time is over.
        if let Some(next) = state.notes.next_expiry() {
            context.request_repaint_after(
                next.saturating_duration_since(now) + std::time::Duration::from_millis(30),
            );
        }
        Flow::Continue
    }

    /// Open the launcher, or close it when it is open.
    /// The window switcher on its key: up, or (pressed again while it is
    /// up without its modifier held) away.
    fn toggle_switcher(state: &Rc<RefCell<State>>, control: &mut Control) {
        // Its key again while it is up (the modifier still held, Tab
        // pressed once more: the key is the window manager's, the surface
        // never sees it): on to the next window.
        let open = {
            let mut state = state.borrow_mut();
            state.switcher.as_mut().map(|(id, switcher)| {
                switcher.step(1);
                *id
            })
        };
        if let Some(id) = open {
            control.redraw(id);
            return;
        }
        log::debug!("the switcher is opened");
        let entries = {
            let state = state.borrow();
            let entries = state
                .wm
                .as_ref()
                .map(crate::switcher::entries)
                .unwrap_or_default();
            state.used.order(entries)
        };
        let (width, height) = crate::switcher::size(entries.len());
        let place = Place {
            name: "gevrek-switcher".to_string(),
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
        let mut icons_seen: std::collections::HashMap<String, Option<egui::TextureHandle>> =
            Default::default();
        let id = control.open(place, move |ui, _| {
            let context = ui.ctx().clone();
            let mut state = shared.borrow_mut();
            if !fonts {
                theme::install_fonts(&context, state.settings.system_font);
                fonts = true;
            }
            let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
            theme::apply(&context, &palette, &state.settings);
            let asked = {
                let Some((_, switcher)) = state.switcher.as_mut() else {
                    return Flow::Close;
                };
                crate::switcher::draw(ui, &palette, switcher, &mut |context, class| {
                    // A window's class is usually its application's icon.
                    icons_seen
                        .entry(class.to_string())
                        .or_insert_with(|| {
                            let file = gevrek_system::icons::find(class, 64)?;
                            let picture = gevrek_system::icons::load(&file, 64)?;
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [picture.width as usize, picture.height as usize],
                                &picture.pixels,
                            );
                            Some(context.load_texture(
                                class.to_string(),
                                image,
                                egui::TextureOptions::LINEAR,
                            ))
                        })
                        .clone()
                })
            };
            if let Some(window) = asked.go {
                let command = serde_json::json!({
                    "command": "focus_window",
                    "selector": {"kind": "handle", "value": window},
                });
                state.link.send(Request::Dispatch(command.to_string()));
            }
            if asked.close {
                state.switcher = None;
                Flow::Close
            } else {
                Flow::Continue
            }
        });
        state.borrow_mut().switcher = Some((id, crate::switcher::Switcher::new(entries)));
        // The pictures of the windows, taken beside the switcher: it is
        // up at once, and shows them when they are there.
        if let Some(sender) = state.borrow().verdicts.clone() {
            let _ = std::thread::Builder::new()
                .name("gevrek-thumbs".to_string())
                .spawn(move || {
                    sender.send(Message::Thumbs(crate::thumbs::take_all(640)));
                });
        }
    }

    /// Begin the tutorial (the lesson to take next), or nothing when it is
    /// taken already: its card comes up at the bottom of the display, over
    /// everything, and never has the keyboard.
    fn tutorial_start(state: &Rc<RefCell<State>>, control: &mut Control) {
        if state.borrow().tutorial.is_some() {
            return;
        }
        let now = Instant::now();
        let begun = crate::tutoring::Tutoring::begin(
            None,
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
            name: "gevrek-coach".to_string(),
            edge: Edge::Bottom,
            level: Level::Overlay,
            width: crate::coach::SIZE.0,
            height: crate::coach::SIZE.1,
            reserve: false,
            keyboard: Keyboard::Never,
            margin: crate::tutoring::CARD_MARGIN as i32,
            output: None,
        };
        let shared = Rc::clone(state);
        let mut fonts = false;
        let id = control.open(place, move |ui, _| {
            use crate::tutoring::Shown;
            let context = ui.ctx().clone();
            let mut state = shared.borrow_mut();
            let state = &mut *state;
            if !fonts {
                theme::install_fonts(&context, state.settings.system_font);
                fonts = true;
            }
            let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
            theme::apply(&context, &palette, &state.settings);
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
                    let keys = match (step.act, state.running.as_ref()) {
                        (gevrek_tutorial::lessons::Act::Command(command), Some((config, _))) => {
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
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
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
                        crate::tutoring::tidy(
                        tutoring.before.clone(),
                        windows,
                        tutoring.own_windows(),
                        state.link.teller(),
                    );
                    }
                }
            }
        }
    }

    fn toggle_launcher(state: &Rc<RefCell<State>>, control: &mut Control) {
        let open = state.borrow_mut().launcher.take();
        if let Some((id, _)) = open {
            log::debug!("the launcher is closed: asked again");
            control.close(id);
            return;
        }
        log::debug!("the launcher is opened");
        // On the display that has the keyboard: what is chosen in it
        // opens on that display too, wherever the pointer and the focus
        // are by the time its window comes.
        let shown_on: Option<String> = state.borrow().wm.as_ref().and_then(|wm| {
            let monitors = wm["monitors"].as_array()?;
            monitors
                .iter()
                .find(|monitor| monitor["is_focused"].as_bool().unwrap_or(false))
                .and_then(crate::panel::name_of)
                .map(str::to_string)
        });
        let place = Place {
            name: "gevrek-launcher".to_string(),
            edge: Edge::Centre,
            level: Level::Overlay,
            width: crate::launcher::SIZE.0,
            height: crate::launcher::SIZE.1,
            reserve: false,
            keyboard: Keyboard::Exclusive,
            margin: 0,
            output: shown_on.clone(),
        };
        // (Looked for once, when the launcher is opened.)
        let gamescope = crate::launcher::gamescope_is_there();
        let shared = Rc::clone(state);
        let mut fonts = false;
        // The icons that were looked for, by name (`None`: there is none).
        let mut icons_seen: std::collections::HashMap<String, Option<egui::TextureHandle>> =
            Default::default();
        let id = control.open(place, move |ui, _| {
            let context = ui.ctx().clone();
            let mut state = shared.borrow_mut();
            if !fonts {
                theme::install_fonts(&context, state.settings.system_font);
                fonts = true;
            }
            let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
            theme::apply(&context, &palette, &state.settings);
            let asked = {
                let Some((_, launcher)) = state.launcher.as_mut() else {
                    return Flow::Close;
                };
                let hint = format!(
                    "{}  An application · > a command · : what was copied{}",
                    icons::MAGNIFYING_GLASS,
                    if gamescope {
                        " · Shift+Enter: in gamescope"
                    } else {
                        ""
                    }
                );
                crate::launcher::draw(ui, &palette, launcher, &hint, &mut |context, entry| {
                    // Its icon, looked for once.
                    icons_seen
                        .entry(entry.icon.clone())
                        .or_insert_with(|| {
                            let file = gevrek_system::icons::find(&entry.icon, 64)?;
                            let picture = gevrek_system::icons::load(&file, 64)?;
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [picture.width as usize, picture.height as usize],
                                &picture.pixels,
                            );
                            Some(context.load_texture(
                                entry.icon.clone(),
                                image,
                                egui::TextureOptions::LINEAR,
                            ))
                        })
                        .clone()
                })
            };
            let mut flow = if asked.close {
                Flow::Close
            } else {
                Flow::Continue
            };
            if let Some(mut entry) = asked.run {
                // Shift+Enter: inside gamescope, where that is installed
                // (by its command; not over the bus, which would start it
                // outside).
                if asked.in_gamescope && gamescope {
                    if let Some(command) = crate::launcher::in_gamescope(&entry.command) {
                        entry.command = command;
                        entry.activate = None;
                    }
                }
                match (&entry.copy, &state.clipboard) {
                    (Some(text), Some(clipboard)) => clipboard.put(text),
                    (Some(_), None) => {}
                    (None, _) => {
                        // (Its window is to open on the launcher's display.
                        // Not for a window that is there already and is
                        // only gone to: no new one comes.)
                        if let (Some(output), true) = (&shown_on, entry.app.is_some()) {
                            let expect =
                                serde_json::json!({"command": "expect_window", "output": output});
                            state.link.send(Request::Dispatch(expect.to_string()));
                        }
                        match &entry.activate {
                            // Over the bus where the application wants that;
                            // its command where the bus does not answer.
                            Some(activation) => {
                                let tell = state.link.teller();
                                let command = entry.command.clone();
                                crate::activate::start(activation.clone(), move || {
                                    if !command.is_null() {
                                        tell(Request::Dispatch(command.to_string()));
                                    }
                                });
                            }
                            None if entry.command.is_null() => {}
                            None => state
                                .link
                                .send(Request::Dispatch(entry.command.to_string())),
                        }
                    }
                }
                if let Some(app) = &entry.app {
                    let path = crate::launcher::usage_file();
                    crate::launcher::Usage::load(&path).bump(app, &path);
                }
                flow = Flow::Close;
            }
            if flow == Flow::Close {
                state.launcher = None;
            }
            flow
        });
        let mut state = state.borrow_mut();
        let terminal = state.terminal.clone();
        // The windows that are open, to switch to by typing.
        let windows = state
            .wm
            .as_ref()
            .map(crate::launcher::open_windows)
            .unwrap_or_default();
        // (Kept from the last time, and read again only where a folder
        // of entries changed: with many applications reading them all
        // took longer than the launcher should take to come up.)
        let apps = state.apps.installed();
        let launcher = Launcher::new(apps, &terminal)
            .with(
                windows,
                crate::launcher::Usage::load(&crate::launcher::usage_file()),
            )
            .with_copied(
                state
                    .clipboard
                    .as_ref()
                    .map(|clipboard| {
                        clipboard
                            .history
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .texts()
                            .to_vec()
                    })
                    .unwrap_or_default(),
            );
        state.launcher = Some((id, launcher));
    }
}
