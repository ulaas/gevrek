//! Notifications: what programs want the user to know.
//!
//! On Linux programs send them over the session's message bus, by the
//! desktop notifications specification (`org.freedesktop.Notifications`).
//! The service here answers on the bus, on a thread of its own; what
//! arrives is put on a stack that the shell shows in a corner of the
//! screen. On Windows the system keeps its own notifications; the stack
//! there holds what gevrek itself has to tell (the window manager's
//! notices, and what `gevrek notify` sends).
//!
//! [`Stack`] is the stack and its rules (what is shown, for how long,
//! what replaces what); it knows nothing of the bus or the screen.
//! [`draw_cards`] and [`draw_centre`] draw it, the same on every system;
//! the surfaces they are drawn on are each system's own.

use std::time::{Duration, Instant};

/// How long a notification stays that does not say itself.
const STAY: Duration = Duration::from_secs(6);
/// How many are shown at once; the rest wait their turn.
pub const SHOWN: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    /// Stays until it is dismissed.
    Critical,
}

/// One notification as it arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    /// What can be done about it: a key the sender knows, and its words.
    pub actions: Vec<(String, String)>,
    /// How long it stays; `None`: until it is dismissed.
    pub stay: Option<Duration>,
    pub since: Instant,
    /// The picture it came with, or its sender's icon.
    pub picture: Option<std::sync::Arc<Pixels>>,
}

/// Whether a notification that arrives is heard: the setting is on, its
/// sender does not ask for silence, and the user is not to be left
/// undisturbed (unless it cannot wait).
pub fn is_heard(sound_on: bool, silent: bool, quiet: bool, urgency: Urgency) -> bool {
    sound_on && !silent && (!quiet || urgency == Urgency::Critical)
}

/// A picture as it is drawn: red, green, blue and how solid, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The largest side a notification's picture is kept at.
pub const PICTURE_SIDE: u32 = 96;

/// What a sender says its notification's picture is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// An icon's name, or a file.
    Named(String),
    Pixels(Pixels),
}

/// The pixels a sender put into its message (the hint `image-data`: so
/// many bytes a row, three or four a pixel), as a picture no larger than
/// [`PICTURE_SIDE`]. `None` when the numbers do not fit the bytes.
pub fn pixels_sent(
    width: i32,
    height: i32,
    row_bytes: i32,
    channels: i32,
    sent: &[u8],
) -> Option<Pixels> {
    let (width, height) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let (row_bytes, channels) = (
        usize::try_from(row_bytes).ok()?,
        usize::try_from(channels).ok()?,
    );
    if width == 0 || height == 0 || !(3..=4).contains(&channels) {
        return None;
    }
    if row_bytes < width.checked_mul(channels)? {
        return None;
    }
    // (The last row need not be filled up to the row's length.)
    let needed = row_bytes
        .checked_mul(height - 1)?
        .checked_add(width * channels)?;
    if sent.len() < needed {
        return None;
    }
    // Made smaller by taking every so-manieth pixel: it is drawn small.
    let shrink = (width.max(height) as f32 / PICTURE_SIDE as f32).max(1.0);
    let (out_width, out_height) = (
        ((width as f32 / shrink) as usize).max(1),
        ((height as f32 / shrink) as usize).max(1),
    );
    let mut rgba = Vec::with_capacity(out_width * out_height * 4);
    for y in 0..out_height {
        let from_y = ((y as f32 * shrink) as usize).min(height - 1);
        for x in 0..out_width {
            let from_x = ((x as f32 * shrink) as usize).min(width - 1);
            let at = from_y * row_bytes + from_x * channels;
            rgba.extend_from_slice(&sent[at..at + 3]);
            rgba.push(if channels == 4 { sent[at + 3] } else { 255 });
        }
    }
    Some(Pixels {
        width: out_width as u32,
        height: out_height as u32,
        rgba,
    })
}

/// Which picture a notification has, of all a sender may name: the
/// pixels in the message before a file or name given with it, that before
/// the sender's icon, and the icon of the sender's desktop entry last.
pub fn picture_of(
    sent: Option<Pixels>,
    image_path: Option<&str>,
    app_icon: &str,
    desktop_entry: Option<&str>,
) -> Option<Picture> {
    let named = |name: &str| {
        let name = name.trim();
        let name = name.strip_prefix("file://").unwrap_or(name);
        (!name.is_empty()).then(|| Picture::Named(name.to_string()))
    };
    sent.map(Picture::Pixels)
        .or_else(|| image_path.and_then(named))
        .or_else(|| named(app_icon))
        .or_else(|| desktop_entry.and_then(named))
}

/// The picture as pixels: an icon is looked for and read, a file is read.
pub fn pixels_of(picture: Picture) -> Option<Pixels> {
    match picture {
        Picture::Pixels(pixels) => Some(pixels),
        Picture::Named(name) => {
            let read = if std::path::Path::new(&name).is_absolute() {
                gevrek_system::icons::load(std::path::Path::new(&name), PICTURE_SIDE)
            } else {
                gevrek_system::icons::picture(&name, PICTURE_SIDE)
            }?;
            Some(Pixels {
                width: read.width,
                height: read.height,
                rgba: read.pixels,
            })
        }
    }
}

/// Why a notification went (the numbers of the specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    Expired = 1,
    Dismissed = 2,
    /// Its sender took it back.
    Closed = 3,
}

/// How many past notifications are kept to look at again.
pub const REMEMBERED: usize = 60;

pub struct Stack {
    items: Vec<Notification>,
    last_id: u32,
    /// Do not disturb: what arrives is remembered and not shown, unless
    /// it is critical.
    pub quiet: bool,
    /// What arrived, the newest first, for the notification centre.
    history: Vec<Notification>,
    /// How many are shown at once and how long one stays that does not
    /// say itself (`desktop.shell.notifications`; five and six seconds
    /// until the configuration is known).
    at_once: usize,
    stay: Duration,
}

impl Default for Stack {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            last_id: 0,
            quiet: false,
            history: Vec::new(),
            at_once: SHOWN,
            stay: STAY,
        }
    }
}

impl Stack {
    /// How many are shown at once, and how long one stays, from now on.
    pub fn configure(&mut self, at_once: usize, stay: Duration) {
        self.at_once = at_once.max(1);
        self.stay = stay;
    }

    /// Take a notification in; it gets a number, or keeps the number of
    /// the one it replaces. Returns the number.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        app: &str,
        replaces: u32,
        summary: &str,
        body: &str,
        actions: &[String],
        urgency: Urgency,
        expire_ms: i32,
        now: Instant,
    ) -> u32 {
        self.take(
            app, replaces, summary, body, actions, urgency, expire_ms, now, true,
        )
    }

    /// Take in words of gevrek's own, which are text as they stand (no
    /// markup to take out: a `<` is a `<`). `stay_ms` as a sender says
    /// it: 0 is until dismissed, less the usual time. Returns the number.
    pub fn tell(
        &mut self,
        app: &str,
        summary: &str,
        body: &str,
        stay_ms: i32,
        now: Instant,
    ) -> u32 {
        self.take(
            app,
            0,
            summary,
            body,
            &[],
            Urgency::Normal,
            stay_ms,
            now,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn take(
        &mut self,
        app: &str,
        replaces: u32,
        summary: &str,
        body: &str,
        actions: &[String],
        urgency: Urgency,
        expire_ms: i32,
        now: Instant,
        markup: bool,
    ) -> u32 {
        let stay = match (urgency, expire_ms) {
            // The sender says how long.
            (_, milliseconds) if milliseconds > 0 => {
                Some(Duration::from_millis(milliseconds as u64))
            }
            // The sender says: never.
            (_, 0) => None,
            (Urgency::Critical, _) => None,
            _ => Some(self.stay),
        };
        let replaced = self
            .items
            .iter()
            .position(|item| replaces != 0 && item.id == replaces);
        let id = match replaced {
            Some(_) => replaces,
            None => {
                self.last_id = self.last_id.wrapping_add(1).max(1);
                self.last_id
            }
        };
        let (summary, body) = if markup {
            (plain(summary), plain(body))
        } else {
            (summary.to_string(), body.to_string())
        };
        let notification = Notification {
            id,
            app: app.to_string(),
            summary,
            body,
            urgency,
            // The list comes as key, words, key, words.
            actions: actions
                .chunks(2)
                .filter_map(|pair| Some((pair.first()?.clone(), pair.get(1)?.clone())))
                .collect(),
            stay,
            since: now,
            picture: None,
        };
        self.history.retain(|past| past.id != id);
        self.history.insert(0, notification.clone());
        self.history.truncate(REMEMBERED);
        match replaced {
            Some(index) => self.items[index] = notification,
            // Not shown while the user does not want to be disturbed,
            // unless the sender says it cannot wait.
            None if self.quiet && urgency != Urgency::Critical => {}
            None => self.items.push(notification),
        }
        id
    }

    /// Take one away. `false` when there is none with that number.
    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        self.items.len() != before
    }

    /// Those whose time is over at `now`, taken off the stack. Only what
    /// is shown counts its time: one that waits its turn has all of it.
    pub fn expire(&mut self, now: Instant) -> Vec<u32> {
        let over: Vec<u32> = self
            .items
            .iter()
            .take(self.at_once)
            .filter(|item| {
                item.stay
                    .is_some_and(|stay| now.duration_since(item.since) >= stay)
            })
            .map(|item| item.id)
            .collect();
        self.items.retain(|item| !over.contains(&item.id));
        // Those that come into view now start their time now.
        if !over.is_empty() {
            let came = self.at_once.saturating_sub(over.len());
            for item in self.items.iter_mut().take(self.at_once).skip(came) {
                item.since = now;
            }
        }
        over
    }

    /// What arrived, the newest first.
    /// Give a notification its picture.
    pub fn set_picture(&mut self, id: u32, picture: Pixels) {
        let picture = std::sync::Arc::new(picture);
        for note in self
            .items
            .iter_mut()
            .chain(self.history.iter_mut())
            .filter(|note| note.id == id)
        {
            note.picture = Some(std::sync::Arc::clone(&picture));
        }
    }

    pub fn history(&self) -> &[Notification] {
        &self.history
    }

    /// What arrived, as it is written down to be there again after the
    /// shell was started anew: who said it, what, and how long ago (by the
    /// clock on the wall, which outlives the program).
    pub fn remembered(&self, now: Instant, wall: std::time::SystemTime) -> String {
        let saved = wall
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let notes: Vec<serde_json::Value> = self
            .history
            .iter()
            .map(|note| {
                serde_json::json!({
                    "app": note.app,
                    "summary": note.summary,
                    "body": note.body,
                    "age": now.saturating_duration_since(note.since).as_secs(),
                })
            })
            .collect();
        serde_json::json!({"saved": saved, "notes": notes}).to_string()
    }

    /// Take what was written down by [`Stack::remembered`] as what
    /// arrived (none of it is shown again). Only into a stack that has
    /// heard nothing yet; what cannot be read is nothing.
    pub fn remember(&mut self, written: &str, now: Instant, wall: std::time::SystemTime) {
        if !self.history.is_empty() {
            return;
        }
        let Ok(read) = serde_json::from_str::<serde_json::Value>(written) else {
            return;
        };
        let since_saved = wall
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs())
            .saturating_sub(read["saved"].as_u64().unwrap_or(0));
        for note in read["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .take(REMEMBERED)
        {
            let text = |key: &str| note[key].as_str().unwrap_or_default().to_string();
            let age = Duration::from_secs(note["age"].as_u64().unwrap_or(0) + since_saved);
            self.last_id = self.last_id.wrapping_add(1).max(1);
            self.history.push(Notification {
                id: self.last_id,
                app: text("app"),
                summary: text("summary"),
                body: text("body"),
                urgency: Urgency::Normal,
                actions: Vec::new(),
                stay: None,
                // (As long ago as the machine's own clock reaches back.)
                since: now.checked_sub(age).unwrap_or(now),
                picture: None,
            });
        }
    }

    pub fn forget_history(&mut self) {
        self.history.clear();
    }

    /// Put away everything that is shown or waits to be; what arrived is
    /// still remembered.
    pub fn put_away(&mut self) {
        self.items.clear();
    }

    /// The ones to show, the oldest first.
    pub fn shown(&self) -> &[Notification] {
        &self.items[..self.items.len().min(self.at_once)]
    }

    /// How many wait behind the ones that are shown.
    pub fn waiting(&self) -> usize {
        self.items.len().saturating_sub(self.at_once)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// When the next one's time is over, if any counts.
    pub fn next_expiry(&self) -> Option<Instant> {
        self.items
            .iter()
            .take(self.at_once)
            .filter_map(|item| Some(item.since + item.stay?))
            .min()
    }
}

/// Text without the markup a sender may put in it (`<b>`, `<a href>`),
/// and with the few named characters written out.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for letter in text.chars() {
        match letter {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            other if !in_tag => out.push(other),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(target_os = "linux")]
/// What the service on the bus tells the shell.
pub enum FromBus {
    Notify {
        app: String,
        replaces: u32,
        summary: String,
        body: String,
        actions: Vec<String>,
        urgency: Urgency,
        expire_ms: i32,
        picture: Option<Picture>,
        /// The sender asks for no sound, or names the one it wants.
        silent: bool,
        sound: Option<String>,
        /// The number is decided by the shell and sent back here.
        answer: std::sync::mpsc::Sender<u32>,
    },
    Close(u32),
}

#[cfg(target_os = "linux")]
/// What the shell tells the bus.
pub enum ToBus {
    Closed(u32, Gone),
    Action(u32, String),
}

#[cfg(target_os = "linux")]
struct Service<F: Fn(FromBus) + Send + Sync + 'static> {
    tell: F,
}

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.freedesktop.Notifications")]
impl<F: Fn(FromBus) + Send + Sync + 'static> Service<F> {
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["body", "actions", "persistence", "icon-static", "sound"]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let urgency = match hints
            .get("urgency")
            .and_then(|value| u8::try_from(value).ok())
        {
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };
        // Its picture: pixels in the message (under the name of today
        // or those of before), a file or a name, the sender's icon.
        let sent = ["image-data", "image_data", "icon_data"]
            .iter()
            .find_map(|name| hints.get(*name))
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| <(i32, i32, i32, bool, i32, i32, Vec<u8>)>::try_from(value).ok())
            .and_then(|(width, height, row_bytes, _, _, channels, bytes)| {
                pixels_sent(width, height, row_bytes, channels, &bytes)
            });
        let text = |name: &str| {
            hints
                .get(name)
                .and_then(|value| <&str>::try_from(value).ok())
                .map(str::to_string)
        };
        let image_path = text("image-path").or_else(|| text("image_path"));
        let picture = picture_of(
            sent,
            image_path.as_deref(),
            app_icon,
            text("desktop-entry").as_deref(),
        );
        let silent = hints
            .get("suppress-sound")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        let (answer, number) = std::sync::mpsc::channel();
        (self.tell)(FromBus::Notify {
            picture,
            silent,
            sound: text("sound-name"),
            app: app_name.to_string(),
            replaces: replaces_id,
            summary: summary.to_string(),
            body: body.to_string(),
            actions,
            urgency,
            expire_ms: expire_timeout,
            answer,
        });
        // The shell answers within its next turn; a sender is not kept
        // waiting when it does not.
        number.recv_timeout(Duration::from_secs(2)).unwrap_or(0)
    }

    fn close_notification(&self, id: u32) {
        (self.tell)(FromBus::Close(id));
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("gevrek", "gevrek", env!("CARGO_PKG_VERSION"), "1.2")
    }
}

#[cfg(target_os = "linux")]
const PATH: &str = "/org/freedesktop/Notifications";
#[cfg(target_os = "linux")]
const NAME: &str = "org.freedesktop.Notifications";

#[cfg(target_os = "linux")]
/// Answer on the session's bus as the notification service, on a thread
/// of its own. `tell` is called for what arrives; what is sent into the
/// returned channel goes out as the service's signals. An error when the
/// bus is not there or another service has the name.
pub fn serve(
    tell: impl Fn(FromBus) + Send + Sync + 'static,
) -> anyhow::Result<std::sync::mpsc::Sender<ToBus>> {
    let connection = zbus::blocking::connection::Builder::session()?
        .name(NAME)?
        .serve_at(PATH, Service { tell })?
        .build()
        .map_err(|error| {
            anyhow::anyhow!("the notification service could not take its name on the bus: {error}")
        })?;
    let (sender, receiver) = std::sync::mpsc::channel::<ToBus>();
    std::thread::Builder::new()
        .name("gevrek-notifications".to_string())
        .spawn(move || {
            for message in receiver {
                let sent = match message {
                    ToBus::Closed(id, why) => connection.emit_signal(
                        Option::<&str>::None,
                        PATH,
                        NAME,
                        "NotificationClosed",
                        &(id, why as u32),
                    ),
                    ToBus::Action(id, key) => connection.emit_signal(
                        Option::<&str>::None,
                        PATH,
                        NAME,
                        "ActionInvoked",
                        &(id, key.as_str()),
                    ),
                };
                if let Err(error) = sent {
                    log::warn!("a notification's sender was not told: {error}");
                }
            }
        })?;
    Ok(sender)
}

/// The width of the notifications' surface, in logical pixels.
pub const NOTE_WIDTH: u32 = 380;
/// A card's height, its edge and inside margin included.
pub const NOTE_HEIGHT: u32 = 92;

/// The height of the notifications' surface for what is on the stack.
pub fn height(stack: &Stack) -> u32 {
    let shown = stack.shown().len() as u32;
    shown * (NOTE_HEIGHT + 8) + if stack.waiting() > 0 { 22 } else { 0 }
}

/// Draw the cards of what is shown, one under the other. The one that
/// was pressed, if one was: its number, and the key of what was chosen
/// on it (`default` for a press on a card that offers that, nothing for
/// a press that only puts it away).
/// A notification's picture as something to draw, made once for each
/// picture (kept with the drawing's own memory, by the notification's
/// number and which picture it was made of).
fn texture_of(ui: &egui::Ui, note: &Notification) -> Option<egui::TextureHandle> {
    let pixels = note.picture.as_ref()?;
    let key = egui::Id::unique(("note-picture", note.id));
    let address = std::sync::Arc::as_ptr(pixels) as usize;
    let known = ui
        .ctx()
        .data(|data| data.get_temp::<(usize, egui::TextureHandle)>(key));
    if let Some((made_of, texture)) = known {
        if made_of == address {
            return Some(texture);
        }
    }
    let texture = ui.ctx().load_texture(
        format!("note-{}", note.id),
        egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width as usize, pixels.height as usize],
            &pixels.rgba,
        ),
        egui::TextureOptions::LINEAR,
    );
    ui.ctx()
        .data_mut(|data| data.insert_temp(key, (address, texture.clone())));
    Some(texture)
}

/// How large a notification's picture is drawn (its longer side).
const PICTURE_DRAWN: f32 = 40.0;

pub fn draw_cards(
    ui: &mut egui::Ui,
    palette: &gevrek_ui::theme::Palette,
    stack: &Stack,
) -> Option<(u32, Option<String>)> {
    let mut gone: Option<(u32, Option<String>)> = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for note in stack.shown() {
                let edge = if note.urgency == Urgency::Critical {
                    palette.error
                } else {
                    palette.border_strong()
                };
                let card = egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, edge))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(NOTE_WIDTH as f32 - 30.0);
                        // A card is as high as its place in the sum above:
                        // the frame adds its margin (10 above and below)
                        // and its edge.
                        let inside = NOTE_HEIGHT as f32 - 22.0;
                        ui.set_min_height(inside);
                        ui.set_max_height(inside);
                        let picture = texture_of(ui, note);
                        ui.horizontal_top(|ui| {
                            if let Some(texture) = &picture {
                                // As high as two lines, its shape kept.
                                let size = texture.size_vec2();
                                let scale = PICTURE_DRAWN / size.x.max(size.y).max(1.0);
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(size * scale)
                                        .corner_radius(6),
                                );
                                ui.add_space(4.0);
                            }
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    // The sender's name first, at the right
                                    // end; what is left of the line is the
                                    // title's, and one that is longer ends
                                    // in dots (it ran on under the name).
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(&note.app)
                                                    .size(11.5)
                                                    .color(palette.faint),
                                            );
                                            ui.with_layout(
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.add(
                                                        egui::Label::new(
                                                            egui::RichText::new(&note.summary)
                                                                .strong()
                                                                .size(14.5)
                                                                .color(palette.text),
                                                        )
                                                        .truncate(),
                                                    );
                                                },
                                            );
                                        },
                                    );
                                });
                                if !note.body.is_empty() {
                                    let body: String = note.body.chars().take(96).collect();
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(body)
                                                .size(13.0)
                                                .color(palette.muted),
                                        )
                                        .wrap(),
                                    );
                                }
                                // What the sender offers beyond a click on the card.
                                if note.actions.iter().any(|(key, _)| key != "default") {
                                    ui.horizontal(|ui| {
                                        for (key, words) in
                                            note.actions.iter().filter(|(key, _)| key != "default")
                                        {
                                            if ui.small_button(words).clicked() {
                                                gone = Some((note.id, Some(key.clone())));
                                            }
                                        }
                                    });
                                }
                            });
                        });
                    });
                let click = ui.interact(
                    card.response.rect,
                    ui.scope_id().with(("note", note.id)),
                    egui::Sense::click(),
                );
                if click.clicked() && gone.is_none() {
                    let default = note
                        .actions
                        .iter()
                        .any(|(key, _)| key == "default")
                        .then(|| "default".to_string());
                    gone = Some((note.id, default));
                }
            }
            if stack.waiting() > 0 {
                ui.label(
                    egui::RichText::new(format!("and {} more", stack.waiting()))
                        .size(12.0)
                        .color(palette.muted),
                );
            }
        });
    gone
}

/// What a frame of the notification centre asks for.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CentreAsked {
    /// Put the centre away (its button; the surface sees to Escape).
    pub close: bool,
    /// Forget everything that arrived.
    pub forget: bool,
    /// Do not disturb, as its switch stands after this frame.
    pub quiet: bool,
    /// Where the button that forgets everything is, on the surface.
    pub clear: Option<egui::Rect>,
}

/// How long ago something arrived, as it is said in a list: `now`, `5
/// min`, `3 h`, `2 d`.
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{} min", seconds / 60),
        3600..=86_399 => format!("{} h", seconds / 3600),
        _ => format!("{} d", seconds / 86_400),
    }
}

/// What arrived, put together by who sent it: the senders in the order
/// of their newest notification, each with its own, the newest first.
pub fn by_sender(history: &[Notification]) -> Vec<(&str, Vec<&Notification>)> {
    let mut groups: Vec<(&str, Vec<&Notification>)> = Vec::new();
    for note in history {
        match groups.iter_mut().find(|(app, _)| *app == note.app) {
            Some((_, notes)) => notes.push(note),
            None => groups.push((note.app.as_str(), vec![note])),
        }
    }
    groups
}

/// Draw the notification centre: what arrived, put together by who sent
/// it, the newest first; do not disturb as the tile it is in the control
/// centre; and at the foot, always in sight, the button that forgets all
/// of it.
pub fn draw_centre(
    ui: &mut egui::Ui,
    palette: &gevrek_ui::theme::Palette,
    stack: &Stack,
    now: Instant,
    quiet: bool,
    look: &crate::frost::Look,
) -> CentreAsked {
    use gevrek_ui::controls::{self, Tile};
    use gevrek_ui::theme::icons;
    use gevrek_ui::widgets;
    let mut asked = CentreAsked {
        quiet,
        ..Default::default()
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            // The ground (frosted glass, or plain), where it is in this
            // frame; what follows stands on it.
            let card = look.ground(ui, palette, 18);
            let builder = egui::UiBuilder::new().max_rect(card.shrink(18.0));
            ui.scope_builder(builder, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let history = stack.history();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Notifications")
                            .size(15.0)
                            .strong()
                            .color(palette.text),
                    );
                    if !history.is_empty() {
                        ui.label(
                            egui::RichText::new(history.len().to_string())
                                .size(12.0)
                                .color(palette.faint),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::brand_icon_button(
                            ui,
                            gevrek_ui::brand_icons::BrandIcon::Close,
                            icons::X,
                            "Put it away (Escape)",
                        )
                        .clicked()
                        {
                            asked.close = true;
                        }
                    });
                });
                // Do not disturb: what arrives is kept here and not shown.
                let inside = ui.available_width();
                let pressed = controls::tile(
                    ui,
                    egui::vec2(inside, 52.0),
                    &Tile {
                        icon: if quiet {
                            icons::BELL_SLASH
                        } else {
                            icons::BELL
                        },
                        name: "Do not disturb",
                        state: if quiet {
                            "What arrives is kept here and not shown"
                        } else {
                            "Off: what arrives is shown as it comes"
                        },
                        on: quiet,
                        more: None,
                        busy: false,
                    },
                );
                if pressed.switched {
                    asked.quiet = !quiet;
                }

                // The foot is kept room for; what is between scrolls, its
                // bar in the card's margin.
                let foot = 30.0 + ui.spacing().item_spacing.y + 8.0;
                let room = (ui.available_height() - foot).max(80.0);
                let part =
                    egui::Rect::from_min_size(ui.cursor().min, egui::vec2(inside + 12.0, room));
                ui.scope_builder(egui::UiBuilder::new().max_rect(part), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("the notifications")
                        .max_height(room)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(inside);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            if history.is_empty() {
                                ui.add_space(room * 0.28);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new(icons::BELL)
                                            .size(34.0)
                                            .color(palette.faint),
                                    );
                                    ui.label(
                                        egui::RichText::new("Nothing has arrived.")
                                            .color(palette.muted),
                                    );
                                });
                                return;
                            }
                            for (app, notes) in by_sender(history) {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    widgets::caption(ui, if app.is_empty() { "a program" } else { app });
                                    if notes.len() > 1 {
                                        ui.label(
                                            egui::RichText::new(notes.len().to_string())
                                                .size(10.5)
                                                .color(palette.faint),
                                        );
                                    }
                                });
                                for note in notes {
                                    let edge = if note.urgency == Urgency::Critical {
                                        palette.error
                                    } else {
                                        palette.border
                                    };
                                    egui::Frame::new()
                                        .fill(palette.control)
                                        .stroke(egui::Stroke::new(1.0, edge))
                                        .corner_radius(12)
                                        .inner_margin(egui::Margin::symmetric(12, 9))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.spacing_mut().item_spacing.y = 3.0;
                                            ui.horizontal(|ui| {
                                                // Its picture, as high as the line.
                                                if let Some(texture) = texture_of(ui, note) {
                                                    let size = texture.size_vec2();
                                                    let scale = 18.0 / size.x.max(size.y).max(1.0);
                                                    ui.add(
                                                        egui::Image::new(&texture)
                                                            .fit_to_exact_size(size * scale)
                                                            .corner_radius(4),
                                                    );
                                                }
                                                // When, at the right end; the
                                                // title has what is left, cut
                                                // where it is longer.
                                                let when = ago(
                                                    now.saturating_duration_since(note.since).as_secs(),
                                                );
                                                let when = ui.painter().layout_no_wrap(
                                                    when,
                                                    egui::FontId::proportional(11.5),
                                                    palette.faint,
                                                );
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(ui.available_width(), 20.0),
                                                    egui::Sense::hover(),
                                                );
                                                let title = controls::one_line(
                                                    ui,
                                                    &note.summary,
                                                    egui::FontId::proportional(13.5),
                                                    palette.text,
                                                    rect.width() - when.size().x - 10.0,
                                                );
                                                ui.painter().galley(
                                                    egui::pos2(
                                                        rect.left(),
                                                        rect.center().y - title.size().y / 2.0,
                                                    ),
                                                    title,
                                                    palette.text,
                                                );
                                                ui.painter().galley(
                                                    egui::pos2(
                                                        rect.right() - when.size().x,
                                                        rect.center().y - when.size().y / 2.0,
                                                    ),
                                                    when,
                                                    palette.faint,
                                                );
                                            });
                                            if !note.body.is_empty() {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(&note.body)
                                                            .size(12.5)
                                                            .color(palette.muted),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                        });
                                }
                            }
                        });
                });

                // The foot: always in sight, as wide as what is above it.
                let line = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(inside, 1.0));
                ui.painter().hline(
                    line.x_range(),
                    line.top() + 3.0,
                    egui::Stroke::new(1.0, palette.border),
                );
                ui.add_space(7.0);
                let clear = ui.add_enabled(
                    !history.is_empty(),
                    egui::Button::new(format!("{}  Clear all", icons::TRASH))
                        .min_size(egui::vec2(inside, 30.0)),
                );
                asked.clear = Some(clear.rect);
                if clear.clicked() {
                    asked.forget = true;
                }
            });
        });
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_arrived_is_put_together_by_who_sent_it_and_says_how_long_ago() {
        assert_eq!(ago(0), "now");
        assert_eq!(ago(59), "now");
        assert_eq!(ago(60), "1 min");
        assert_eq!(ago(3599), "59 min");
        assert_eq!(ago(3600), "1 h");
        assert_eq!(ago(86_399), "23 h");
        assert_eq!(ago(3 * 86_400 + 5), "3 d");

        let start = Instant::now();
        let mut stack = Stack::default();
        for (app, title) in [
            ("Mail", "one"),
            ("Chat", "two"),
            ("Mail", "three"),
            ("gevrek", "four"),
            ("Chat", "five"),
        ] {
            stack.tell(app, title, "", -1, start);
        }
        // The newest first: so the senders are in the order of their
        // newest, and each has its own together.
        let groups: Vec<(&str, Vec<&str>)> = by_sender(stack.history())
            .into_iter()
            .map(|(app, notes)| {
                (
                    app,
                    notes.iter().map(|note| note.summary.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            groups,
            [
                ("Chat", vec!["five", "two"]),
                ("gevrek", vec!["four"]),
                ("Mail", vec!["three", "one"]),
            ]
        );
        assert!(by_sender(&[]).is_empty());
    }

    /// The notification centre drawn into a picture, to be looked at:
    ///
    ///     GEVREK_LOOK_OUT=notes.png cargo test -p gevrek-shell \
    ///         notification_centre_is_drawn -- --ignored
    ///
    /// `GEVREK_LOOK=empty` for one that nothing arrived in.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_notification_centre_is_drawn_to_be_looked_at() {
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let out = std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let start = Instant::now();
        let mut stack = Stack::default();
        if said("GEVREK_LOOK").as_deref() != Some("empty") {
            for (app, title, text) in [
                ("Mail", "Weekly report", "The numbers for the week are in the sheet that is attached."),
                ("gevrek", "A new configuration is applied", ""),
                ("Chat", "Deniz", "Are you coming to the simit place at twelve? We have a table by the window."),
                ("Mail", "Your order has shipped and will arrive on Thursday before noon", "Track it with the number in this message."),
                ("Chat", "Deniz", "Bring the cables."),
                ("gevrek", "Recording the screen", "gevrek does not record the screen on Windows yet."),
                ("Calendar", "Stand-up in ten minutes", "Room 3"),
            ] {
                stack.tell(app, title, text, -1, start);
            }
        }
        let size = (400, 760);
        let mut painter = gevrek_ui::picture::Painter::sized(size)
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        gevrek_ui::theme::apply(&context, &palette, &settings);
        let later = start + Duration::from_secs(7 * 60);
        let mut pixels = Vec::new();
        for step in 0..4 {
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.4, Vec::new(), |ui| {
                draw_centre(ui, &palette, &stack, later, false, &crate::frost::Look::plain());
            });
        }
        painter.save(&pixels, &out).expect("the picture is written");
        println!("drawn: {}", out.display());
    }

    #[test]
    fn a_notification_is_heard_when_the_setting_the_sender_and_the_user_allow() {
        assert!(is_heard(true, false, false, Urgency::Normal));
        assert!(!is_heard(false, false, false, Urgency::Critical));
        assert!(!is_heard(true, true, false, Urgency::Critical));
        // Do not disturb: only what cannot wait.
        assert!(!is_heard(true, false, true, Urgency::Normal));
        assert!(is_heard(true, false, true, Urgency::Critical));
    }

    #[test]
    fn a_notifications_picture_is_the_nearest_to_it_that_is_named() {
        let dot = Pixels {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        };
        // The pixels in the message before everything else.
        assert_eq!(
            picture_of(Some(dot.clone()), Some("/a.png"), "icon", Some("app")),
            Some(Picture::Pixels(dot))
        );
        assert_eq!(
            picture_of(None, Some("file:///home/x/a.png"), "icon", Some("app")),
            Some(Picture::Named("/home/x/a.png".to_string()))
        );
        assert_eq!(
            picture_of(None, None, " mail-unread ", Some("app")),
            Some(Picture::Named("mail-unread".to_string()))
        );
        assert_eq!(
            picture_of(None, Some(""), "", Some("org.gnome.Calendar")),
            Some(Picture::Named("org.gnome.Calendar".to_string()))
        );
        assert_eq!(picture_of(None, None, "", None), None);
    }

    #[test]
    fn pixels_of_a_message_are_read_by_its_rows() {
        // Two by two, three bytes a pixel, rows of eight bytes (two to
        // spare), the last row not filled up.
        let sent = [
            10, 11, 12, 20, 21, 22, 0, 0, //
            30, 31, 32, 40, 41, 42,
        ];
        let read = pixels_sent(2, 2, 8, 3, &sent).expect("a picture");
        assert_eq!((read.width, read.height), (2, 2));
        assert_eq!(
            read.rgba,
            [10, 11, 12, 255, 20, 21, 22, 255, 30, 31, 32, 255, 40, 41, 42, 255]
        );
        // With solidness.
        let read = pixels_sent(1, 1, 4, 4, &[1, 2, 3, 128]).expect("a picture");
        assert_eq!(read.rgba, [1, 2, 3, 128]);
        // A large one is made small, its shape kept.
        let large = vec![7u8; 400 * 200 * 4];
        let read = pixels_sent(400, 200, 1600, 4, &large).expect("a picture");
        assert_eq!((read.width, read.height), (96, 48));
        assert_eq!(read.rgba.len(), 96 * 48 * 4);
        // Numbers that do not fit the bytes.
        assert_eq!(pixels_sent(2, 2, 8, 3, &sent[..10]), None);
        assert_eq!(pixels_sent(2, 2, 4, 3, &sent), None);
        assert_eq!(pixels_sent(0, 2, 8, 3, &sent), None);
        assert_eq!(pixels_sent(2, 2, 8, 2, &sent), None);
        assert_eq!(pixels_sent(-2, 2, 8, 3, &sent), None);
    }

    fn push(
        stack: &mut Stack,
        summary: &str,
        urgency: Urgency,
        expire_ms: i32,
        replaces: u32,
        now: Instant,
    ) -> u32 {
        stack.push("test", replaces, summary, "", &[], urgency, expire_ms, now)
    }

    #[test]
    fn a_notification_stays_its_time_and_a_critical_one_until_dismissed() {
        let start = Instant::now();
        let mut stack = Stack::default();
        let usual = push(&mut stack, "usual", Urgency::Normal, -1, 0, start);
        let short = push(&mut stack, "short", Urgency::Normal, 1000, 0, start);
        let critical = push(&mut stack, "critical", Urgency::Critical, -1, 0, start);
        let forever = push(&mut stack, "forever", Urgency::Normal, 0, 0, start);
        assert_eq!((usual, short, critical, forever), (1, 2, 3, 4));
        assert!(stack.expire(start + Duration::from_millis(500)).is_empty());
        assert_eq!(stack.expire(start + Duration::from_millis(1500)), [short]);
        assert_eq!(stack.next_expiry(), Some(start + STAY));
        assert_eq!(stack.expire(start + Duration::from_secs(60)), [usual]);
        let left: Vec<&str> = stack
            .shown()
            .iter()
            .map(|item| item.summary.as_str())
            .collect();
        assert_eq!(left, ["critical", "forever"]);
        assert_eq!(stack.next_expiry(), None);
        assert!(stack.remove(critical));
        assert!(!stack.remove(critical));
    }

    #[test]
    fn what_arrived_is_there_again_after_the_shell_was_started_anew() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let wall = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut stack = Stack::default();
        stack.tell("gevrek", "first", "one", -1, now - Duration::from_secs(120));
        stack.tell("gevrek", "second", "two", -1, now - Duration::from_secs(30));
        let written = stack.remembered(now, wall);

        // Started anew a minute later (by the clock on the wall).
        let later = now + Duration::from_secs(60);
        let mut anew = Stack::default();
        anew.remember(&written, later, wall + Duration::from_secs(60));
        let titles: Vec<&str> = anew
            .history()
            .iter()
            .map(|note| note.summary.as_str())
            .collect();
        assert_eq!(titles, ["second", "first"]);
        // As long ago as they were, plus the time in between.
        assert_eq!(
            later.duration_since(anew.history()[0].since),
            Duration::from_secs(90)
        );
        // None of it is shown again, and what arrives now gets a number
        // of its own.
        assert!(anew.shown().is_empty());
        let id = anew.tell("gevrek", "third", "", -1, later);
        assert!(anew.history().iter().filter(|note| note.id == id).count() == 1);
        assert_eq!(anew.history().len(), 3);
        // A stack that has heard something is left as it is; what cannot
        // be read is nothing.
        anew.remember(&written, later, wall);
        assert_eq!(anew.history().len(), 3);
        let mut empty = Stack::default();
        empty.remember("not json", later, wall);
        assert!(empty.history().is_empty());
    }

    #[test]
    fn a_notification_can_be_replaced_and_keeps_its_number_and_place() {
        let now = Instant::now();
        let mut stack = Stack::default();
        let first = push(&mut stack, "download 10%", Urgency::Normal, -1, 0, now);
        let second = push(&mut stack, "mail", Urgency::Normal, -1, 0, now);
        let again = push(&mut stack, "download 90%", Urgency::Normal, -1, first, now);
        assert_eq!(again, first);
        let shown: Vec<&str> = stack
            .shown()
            .iter()
            .map(|item| item.summary.as_str())
            .collect();
        assert_eq!(shown, ["download 90%", "mail"]);
        // A number nothing has is a new notification.
        let new = push(&mut stack, "other", Urgency::Normal, -1, 999, now);
        assert!(new != 999 && new != first && new != second);
    }

    #[test]
    fn only_a_few_are_shown_and_the_rest_wait() {
        let now = Instant::now();
        let mut stack = Stack::default();
        for index in 0..8 {
            push(&mut stack, &format!("n{index}"), Urgency::Normal, 0, 0, now);
        }
        assert_eq!(stack.shown().len(), SHOWN);
        assert_eq!(stack.waiting(), 3);
        assert_eq!(stack.shown()[0].summary, "n0");
        stack.remove(1);
        assert_eq!(stack.shown().len(), SHOWN);
        assert_eq!(stack.waiting(), 2);
        // Put away, all of them are still what arrived.
        stack.put_away();
        assert!(stack.is_empty() && stack.waiting() == 0);
        assert_eq!(stack.history().len(), 8);
    }

    #[test]
    fn do_not_disturb_remembers_and_does_not_show() {
        let now = Instant::now();
        let mut stack = Stack::default();
        push(&mut stack, "before", Urgency::Normal, -1, 0, now);
        stack.quiet = true;
        let hidden = push(&mut stack, "quietly", Urgency::Normal, -1, 0, now);
        let urgent = push(&mut stack, "fire", Urgency::Critical, -1, 0, now);
        let shown: Vec<&str> = stack
            .shown()
            .iter()
            .map(|item| item.summary.as_str())
            .collect();
        assert_eq!(shown, ["before", "fire"]);
        assert!(
            hidden != 0 && urgent != hidden,
            "what is not shown still has a number"
        );
        // All three are remembered, the newest first.
        let past: Vec<&str> = stack
            .history()
            .iter()
            .map(|item| item.summary.as_str())
            .collect();
        assert_eq!(past, ["fire", "quietly", "before"]);
        // A replaced one is remembered once, as it is now.
        push(&mut stack, "before, changed", Urgency::Normal, -1, 1, now);
        let past: Vec<&str> = stack
            .history()
            .iter()
            .map(|item| item.summary.as_str())
            .collect();
        assert_eq!(past, ["before, changed", "fire", "quietly"]);
        for index in 0..REMEMBERED + 10 {
            push(&mut stack, &format!("n{index}"), Urgency::Low, -1, 0, now);
        }
        assert_eq!(stack.history().len(), REMEMBERED);
        stack.forget_history();
        assert!(stack.history().is_empty());
    }

    #[test]
    fn gevreks_own_words_are_text_as_they_stand() {
        let now = Instant::now();
        let mut stack = Stack::default();
        let id = stack.tell("gevrek", "a < b", "and b > c", 2000, now);
        assert_eq!(stack.shown()[0].summary, "a < b");
        assert_eq!(stack.shown()[0].body, "and b > c");
        assert_eq!(stack.history()[0].id, id);
        // It goes by itself after its time, and stays in what arrived.
        assert_eq!(stack.expire(now + Duration::from_millis(2100)), [id]);
        assert!(stack.is_empty());
        assert_eq!(stack.history().len(), 1);
    }

    #[test]
    fn the_cards_and_the_centre_are_drawn_and_ask_for_nothing_by_themselves() {
        let now = Instant::now();
        let mut stack = Stack::default();
        for index in 0..7 {
            stack.tell("gevrek", &format!("n{index}"), "some words", 0, now);
        }
        assert_eq!(height(&stack), SHOWN as u32 * (NOTE_HEIGHT + 8) + 22);
        assert_eq!(height(&Stack::default()), 0);
        let context = egui::Context::default();
        let palette = gevrek_ui::Settings::default()
            .resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 700.0),
            )),
            ..Default::default()
        };
        let mut gone = None;
        let mut output = context.run_ui(input(), |ui| {
            gone = draw_cards(ui, &palette, &stack);
        });
        output.textures_delta.clear();
        assert_eq!(gone, None);
        let mut asked = None;
        let mut output = context.run_ui(input(), |ui| {
            asked = Some(draw_centre(
                ui,
                &palette,
                &stack,
                now,
                false,
                &crate::frost::Look::plain(),
            ));
        });
        output.textures_delta.clear();
        let asked = asked.expect("drawn");
        assert!(!asked.close && !asked.forget && !asked.quiet);
        // The button that forgets is on the surface, for who looks for it:
        // its foot, as wide as what is above it.
        let clear = asked.clear.expect("the button");
        assert!(
            clear.min.x >= 18.0 && clear.max.x <= 382.0 && clear.min.y > 600.0 && clear.max.y <= 700.0,
            "{clear:?}"
        );
    }

    #[test]
    fn markup_is_taken_out_and_actions_come_in_pairs() {
        assert_eq!(
            plain("<b>Done</b>: 3 &lt; 5 &amp; <a href=\"x\">more</a>"),
            "Done: 3 < 5 & more"
        );
        let mut stack = Stack::default();
        let actions = vec![
            "default".to_string(),
            "Open".to_string(),
            "snooze".to_string(),
            "Later".to_string(),
            "odd".to_string(),
        ];
        stack.push(
            "app",
            0,
            "s",
            "b",
            &actions,
            Urgency::Normal,
            -1,
            Instant::now(),
        );
        assert_eq!(
            stack.shown()[0].actions,
            [
                ("default".to_string(), "Open".to_string()),
                ("snooze".to_string(), "Later".to_string())
            ]
        );
    }

    #[test]
    fn how_many_are_shown_and_how_long_follows_the_configuration() {
        let mut stack = Stack::default();
        stack.configure(2, Duration::from_secs(20));
        let start = Instant::now();
        for index in 0..4 {
            push(
                &mut stack,
                &format!("note {index}"),
                Urgency::Normal,
                -1,
                0,
                start,
            );
        }
        assert_eq!(stack.shown().len(), 2);
        assert_eq!(stack.waiting(), 2);
        assert_eq!(stack.next_expiry(), Some(start + Duration::from_secs(20)));
    }
}
