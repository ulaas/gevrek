//! The control centre: the things that are set often, in one place at
//! the side of the display. How loud it is and which device plays, how
//! bright the display is (on a machine where that can be set), do not
//! disturb, the night light, what is playing, a recording of the screen,
//! and the way to the lock and the power menu.
//!
//! It shows what the shell knows anyway (the panel's modules read the
//! same) and asks what the keys ask; nothing here is a second way of
//! doing a thing.

use gevrek_system::sound::Output;
use gevrek_ui::theme::{self, icons};

/// How wide the control centre is, in points.
pub const WIDTH: u32 = 380;

/// What the control centre shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    /// How loud, in percent of full, and whether it is muted; `None`:
    /// there is no sound server.
    pub volume: Option<(u32, bool)>,
    /// The devices to play on.
    pub outputs: Vec<Output>,
    /// The devices to record from.
    pub inputs: Vec<Output>,
    /// The programs that play now, each with how loud it is by itself.
    pub streams: Vec<gevrek_system::sound::Stream>,
    /// How bright the machine's own display is; `None`: it has none
    /// whose brightness can be set.
    pub brightness: Option<u32>,
    pub quiet: bool,
    pub night_light: bool,
    pub recording: bool,
    /// The machine's power profiles, where it has a service for them.
    pub profiles: Option<gevrek_system::power_profile::Profiles>,
    /// The network the machine is on: its name, or nothing said where
    /// nobody can be asked.
    pub network: Option<gevrek_system::status::Network>,
    /// What a player has on, where there is a player.
    pub playing: Option<crate::media::Playing>,
    /// Bluetooth, on a machine that has it.
    pub bluetooth: Option<gevrek_system::bluetooth::Bluetooth>,
    /// The wireless networks, on a machine that has a radio for them.
    pub wifi: Option<gevrek_system::wifi::Wifi>,
    /// The wireless network that is being joined now.
    pub joining: Option<String>,
    /// The system was asked to look for wireless networks and has not
    /// been asked often enough yet to say that there are none.
    pub wifi_looking: bool,
}

/// Something the control centre asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Put it away.
    Close,
    /// Louder or quieter by so many percent of full.
    Volume(i32),
    Mute,
    /// Play on this device.
    Output(u32),
    /// Record from this device.
    Input(u32),
    /// Make one program so loud, in percent of full.
    Stream(u32, u32),
    /// Mute one program, or let it be heard again.
    StreamMute(u32),
    /// Brighter or darker by so many percent of the most.
    Brightness(i32),
    Quiet(bool),
    /// Switch the night light on, or off.
    NightLight,
    Media(crate::media::Ask),
    /// Begin a recording of the screen, or end the one that goes on.
    Record,
    Lock,
    PowerMenu,
    /// Make this the machine's power profile.
    Profile(String),
    /// Switch bluetooth's radio on, or off.
    Bluetooth(bool),
    /// Connect the device with this address, or let it go.
    BluetoothDevice(String, bool),
    /// Switch the wireless radio on, or off.
    Wifi(bool),
    /// Join a wireless network: its name, whether the machine keeps its
    /// settings, and the password typed for one it does not keep.
    WifiJoin {
        name: String,
        known: bool,
        password: Option<String>,
    },
}

/// The sliders' own places while they are held: the sound and the
/// brightness are set when a slider is let go, not at every place it
/// passes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Held {
    /// The wireless network a password is being typed for, and what is
    /// typed so far.
    pub wifi_password: Option<(String, String)>,
    pub volume: Option<f32>,
    /// The program whose slider is held, and where.
    pub stream: Option<(u32, f32)>,
    pub brightness: Option<f32>,
    /// The level that was asked for and has not been seen yet: the
    /// system takes a moment, and until then the slider stays where it
    /// was put instead of going back to the old level and forth again.
    pub volume_asked: Asked,
    pub brightness_asked: Asked,
    /// The switches of the wireless radio and of bluetooth, while the
    /// system follows a press.
    pub wifi_switched: Switched,
    pub bluetooth_switched: Switched,
    /// What a tile has more of, open under the tiles.
    pub open: Option<Detail>,
    /// The lists of the devices to play on and to record from are open.
    pub outputs_open: bool,
    pub inputs_open: bool,
    /// The programs that play, where there are several, are open.
    pub streams_open: bool,
}

/// A switch that was pressed: what it was put to, and when. The system
/// takes a while to follow (a radio comes on in seconds), and says the
/// old state meanwhile: the switch shows what was asked for until the
/// system is there, instead of going back and forth, and a sign beside
/// it turns while that lasts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Switched(Option<(bool, std::time::Instant)>);

/// A switch that was pressed is waited for this long; then what the
/// system says counts again (it may have refused).
const SWITCH_HOLDS: std::time::Duration = std::time::Duration::from_secs(12);

impl Switched {
    /// What the switch shows: what was asked for while the system has
    /// not caught up, else the system's own state.
    pub fn shown(&mut self, system: bool, now: std::time::Instant) -> bool {
        match self.0 {
            Some((asked, since)) if asked != system && now.duration_since(since) < SWITCH_HOLDS => {
                asked
            }
            _ => {
                self.0 = None;
                system
            }
        }
    }

    pub fn ask(&mut self, on: bool, now: std::time::Instant) {
        self.0 = Some((on, now));
    }

    /// The system has not caught up yet.
    pub fn waiting(&self) -> bool {
        self.0.is_some()
    }
}

/// A level that was asked for, and when.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Asked(Option<(u32, std::time::Instant)>);

/// A level that was asked for is waited for this long; then what the
/// system says counts again (something else may have set it).
const ASKED_HOLDS: std::time::Duration = std::time::Duration::from_millis(1500);

impl Asked {
    /// The level to go by: the one asked for while the system has not
    /// caught up, else the system's own.
    pub fn level(&mut self, system: u32, now: std::time::Instant) -> u32 {
        match self.0 {
            Some((asked, since))
                if asked != system && now.saturating_duration_since(since) < ASKED_HOLDS =>
            {
                asked
            }
            _ => {
                self.0 = None;
                system
            }
        }
    }

    pub fn ask(&mut self, level: u32, now: std::time::Instant) {
        self.0 = Some((level, now));
    }

    pub fn waiting(&self) -> bool {
        self.0.is_some()
    }
}

/// The step from where a level is to where a slider was let go, in whole
/// percent; nothing when that is no step.
pub fn step_to(now: u32, wanted: f32) -> Option<i32> {
    let step = wanted.round() as i32 - now as i32;
    (step != 0).then_some(step)
}

/// What has more to it than a tile says, open under the tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// The wireless networks in reach.
    Wifi,
    /// The bluetooth devices the machine knows.
    Bluetooth,
    /// The machine's power profiles.
    Profiles,
}

/// How many lines of a list are seen before it is scrolled.
const LINES: usize = 6;

/// How high the tiles are, and how far apart.
const TILE: f32 = 56.0;
const BETWEEN: f32 = 8.0;

/// One tile of the control centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Switch {
    Wifi,
    /// The network as far as it is known, where no radio can be asked:
    /// said, not switched.
    Network,
    Bluetooth,
    Quiet,
    NightLight,
    Profiles,
    Record,
}

/// The tiles this machine has, in their order: each with its sign, its
/// name, how it stands, whether it is on, what it opens, and whether the
/// system is still following a press.
fn tiles_of(
    facts: &Facts,
    held: &mut Held,
    now: std::time::Instant,
) -> Vec<(Switch, &'static str, &'static str, String, bool, Option<Detail>, bool)> {
    let mut tiles = Vec::new();
    match (&facts.wifi, &facts.network) {
        (Some(wifi), _) => {
            let on = held.wifi_switched.shown(wifi.enabled, now);
            let in_use = wifi.networks.iter().find(|network| network.in_use);
            let state = match (&facts.joining, in_use) {
                _ if !on => "Off".to_string(),
                (Some(joining), _) => format!("Joining {joining}…"),
                (None, Some(network)) => network.name.clone(),
                (None, None) if facts.wifi_looking => "Looking for networks…".to_string(),
                // (On a cable meanwhile: said, so that the tile does not
                // read as if the machine were on no network at all.)
                (None, None) => match &facts.network {
                    Some(network) if network.connected && !network.wireless => {
                        "On a cable".to_string()
                    }
                    _ => "Not on a network".to_string(),
                },
            };
            let sign = if on { icons::WIFI_HIGH } else { icons::WIFI_SLASH };
            tiles.push((
                Switch::Wifi,
                sign,
                "Wireless",
                state,
                on,
                Some(Detail::Wifi),
                held.wifi_switched.waiting(),
            ));
        }
        (None, Some(network)) => {
            let (sign, state) = match (network.connected, network.wireless) {
                (true, true) => (icons::WIFI_HIGH, network.name.as_str()),
                (true, false) => (icons::PLUGS_CONNECTED, network.name.as_str()),
                (false, _) => (icons::WIFI_SLASH, "Not on a network"),
            };
            let state = if state.is_empty() { "On a network" } else { state };
            tiles.push((
                Switch::Network,
                sign,
                "Network",
                state.to_string(),
                network.connected,
                None,
                false,
            ));
        }
        (None, None) => {}
    }
    if let Some(bluetooth) = &facts.bluetooth {
        let on = held.bluetooth_switched.shown(bluetooth.powered, now);
        let connected: Vec<&str> = bluetooth
            .devices
            .iter()
            .filter(|device| device.connected)
            .map(|device| device.name.as_str())
            .collect();
        let state = match (on, connected.as_slice()) {
            (false, _) => "Off".to_string(),
            (true, []) => "No device connected".to_string(),
            (true, names) => names.join(", "),
        };
        tiles.push((
            Switch::Bluetooth,
            if connected.is_empty() {
                icons::BLUETOOTH
            } else {
                icons::BLUETOOTH_CONNECTED
            },
            "Bluetooth",
            state,
            on,
            Some(Detail::Bluetooth),
            held.bluetooth_switched.waiting(),
        ));
    }
    tiles.push((
        Switch::Quiet,
        if facts.quiet {
            icons::BELL_SLASH
        } else {
            icons::BELL
        },
        "Do not disturb",
        if facts.quiet {
            "Notifications are kept"
        } else {
            "Off"
        }
        .to_string(),
        facts.quiet,
        None,
        false,
    ));
    tiles.push((
        Switch::NightLight,
        icons::MOON,
        "Night light",
        if facts.night_light { "On" } else { "Off" }.to_string(),
        facts.night_light,
        None,
        false,
    ));
    if let Some(profiles) = &facts.profiles {
        tiles.push((
            Switch::Profiles,
            icons::LIGHTNING,
            "Power",
            gevrek_system::power_profile::label(&profiles.active).to_string(),
            false,
            Some(Detail::Profiles),
            false,
        ));
    }
    tiles.push((
        Switch::Record,
        if facts.recording {
            icons::STOP
        } else {
            icons::RECORD
        },
        "Record screen",
        if facts.recording {
            "Recording: press to end"
        } else {
            "Off"
        }
        .to_string(),
        facts.recording,
        None,
        false,
    ));
    tiles
}

/// What is open under the tiles: the networks in reach, the devices
/// bluetooth knows, or the power profiles; each a list that scrolls
/// where it is long.
fn detail(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    facts: &Facts,
    held: &mut Held,
    asked: &mut Vec<Ask>,
) {
    use gevrek_ui::controls;
    let Some(open) = held.open else {
        return;
    };
    let said = |ui: &mut egui::Ui, words: &str| {
        ui.label(egui::RichText::new(words).size(13.0).color(palette.muted));
    };
    egui::Frame::new()
        .fill(palette.control)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(12)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            match open {
                Detail::Wifi => {
                    let Some(wifi) = &facts.wifi else {
                        return;
                    };
                    if !wifi.enabled {
                        said(ui, "The radio is off.");
                        return;
                    }
                    // (The way to switch it off, since a press on the
                    // tile opens this list.)
                    if controls::choice_row(ui, Some(icons::WIFI_SLASH), "Switch wireless off", None, false)
                        .clicked()
                    {
                        held.wifi_switched.ask(false, std::time::Instant::now());
                        asked.push(Ask::Wifi(false));
                        held.wifi_password = None;
                        held.open = None;
                        return;
                    }
                    if wifi.networks.is_empty() {
                        ui.horizontal(|ui| {
                            if facts.wifi_looking {
                                ui.add(egui::Spinner::new().size(13.0));
                            }
                            said(
                                ui,
                                if facts.wifi_looking {
                                    "Looking for networks…"
                                } else {
                                    "No network in reach"
                                },
                            );
                        });
                        return;
                    }
                    controls::list_under(ui, "networks", wifi.networks.len(), LINES, |ui| {
                        for network in &wifi.networks {
                            let sign = match network.signal {
                                0..=33 => icons::WIFI_LOW,
                                34..=66 => icons::WIFI_MEDIUM,
                                _ => icons::WIFI_HIGH,
                            };
                            let joining = facts.joining.as_deref() == Some(network.name.as_str());
                            let about = if joining {
                                Some("joining…")
                            } else if network.secured && !network.known {
                                Some(icons::LOCK)
                            } else {
                                None
                            };
                            let hint = if network.in_use {
                                "The machine is on this network."
                            } else if network.secured && !network.known {
                                "A press asks for its password."
                            } else {
                                "A press joins it."
                            };
                            let row = controls::choice_row(
                                ui,
                                Some(sign),
                                &network.name,
                                about,
                                network.in_use,
                            )
                            .on_hover_text(hint);
                            if row.clicked() && !network.in_use && !joining {
                                if network.secured && !network.known {
                                    held.wifi_password =
                                        Some((network.name.clone(), String::new()));
                                } else {
                                    held.wifi_password = None;
                                    asked.push(Ask::WifiJoin {
                                        name: network.name.clone(),
                                        known: network.known,
                                        password: None,
                                    });
                                }
                            }
                            // The password, under the network it is for.
                            let mut done = false;
                            if let Some((_, typed)) = held
                                .wifi_password
                                .as_mut()
                                .filter(|(name, _)| *name == network.name)
                            {
                                ui.horizontal(|ui| {
                                    let field = ui.add(
                                        egui::TextEdit::singleline(typed)
                                            .password(true)
                                            .hint_text("Password")
                                            .desired_width(ui.available_width() - 70.0),
                                    );
                                    // (It is the one place to type in
                                    // here: it has the keyboard for as
                                    // long as it is shown, and Enter is
                                    // its own.)
                                    field.request_focus();
                                    let entered =
                                        ui.input(|input| input.key_pressed(egui::Key::Enter));
                                    if (ui.button("Join").clicked() || entered) && !typed.is_empty()
                                    {
                                        asked.push(Ask::WifiJoin {
                                            name: network.name.clone(),
                                            known: false,
                                            password: Some(typed.clone()),
                                        });
                                        done = true;
                                    }
                                });
                            }
                            if done {
                                held.wifi_password = None;
                            }
                        }
                    });
                }
                Detail::Bluetooth => {
                    let Some(bluetooth) = &facts.bluetooth else {
                        return;
                    };
                    if !bluetooth.powered {
                        said(ui, "The radio is off.");
                        return;
                    }
                    if controls::choice_row(ui, Some(icons::BLUETOOTH_SLASH), "Switch bluetooth off", None, false)
                        .clicked()
                    {
                        held.bluetooth_switched.ask(false, std::time::Instant::now());
                        asked.push(Ask::Bluetooth(false));
                        held.open = None;
                        return;
                    }
                    if bluetooth.devices.is_empty() {
                        said(ui, "The machine knows no device yet.");
                        return;
                    }
                    controls::list_under(ui, "devices", bluetooth.devices.len(), LINES, |ui| {
                        for device in &bluetooth.devices {
                            let row = controls::choice_row(
                                ui,
                                Some(if device.connected {
                                    icons::BLUETOOTH_CONNECTED
                                } else {
                                    icons::BLUETOOTH
                                }),
                                &device.name,
                                None,
                                device.connected,
                            )
                            .on_hover_text(if !gevrek_system::bluetooth::can_connect() {
                                // (Windows lets no program connect a
                                // device: its own page does.)
                                "A press opens Windows' bluetooth page, where it is connected or let go."
                            } else if device.connected {
                                "Connected. A press lets it go."
                            } else {
                                "A press connects it."
                            });
                            if row.clicked() {
                                asked.push(Ask::BluetoothDevice(
                                    device.address.clone(),
                                    !device.connected,
                                ));
                            }
                        }
                    });
                }
                Detail::Profiles => {
                    let Some(profiles) = &facts.profiles else {
                        return;
                    };
                    for profile in &profiles.all {
                        let chosen = *profile == profiles.active;
                        let row = controls::choice_row(
                            ui,
                            None,
                            gevrek_system::power_profile::label(profile),
                            None,
                            chosen,
                        );
                        if row.clicked() && !chosen {
                            asked.push(Ask::Profile(profile.clone()));
                            // (Chosen: the list has done what it was
                            // opened for.)
                            held.open = None;
                        }
                    }
                }
            }
        });
}

/// The devices of one kind (to play on, to record from): the one in use
/// in a line, and all of them under it when that is pressed. Returns the
/// device that was chosen.
fn devices(
    ui: &mut egui::Ui,
    of: &str,
    list: &str,
    devices: &[Output],
    open: &mut bool,
) -> Option<u32> {
    use gevrek_ui::controls;
    if devices.len() < 2 {
        *open = false;
        return None;
    }
    let in_use = devices
        .iter()
        .find(|device| device.chosen)
        .map_or("none", |device| device.name.as_str());
    if controls::chosen_line(ui, of, in_use, *open).clicked() {
        *open = !*open;
    }
    if !*open {
        return None;
    }
    let mut picked = None;
    controls::list_under(ui, list, devices.len(), LINES, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for device in devices {
            let row = controls::choice_row(ui, None, &device.name, None, device.chosen);
            if row.clicked() && !device.chosen {
                picked = Some(device.id);
            }
        }
    });
    // (Chosen: the list has done what it was opened for.)
    if picked.is_some() {
        *open = false;
    }
    picked
}

/// Draw the control centre. What was asked for in this frame.
///
/// From the top: its name and the way out; what is on or off, as tiles
/// (the radios, do not disturb, the night light, the power profile, a
/// recording), with what a tile has more of open under them; the sound
/// (how loud, where it plays, the programs that play, where it records
/// from); the display's brightness; what is playing; and at the foot,
/// always in sight, the lock and the power menu. What does not fit is
/// scrolled: the foot and the head stay.
pub fn draw(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    facts: &Facts,
    held: &mut Held,
    look: &crate::frost::Look,
) -> Vec<Ask> {
    use gevrek_ui::controls::{self, Tile};
    use gevrek_ui::widgets;
    let mut asked = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            // The ground (frosted glass, or plain), where it is in this
            // frame; what follows stands on it.
            let card = look.ground(ui, palette, 18);
            let builder = egui::UiBuilder::new().max_rect(card.shrink(18.0));
            ui.scope_builder(builder, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.label(
                        // The desktop's own name: this is where it is set.
                        egui::RichText::new("GEVREK")
                            .strong()
                            .size(15.0)
                            .extra_letter_spacing(3.5)
                            .color(palette.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::brand_icon_button(
                            ui,
                            gevrek_ui::brand_icons::BrandIcon::Close,
                            icons::X,
                            "Put it away (Escape)",
                        )
                        .clicked()
                        {
                            asked.push(Ask::Close);
                        }
                    });
                });

                // The foot is kept room for; what is between scrolls.
                let foot = 30.0 + 2.0 * ui.spacing().item_spacing.y + 6.0;
                let room = (ui.available_height() - foot).max(120.0);
                // (The bar it is scrolled by lies in the card's margin, not
                // over what is scrolled: the part that scrolls reaches that
                // much further to the right than what is in it.)
                let inside = ui.available_width();
                let part = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(inside + 12.0, room),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(part), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("the control centre")
                    .max_height(room)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(inside);
                        ui.spacing_mut().item_spacing.y = 8.0;
                        let now = std::time::Instant::now();

                        // What is on and what is off.
                        let tiles = tiles_of(facts, held, now);
                        let wide = (ui.available_width() - BETWEEN) / 2.0;
                        for pair in tiles.chunks(2) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = BETWEEN;
                                for (which, icon, name, state, on, more, busy) in pair {
                                    let pressed = controls::tile(
                                        ui,
                                        egui::vec2(wide, TILE),
                                        &Tile {
                                            icon,
                                            name,
                                            state,
                                            on: *on,
                                            more: more.map(|detail| held.open == Some(detail)),
                                            busy: *busy,
                                        },
                                    );
                                    if let (true, Some(detail)) = (pressed.more, more) {
                                        held.open = (held.open != Some(*detail)).then_some(*detail);
                                    }
                                    if !pressed.switched {
                                        continue;
                                    }
                                    match which {
                                        // (What is chosen is seen on the
                                        // tile; a press brings the list to
                                        // choose from. A radio that is on is
                                        // not switched off by a press meant
                                        // for its list: the list's first
                                        // line does that. One that is off
                                        // is switched on, and its list
                                        // comes with it.)
                                        Switch::Wifi => {
                                            if *on {
                                                held.open = (held.open != Some(Detail::Wifi))
                                                    .then_some(Detail::Wifi);
                                            } else {
                                                held.wifi_switched.ask(true, now);
                                                asked.push(Ask::Wifi(true));
                                                held.open = Some(Detail::Wifi);
                                            }
                                        }
                                        Switch::Bluetooth => {
                                            if *on {
                                                held.open = (held.open != Some(Detail::Bluetooth))
                                                    .then_some(Detail::Bluetooth);
                                            } else {
                                                held.bluetooth_switched.ask(true, now);
                                                asked.push(Ask::Bluetooth(true));
                                                held.open = Some(Detail::Bluetooth);
                                            }
                                        }
                                        Switch::Quiet => asked.push(Ask::Quiet(!on)),
                                        Switch::NightLight => asked.push(Ask::NightLight),
                                        Switch::Record => asked.push(Ask::Record),
                                        // (A choice, not a switch: a press
                                        // opens it.)
                                        Switch::Profiles => {
                                            held.open = (held.open != Some(Detail::Profiles))
                                                .then_some(Detail::Profiles);
                                        }
                                        Switch::Network => {}
                                    }
                                }
                            });
                        }
                        // (A password that is being typed keeps its list open.)
                        if held.wifi_password.is_some() {
                            held.open = Some(Detail::Wifi);
                        }
                        detail(ui, palette, facts, held, &mut asked);

                        // The sound.
                        ui.add_space(2.0);
                        widgets::caption(ui, "Sound");
                        match facts.volume {
                            Some((percent, muted)) => {
                                // (Where it was put, until the system has it there.)
                                let percent = held.volume_asked.level(percent, now);
                                if held.volume_asked.waiting() {
                                    ui.ctx().request_repaint_after(
                                        std::time::Duration::from_millis(100),
                                    );
                                }
                                let mut value = held.volume.unwrap_or(percent as f32);
                                let said = format!("{} %", value.round() as u32);
                                let moved = controls::level(
                                    ui,
                                    crate::panel::sound_icon(percent, muted),
                                    if muted { "Let it be heard" } else { "Mute" },
                                    None,
                                    &mut value,
                                    0.0..=100.0,
                                    Some(&said),
                                );
                                if moved.sign {
                                    asked.push(Ask::Mute);
                                }
                                if moved.slider.dragged() {
                                    held.volume = Some(value);
                                } else if moved.slider.drag_stopped() || moved.slider.changed() {
                                    held.volume = None;
                                    if let Some(step) = step_to(percent, value) {
                                        held.volume_asked.ask(value.round() as u32, now);
                                        asked.push(Ask::Volume(step));
                                    }
                                }
                                // Which device plays, where there is a choice.
                                if let Some(device) = devices(
                                    ui,
                                    "Plays on",
                                    "outputs",
                                    &facts.outputs,
                                    &mut held.outputs_open,
                                ) {
                                    asked.push(Ask::Output(device));
                                }
                                // The programs that play, each by itself:
                                // one in its own line; several behind one
                                // line that names them and opens them.
                                if facts.streams.len() < 2 {
                                    held.streams_open = false;
                                } else {
                                    let names = facts
                                        .streams
                                        .iter()
                                        .map(|stream| stream.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    if controls::chosen_line(ui, "Programs", &names, held.streams_open)
                                        .clicked()
                                    {
                                        held.streams_open = !held.streams_open;
                                    }
                                }
                                let shown = facts.streams.len() < 2 || held.streams_open;
                                for stream in facts.streams.iter().filter(|_| shown) {
                                    let held_here = held
                                        .stream
                                        .filter(|(id, _)| *id == stream.id)
                                        .map(|(_, value)| value);
                                    let mut value =
                                        held_here.unwrap_or(stream.level.percent.min(100) as f32);
                                    let moved = controls::level(
                                        ui,
                                        crate::panel::sound_icon(
                                            stream.level.percent,
                                            stream.level.muted,
                                        ),
                                        if stream.level.muted {
                                            "Let it be heard"
                                        } else {
                                            "Mute this program"
                                        },
                                        Some(&stream.name),
                                        &mut value,
                                        0.0..=100.0,
                                        None,
                                    );
                                    if moved.sign {
                                        asked.push(Ask::StreamMute(stream.id));
                                    }
                                    if moved.slider.dragged() {
                                        held.stream = Some((stream.id, value));
                                    } else if moved.slider.drag_stopped() || moved.slider.changed()
                                    {
                                        held.stream = None;
                                        if value.round() as u32 != stream.level.percent {
                                            asked
                                                .push(Ask::Stream(stream.id, value.round() as u32));
                                        }
                                    }
                                }
                                // Which device records, where there is a choice.
                                if let Some(device) = devices(
                                    ui,
                                    "Records from",
                                    "inputs",
                                    &facts.inputs,
                                    &mut held.inputs_open,
                                ) {
                                    asked.push(Ask::Input(device));
                                }
                            }
                            None => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}  No sound server answers",
                                        icons::SPEAKER_X
                                    ))
                                    .color(palette.muted),
                                );
                            }
                        }

                        // The brightness, on a machine that has one to set.
                        if let Some(percent) = facts.brightness {
                            ui.add_space(2.0);
                            widgets::caption(ui, "Display");
                            let percent = held.brightness_asked.level(percent, now);
                            if held.brightness_asked.waiting() {
                                ui.ctx()
                                    .request_repaint_after(std::time::Duration::from_millis(100));
                            }
                            let mut value = held.brightness.unwrap_or(percent as f32);
                            let said = format!("{} %", value.round() as u32);
                            let moved = controls::level(
                                ui,
                                icons::SUN,
                                "How bright the display is",
                                None,
                                &mut value,
                                1.0..=100.0,
                                Some(&said),
                            );
                            if moved.slider.dragged() {
                                held.brightness = Some(value);
                            } else if moved.slider.drag_stopped() || moved.slider.changed() {
                                held.brightness = None;
                                if let Some(step) = step_to(percent, value) {
                                    held.brightness_asked.ask(value.round() as u32, now);
                                    asked.push(Ask::Brightness(step));
                                }
                            }
                        }

                        // What is playing: the piece by its name where a
                        // player says it, and the player's keys.
                        ui.add_space(2.0);
                        widgets::caption(ui, "Playing");
                        let on = facts
                            .playing
                            .as_ref()
                            .map(|playing| playing.line())
                            .filter(|line| !line.is_empty());
                        // (The keys from the right end, whatever room they
                        // take; the piece's name has what is left.)
                        let line = egui::vec2(ui.available_width(), 30.0);
                        let from_right = egui::Layout::right_to_left(egui::Align::Center);
                        ui.allocate_ui_with_layout(line, from_right, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (sign, hint, ask) in [
                                (icons::SKIP_FORWARD, "Next", crate::media::Ask::Next),
                                (
                                    icons::PLAY_PAUSE,
                                    "Play or pause",
                                    crate::media::Ask::PlayPause,
                                ),
                                (icons::SKIP_BACK, "Previous", crate::media::Ask::Previous),
                            ] {
                                if widgets::icon_button(ui, sign, hint).clicked() {
                                    asked.push(Ask::Media(ask));
                                }
                            }
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2((ui.available_width() - 6.0).max(40.0), 30.0),
                                egui::Sense::hover(),
                            );
                            let colour = if on.is_some() {
                                palette.text
                            } else {
                                palette.muted
                            };
                            let words = controls::one_line(
                                ui,
                                on.as_deref().unwrap_or("Nothing is playing"),
                                egui::FontId::proportional(13.5),
                                colour,
                                rect.width(),
                            );
                            ui.painter().galley(
                                egui::pos2(rect.left(), rect.center().y - words.size().y / 2.0),
                                words,
                                colour,
                            );
                        });
                    });
                });

                // The foot: always in sight. (As wide as what is above it,
                // not as the part that scrolls.)
                let line = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(inside, 1.0));
                ui.painter().hline(
                    line.x_range(),
                    line.top() + 3.0,
                    egui::Stroke::new(1.0, palette.border),
                );
                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    ui.set_max_width(inside);
                    let half = (inside - 8.0) / 2.0;
                    if ui
                        .add(
                            egui::Button::new(format!("{}  Lock", icons::LOCK))
                                .min_size(egui::vec2(half, 30.0)),
                        )
                        .clicked()
                    {
                        asked.push(Ask::Lock);
                    }
                    if ui
                        .add(
                            egui::Button::new(format!("{}  Power…", icons::POWER))
                                .min_size(egui::vec2(half, 30.0)),
                        )
                        .clicked()
                    {
                        asked.push(Ask::PowerMenu);
                    }
                });
            });
        });
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with a great deal of everything: more devices than fit,
    /// programs that play, radios with what they hear.
    pub(crate) fn busy() -> Facts {
        use gevrek_system::sound::{Level, Stream};
        let device = |id: u32, name: &str, chosen: bool| Output {
            id,
            name: name.to_string(),
            chosen,
        };
        let stream = |id: u32, name: &str, percent: u32, muted: bool| Stream {
            id,
            name: name.to_string(),
            level: Level { percent, muted },
        };
        let network = |name: &str, signal: u8, secured: bool, in_use: bool, known: bool| {
            gevrek_system::wifi::Network {
                name: name.to_string(),
                signal,
                secured,
                in_use,
                known,
            }
        };
        Facts {
            volume: Some((62, false)),
            outputs: vec![
                device(0, "Headphones (USB Audio Device)", false),
                device(1, "Speakers (High Definition Audio)", true),
                device(2, "27GL850 (NVIDIA High Definition Audio)", false),
                device(3, "Speakers (2- Blackmagic DeckLink 8K Pro (4) Audio)", false),
                device(4, "Speakers (2- Blackmagic DeckLink 8K Pro (2) Audio)", false),
                device(5, "Speakers (2- Blackmagic DeckLink 8K Pro (3) Audio)", false),
                device(6, "Speakers (Steam Streaming Speakers)", false),
                device(7, "Speakers (2- Blackmagic DeckLink 8K Pro (1) Audio)", false),
            ],
            inputs: vec![
                device(0, "Webcam 1 (NDI Webcam Audio)", false),
                device(1, "Headset Microphone (USB Audio Device)", true),
                device(2, "Webcam 3 (NDI Webcam Audio)", false),
                device(3, "Webcam 4 (NDI Webcam Audio)", false),
                device(4, "Microphone (Steam Streaming Microphone)", false),
                device(5, "Microphone (C922 Pro Stream Webcam)", false),
                device(6, "Webcam 2 (NDI Webcam Audio)", false),
                device(7, "Line In (2- Blackmagic DeckLink 8K Pro (3) Audio)", false),
                device(8, "Line In (2- Blackmagic DeckLink 8K Pro (2) Audio)", false),
                device(9, "Line In (2- Blackmagic DeckLink 8K Pro (4) Audio)", false),
                device(10, "Line In (2- Blackmagic DeckLink 8K Pro (1) Audio)", false),
            ],
            streams: vec![
                stream(11, "firefox", 100, false),
                stream(12, "Spotify", 45, false),
                stream(13, "Discord", 80, true),
            ],
            brightness: Some(70),
            quiet: false,
            night_light: true,
            recording: false,
            profiles: Some(gevrek_system::power_profile::Profiles {
                active: "balanced".to_string(),
                all: vec![
                    "power-saver".to_string(),
                    "balanced".to_string(),
                    "performance".to_string(),
                ],
            }),
            network: Some(gevrek_system::status::Network {
                connected: true,
                wireless: true,
                name: "gevrek-5G".to_string(),
            }),
            playing: Some(crate::media::Playing {
                player: "spotify".to_string(),
                title: "So What".to_string(),
                artist: "Miles Davis".to_string(),
                playing: true,
            }),
            bluetooth: Some(gevrek_system::bluetooth::Bluetooth {
                powered: true,
                devices: vec![
                    gevrek_system::bluetooth::Device {
                        address: "34:E3:FB:60:89:38".to_string(),
                        name: "Studio Headphones".to_string(),
                        connected: true,
                    },
                    gevrek_system::bluetooth::Device {
                        address: "34:E3:FB:60:89:39".to_string(),
                        name: "MX Master 3S".to_string(),
                        connected: false,
                    },
                    gevrek_system::bluetooth::Device {
                        address: "34:E3:FB:60:89:40".to_string(),
                        name: "Keychron K3".to_string(),
                        connected: false,
                    },
                ],
            }),
            wifi: Some(gevrek_system::wifi::Wifi {
                enabled: true,
                networks: vec![
                    network("gevrek-5G", 88, true, true, true),
                    network("gevrek", 71, true, false, true),
                    network("Cafe Simit", 55, false, false, false),
                    network("TurkTelekom_T4F2", 40, true, false, false),
                    network("SUPERONLINE-WiFi_8823", 31, true, false, false),
                    network("Misafir", 22, true, false, false),
                    network("printer-setup", 12, false, false, false),
                ],
            }),
            joining: None,
            wifi_looking: false,
        }
    }

    /// The control centre drawn into a picture, to be looked at while it
    /// is worked on:
    ///
    ///     GEVREK_LOOK_OUT=centre.png cargo test -p gevrek-shell \
    ///         control_centre_is_drawn_to_be_looked_at -- --ignored
    ///
    /// `GEVREK_LOOK=plain` for a machine with little (no radios, one
    /// device), `GEVREK_LOOK_SIZE=380x1000` for another size,
    /// `GEVREK_LOOK_PRESS=x,y` for a place pressed before the picture.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_control_centre_is_drawn_to_be_looked_at() {
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let pair = |text: &str, by: char| -> Option<(f32, f32)> {
            let (one, other) = text.split_once(by)?;
            Some((one.trim().parse().ok()?, other.trim().parse().ok()?))
        };
        let out = std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let size = said("GEVREK_LOOK_SIZE")
            .and_then(|text| pair(&text, 'x'))
            .map_or((WIDTH, 1000), |(width, height)| (width as u32, height as u32));
        let press = said("GEVREK_LOOK_PRESS").and_then(|text| pair(&text, ','));
        let facts = match said("GEVREK_LOOK").as_deref() {
            Some("plain") => Facts {
                volume: Some((40, false)),
                outputs: busy().outputs.into_iter().take(1).collect(),
                ..Facts::default()
            },
            _ => busy(),
        };
        let mut painter = gevrek_ui::picture::Painter::sized(size)
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut held = Held::default();
        let mut pixels = Vec::new();
        for step in 0..8 {
            let mut events = Vec::new();
            if let Some((x, y)) = press {
                events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
                if step == 3 || step == 4 {
                    events.push(egui::Event::PointerButton {
                        pos: egui::pos2(x, y),
                        button: egui::PointerButton::Primary,
                        pressed: step == 3,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.4, events, |ui| {
                draw(ui, &palette, &facts, &mut held, &crate::frost::Look::plain());
            });
        }
        painter.save(&pixels, &out).expect("the picture is written");
        println!("drawn: {}", out.display());
    }

    #[test]
    fn the_tiles_are_what_the_machine_has_and_say_how_it_stands() {
        let now = std::time::Instant::now();
        let names = |facts: &Facts| -> Vec<(&'static str, String, bool)> {
            tiles_of(facts, &mut Held::default(), now)
                .into_iter()
                .map(|(_, _, name, state, on, _, _)| (name, state, on))
                .collect()
        };
        // Everything: the radios with what they are on, the modes, the
        // power profile by its name, the recording.
        let all = names(&busy());
        let said: Vec<&str> = all.iter().map(|(name, _, _)| *name).collect();
        assert_eq!(
            said,
            ["Wireless", "Bluetooth", "Do not disturb", "Night light", "Power", "Record screen"]
        );
        assert_eq!(all[0], ("Wireless", "gevrek-5G".to_string(), true));
        assert_eq!(all[1], ("Bluetooth", "Studio Headphones".to_string(), true));
        assert_eq!(all[3], ("Night light", "On".to_string(), true));
        // A machine without radios that knows its network says it and
        // has nothing to switch there; one that knows nothing has no tile.
        let wired = Facts {
            network: Some(gevrek_system::status::Network {
                connected: true,
                wireless: false,
                name: "Ethernet".to_string(),
            }),
            ..Facts::default()
        };
        assert_eq!(names(&wired)[0], ("Network", "Ethernet".to_string(), true));
        let bare: Vec<&str> = names(&Facts::default())
            .iter()
            .map(|(name, _, _)| *name)
            .collect();
        assert_eq!(bare, ["Do not disturb", "Night light", "Record screen"]);
        // A radio that was just switched on says so before the system does.
        let mut held = Held::default();
        let mut off = busy();
        if let Some(wifi) = off.wifi.as_mut() {
            wifi.enabled = false;
        }
        assert_eq!(tiles_of(&off, &mut held, now)[0].3, "Off");
        held.wifi_switched.ask(true, now);
        let tile = &tiles_of(&off, &mut held, now)[0];
        assert!(tile.4 && tile.6, "shown on, and waiting");
    }

    /// Drawn without a window, with all there is and with nothing:
    /// nothing in it panics, and nothing is asked for by being drawn.
    #[test]
    fn it_draws_without_a_window_and_asks_nothing_by_itself() {
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        for facts in [busy(), Facts::default()] {
            let mut held = Held {
                open: Some(Detail::Wifi),
                outputs_open: true,
                inputs_open: true,
                ..Held::default()
            };
            for _ in 0..3 {
                let mut asked = Vec::new();
                let mut drawn = context.run_ui(egui::RawInput::default(), |ui| {
                    asked = draw(ui, &palette, &facts, &mut held, &crate::frost::Look::plain());
                });
                drawn.textures_delta.clear();
                assert_eq!(asked, []);
            }
        }
    }

    #[test]
    fn a_slider_let_go_is_a_step_from_where_the_level_is() {
        assert_eq!(step_to(40, 55.4), Some(15));
        assert_eq!(step_to(40, 24.6), Some(-15));
        assert_eq!(step_to(40, 40.3), None);
        assert_eq!(step_to(0, 100.0), Some(100));
    }

    #[test]
    fn a_switch_holds_what_was_asked_until_the_system_follows() {
        let start = std::time::Instant::now();
        let later = |ms| start + std::time::Duration::from_millis(ms);
        let mut switch = Switched::default();
        assert!(
            !switch.shown(false, start),
            "nothing asked: the system's state"
        );
        assert!(!switch.waiting());
        // Pressed on; the system still says off: the switch does not go
        // back, and it is waiting.
        switch.ask(true, start);
        assert!(switch.shown(false, later(300)));
        assert!(switch.shown(false, later(3000)));
        assert!(switch.waiting());
        // The system has followed: it counts again, and nothing waits.
        assert!(switch.shown(true, later(4000)));
        assert!(!switch.waiting());
        assert!(
            !switch.shown(false, later(5000)),
            "switched off by something else"
        );
        // A system that never follows is not waited for for ever.
        switch.ask(true, later(6000));
        assert!(switch.shown(false, later(9000)));
        assert!(!switch.shown(false, later(19000)));
        assert!(!switch.waiting());
    }

    #[test]
    fn a_slider_stays_where_it_was_put_until_the_system_is_there() {
        let start = std::time::Instant::now();
        let later = |ms| start + std::time::Duration::from_millis(ms);
        let mut asked = Asked::default();
        assert_eq!(
            asked.level(40, start),
            40,
            "nothing asked: the system's level"
        );
        asked.ask(70, start);
        // The system still says the old level: the slider does not go back.
        assert_eq!(asked.level(40, later(100)), 70);
        // A second press goes on from where the first put it, not from the old level.
        assert_eq!(step_to(asked.level(40, later(200)), 80.0), Some(10));
        // The system has caught up: it counts again, at once and from then on.
        assert_eq!(asked.level(70, later(300)), 70);
        assert!(!asked.waiting());
        assert_eq!(asked.level(65, later(400)), 65, "something else set it");
        // A level that never comes is not waited for for ever.
        asked.ask(90, later(500));
        assert_eq!(asked.level(65, later(1000)), 90);
        assert_eq!(asked.level(65, later(2100)), 65);
    }

    #[test]
    fn the_control_centre_draws_with_and_without_what_the_machine_has() {
        let palette =
            gevrek_ui::Settings::default().resolve(egui::Theme::Dark, egui::Color32::LIGHT_BLUE);
        for facts in [
            Facts::default(),
            Facts {
                volume: Some((40, false)),
                outputs: vec![
                    Output {
                        id: 1,
                        name: "Speakers".to_string(),
                        chosen: true,
                    },
                    Output {
                        id: 2,
                        name: "Headphones".to_string(),
                        chosen: false,
                    },
                ],
                inputs: vec![
                    Output {
                        id: 3,
                        name: "Built-in microphone".to_string(),
                        chosen: false,
                    },
                    Output {
                        id: 4,
                        name: "Webcam".to_string(),
                        chosen: true,
                    },
                ],
                streams: vec![gevrek_system::sound::Stream {
                    id: 9,
                    name: "A player with a long name of its own".to_string(),
                    level: gevrek_system::sound::Level {
                        percent: 60,
                        muted: true,
                    },
                }],
                brightness: Some(70),
                quiet: true,
                night_light: true,
                recording: true,
                profiles: Some(gevrek_system::power_profile::Profiles {
                    active: "balanced".to_string(),
                    all: vec![
                        "power-saver".to_string(),
                        "balanced".to_string(),
                        "performance".to_string(),
                    ],
                }),
                network: Some(gevrek_system::status::Network {
                    connected: true,
                    wireless: true,
                    name: "Home".to_string(),
                }),
                playing: Some(crate::media::Playing {
                    player: "player".to_string(),
                    title: "A Song".to_string(),
                    artist: "Somebody".to_string(),
                    playing: true,
                }),
                wifi: Some(gevrek_system::wifi::Wifi {
                    enabled: true,
                    networks: vec![
                        gevrek_system::wifi::Network {
                            name: "Home".to_string(),
                            signal: 80,
                            secured: true,
                            in_use: true,
                            known: true,
                        },
                        gevrek_system::wifi::Network {
                            name: "A café's".to_string(),
                            signal: 40,
                            secured: true,
                            in_use: false,
                            known: false,
                        },
                    ],
                }),
                joining: Some("A café's".to_string()),
                wifi_looking: true,
                bluetooth: Some(gevrek_system::bluetooth::Bluetooth {
                    powered: true,
                    devices: vec![gevrek_system::bluetooth::Device {
                        address: "34:E3:FB:60:89:38".to_string(),
                        name: "Headphones".to_string(),
                        connected: true,
                    }],
                }),
            },
        ] {
            let context = egui::Context::default();
            let mut held = Held::default();
            let mut asked = vec![Ask::Close];
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                asked = draw(
                    ui,
                    &palette,
                    &facts,
                    &mut held,
                    &crate::frost::Look::plain(),
                );
            });
            output.textures_delta.clear();
            // Nothing is asked for by being drawn.
            assert!(asked.is_empty(), "{asked:?}");
            assert_eq!(held, Held::default());
        }
    }
}
