//! gevrek's backend for the desktop portals.
//!
//! Applications (above all sandboxed ones) ask the desktop for things
//! through `xdg-desktop-portal`, which passes each kind of request on to
//! the desktop's own backend. This is gevrek's, on the session's bus as
//! `org.freedesktop.impl.portal.desktop.gevrek`.
//!
//! It answers four kinds. **Settings**: an application reads whether the
//! desktop is dark or light and what its accent colour is, and is told
//! when that changes. **Notification**: what a sandboxed application
//! sends is shown on the same cards as every other notification.
//! **Wallpaper**: an application sets the desktop's picture. **Inhibit**:
//! an application keeps the screen from going dark and locking.
//! **ScreenCast**: an application asks for a display or a window as a
//! stream of pictures; the user chooses which in the shell's chooser, and
//! the application is told the stream's node in PipeWire
//! (`screencast.rs`). **Screenshot**: an application asks for a picture
//! of the screen (at once, or of what the user chooses on it) or for a
//! colour the user picks from it. The rest (the file chooser) is still
//! another backend's, as `gevrek-portals.conf` says.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::zvariant::{OwnedValue, Value};

pub const NAME: &str = "org.freedesktop.impl.portal.desktop.gevrek";
const PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";

/// What applications are told about the desktop's look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Movement is to be kept to what is needed
    /// (`desktop.animations.reduce_motion`).
    pub reduce_motion: bool,
    pub dark: bool,
    /// More contrast is wanted (`desktop.appearance.high_contrast`).
    pub contrast: bool,
    /// Red, green and blue, 0 to 1 each.
    pub accent: (f64, f64, f64),
}

impl Look {
    /// The settings of the appearance namespace, by their names.
    fn settings(&self) -> Vec<(&'static str, OwnedValue)> {
        let own = |value: Value<'_>| value.try_to_owned().expect("a plain value");
        vec![
            // 1: prefer dark, 2: prefer light.
            (
                "color-scheme",
                own(Value::from(if self.dark { 1u32 } else { 2u32 })),
            ),
            ("accent-color", own(Value::from(self.accent))),
            // 0: no wish for more contrast; 1: more.
            ("contrast", own(Value::from(u32::from(self.contrast)))),
            // 1: as little movement as can be; 0: no such wish.
            (
                "reduced-motion",
                own(Value::from(u32::from(self.reduce_motion))),
            ),
        ]
    }
}

struct Settings {
    look: Arc<Mutex<Look>>,
}

/// What an application asks of the desktop through the portal.
#[derive(Debug, Clone, PartialEq)]
pub enum Asked {
    /// Show a notification. `key` is the same for the same notification
    /// of the same application: sent again, it takes the first one's place.
    Notify {
        key: String,
        app: String,
        title: String,
        body: String,
        urgency: crate::notifications::Urgency,
        /// What can be done about it: the application's name for each,
        /// and its words.
        buttons: Vec<(String, String)>,
        /// What a press on the notification itself is, in the
        /// application's own words.
        default_action: Option<String>,
        picture: Option<crate::notifications::Picture>,
    },
    /// Take a notification back.
    Withdraw(String),
    /// Make this file the wallpaper.
    Wallpaper(std::path::PathBuf),
    /// Keep the screen awake (or stop keeping it) for a request, which
    /// is known by its name.
    Awake(String, bool),
    /// An application wants a display or a window shared with it: the
    /// user is to choose which, or to refuse.
    Share(ShareAsk),
    /// The sharing of this session is over (its application ended it).
    ShareEnd(String),
    /// An application wants a picture of the screen.
    Picture(PictureAsk),
    /// An application wants a colour the user picks from the screen.
    Colour(ColourAsk),
    /// An application wants to move the pointer and type: the user is
    /// to allow it, or to refuse.
    Control(ControlAsk),
    /// An application wants keys of its own on the desktop: the user is
    /// to allow them, or to refuse.
    Shortcuts(ShortcutsAsk),
    /// The shortcuts an application has: asked for.
    ShortcutsList(ShortcutsAsk),
    /// The user would change an application's shortcuts: where the
    /// bindings are changed is opened.
    ShortcutsConfigure,
}

impl Asked {
    /// What is asked, and by whom, as the log says it: never what a
    /// notification says.
    pub fn told(&self) -> String {
        match self {
            Asked::Notify { app, .. } => format!("to show a notification of {app}"),
            Asked::Withdraw(_) => "to take a notification back".to_string(),
            Asked::Wallpaper(file) => format!("for the wallpaper {}", file.display()),
            Asked::Awake(asker, true) => format!("to keep the screen awake for {asker}"),
            Asked::Awake(asker, false) => format!("no longer to keep the screen awake for {asker}"),
            Asked::Share(ask) => format!("to share the screen with {}", ask.app),
            Asked::ShareEnd(_) => "to end a sharing of the screen".to_string(),
            Asked::Picture(ask) => format!(
                "for a picture of the screen{}",
                if ask.interactive { ", chosen by the person" } else { "" }
            ),
            Asked::Colour(_) => "for a colour from the screen".to_string(),
            Asked::Control(ask) => format!("to let {} control the desktop", ask.app),
            Asked::Shortcuts(ask) => {
                format!("for {} shortcuts of {}", ask.wanted.len(), ask.app)
            }
            Asked::ShortcutsList(ask) => format!("which shortcuts {} has", ask.app),
            Asked::ShortcutsConfigure => "to change an application's shortcuts".to_string(),
        }
    }
}

/// A request about an application's shortcuts.
#[derive(Debug, Clone)]
pub struct ShortcutsAsk {
    pub app: String,
    /// The ones it asks for (none: it only asks which it has).
    pub wanted: Vec<crate::app_shortcuts::Wanted>,
    /// The ones that are bound afterwards: the application's name for
    /// each, what it does, and its keys as they are read. `None`: the
    /// user refused.
    pub answer: async_channel::Sender<Option<Vec<(String, String, String)>>>,
}

impl PartialEq for ShortcutsAsk {
    fn eq(&self, other: &Self) -> bool {
        self.app == other.app && self.wanted == other.wanted
    }
}

/// Shortcuts, as the portal has them told: each with its description
/// and its keys in words.
fn shortcuts_told(bound: &[(String, String, String)]) -> zbus::fdo::Result<OwnedValue> {
    let told: Vec<(String, HashMap<String, Value<'_>>)> = bound
        .iter()
        .map(|(id, description, keys)| {
            (
                id.clone(),
                HashMap::from([
                    ("description".to_string(), Value::from(description.clone())),
                    ("trigger_description".to_string(), Value::from(keys.clone())),
                ]),
            )
        })
        .collect();
    Value::from(told)
        .try_to_owned()
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

/// The GlobalShortcuts portal: an application's own keys on the
/// desktop, as bindings of gevrek's (`app_shortcuts.rs`).
struct GlobalShortcuts {
    tell: Tell,
    /// The sessions, each with its application.
    sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl GlobalShortcuts {
    fn app_of(&self, session: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session)
            .cloned()
    }

    async fn asked(
        &self,
        list: bool,
        app: String,
        wanted: Vec<crate::app_shortcuts::Wanted>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let (answer, answered) = async_channel::bounded(1);
        let ask = ShortcutsAsk {
            app,
            wanted,
            answer,
        };
        (self.tell)(if list {
            Asked::ShortcutsList(ask)
        } else {
            Asked::Shortcuts(ask)
        });
        match answered.recv().await {
            Ok(Some(bound)) => Ok((
                0,
                HashMap::from([("shortcuts".to_string(), shortcuts_told(&bound)?)]),
            )),
            Ok(None) => Ok((1, HashMap::new())),
            Err(_) => Ok((2, HashMap::new())),
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl GlobalShortcuts {
    async fn create_session(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let name = session_handle.to_string();
        server
            .at(
                &session_handle,
                ShortcutSession {
                    sessions: Arc::clone(&self.sessions),
                    name: name.clone(),
                },
            )
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, app_id.to_string());
        Ok((0, HashMap::new()))
    }

    /// The application names what it would have keys for; the user is
    /// asked about those that are not bound yet. 1: refused.
    async fn bind_shortcuts(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let Some(app) = self.app_of(session_handle.as_str()) else {
            return Ok((2, HashMap::new()));
        };
        let text = |said: &HashMap<String, OwnedValue>, key: &str| {
            said.get(key)
                .and_then(|value| <&str>::try_from(value).ok())
                .map(str::to_string)
        };
        let wanted = shortcuts
            .iter()
            .map(|(id, said)| crate::app_shortcuts::Wanted {
                id: id.clone(),
                description: text(said, "description").unwrap_or_default(),
                trigger: text(said, "preferred_trigger").filter(|keys| !keys.is_empty()),
            })
            .collect();
        self.asked(false, app, wanted).await
    }

    async fn list_shortcuts(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let Some(app) = self.app_of(session_handle.as_str()) else {
            return Ok((2, HashMap::new()));
        };
        self.asked(true, app, Vec::new()).await
    }

    /// The user would change the keys: that is done where every binding
    /// is changed.
    fn configure_shortcuts(
        &self,
        _session_handle: zbus::zvariant::ObjectPath<'_>,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) {
        (self.tell)(Asked::ShortcutsConfigure);
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

/// A session of an application's shortcuts, on the bus until it is closed.
struct ShortcutSession {
    sessions: Arc<Mutex<HashMap<String, String>>>,
    name: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl ShortcutSession {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.name);
        if let Err(error) = server.remove::<Self, _>(self.name.as_str()).await {
            log::debug!("a portal session {} was not taken off the bus: {error}", self.name);
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// A request for a picture of the screen.
#[derive(Debug, Clone)]
pub struct PictureAsk {
    /// The user is to choose what of the screen (a part dragged out, or
    /// all of it); otherwise the display that has the keyboard is taken
    /// at once.
    pub interactive: bool,
    /// The file the picture was written to; `None`: the user left it.
    pub answer: async_channel::Sender<Option<std::path::PathBuf>>,
}

impl PartialEq for PictureAsk {
    fn eq(&self, other: &Self) -> bool {
        self.interactive == other.interactive
    }
}

/// A request for a colour picked from the screen.
#[derive(Debug, Clone)]
pub struct ColourAsk {
    /// Red, green and blue, 0 to 1 each; `None`: the user left it.
    pub answer: async_channel::Sender<Option<(f64, f64, f64)>>,
}

impl PartialEq for ColourAsk {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A file's place as applications are told it (`file:///...`): what is
/// no letter, digit or one of `-._~/` is written as its bytes.
pub fn uri_of(file: &std::path::Path) -> String {
    let mut uri = String::from("file://");
    for byte in file.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The RemoteDesktop portal: an application that the user lets do so
/// moves the pointer and types, and may see the screen with it (the
/// session is then a screen-sharing one as well).
struct RemoteDesktop {
    tell: Tell,
    sessions: Arc<Mutex<HashMap<String, Wanted>>>,
    /// The made-up pointer and keyboard (`remote.rs`), once a session
    /// was let control.
    devices: Arc<Mutex<Option<std::sync::mpsc::Sender<crate::remote::Deed>>>>,
}

impl RemoteDesktop {
    /// Do what a session did, where it was let control such a device.
    fn act(&self, session: &str, device: u32, deed: crate::remote::Deed) -> zbus::fdo::Result<()> {
        let granted = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session)
            .map_or(0, |session| session.granted);
        if granted & device == 0 {
            return Err(zbus::fdo::Error::AccessDenied(
                "This session was not let control that device".to_string(),
            ));
        }
        if let Some(devices) = self
            .devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            let _ = devices.send(deed);
        }
        Ok(())
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktop {
    async fn create_session(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let name = session_handle.to_string();
        server
            .at(
                &session_handle,
                CastSession {
                    tell: Arc::clone(&self.tell),
                    sessions: Arc::clone(&self.sessions),
                    name: name.clone(),
                },
            )
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, Wanted::default());
        Ok((0, HashMap::new()))
    }

    /// What the application would control: the keyboard, the pointer.
    async fn select_devices(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        match self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(session_handle.as_str())
        {
            Some(kept) => kept.devices = devices_of(number_of(&options, "types")),
            None => return Ok((2, HashMap::new())),
        }
        Ok((0, HashMap::new()))
    }

    /// The user is asked whether the application may control the
    /// desktop (every time: that is never remembered), and, where the
    /// session is to see the screen too, what of it. 1: refused.
    async fn start(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let asked = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_handle.as_str())
            .cloned();
        let Some(asked) = asked else {
            return Ok((2, HashMap::new()));
        };
        let devices = devices_of(Some(asked.devices));
        let (answer, allowed) = async_channel::bounded(1);
        (self.tell)(Asked::Control(ControlAsk {
            app: app_id.to_string(),
            devices,
            answer,
        }));
        if !allowed.recv().await.unwrap_or(false) {
            return Ok((1, HashMap::new()));
        }
        // The pointer and the keyboard that are made up for it.
        {
            let mut made = self
                .devices
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if made.is_none() {
                match crate::remote::spawn() {
                    Ok(devices) => *made = Some(devices),
                    Err(error) => {
                        log::warn!("the desktop cannot be controlled: {error:#}");
                        return Ok((2, HashMap::new()));
                    }
                }
            }
        }
        let broken = |error: zbus::zvariant::Error| zbus::fdo::Error::Failed(error.to_string());
        let mut results = HashMap::new();
        let mut outputs = Vec::new();
        if asked.selected {
            let (answer, chosen) = async_channel::bounded(1);
            (self.tell)(Asked::Share(ShareAsk {
                session: session_handle.to_string(),
                app: app_id.to_string(),
                displays: asked.displays,
                windows: asked.windows,
                pointer: asked.pointer,
                beside: asked.beside,
                several: asked.several,
                // (Asked every time, as the control is.)
                may_remember: false,
                remembered: Vec::new(),
                answer,
            }));
            let Ok(Some(answer)) = chosen.recv().await else {
                return Ok((1, HashMap::new()));
            };
            results.insert("streams".to_string(), streams_of(&answer.streams)?);
            outputs = answer
                .streams
                .iter()
                .filter_map(|stream| Some((stream.node, stream.output.clone()?, stream.size)))
                .collect();
        }
        match self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(session_handle.as_str())
        {
            Some(kept) => {
                kept.granted = devices;
                kept.outputs = outputs;
            }
            // (Closed while the user was asked.)
            None => return Ok((2, HashMap::new())),
        }
        results.insert(
            "devices".to_string(),
            Value::from(devices).try_to_owned().map_err(broken)?,
        );
        results.insert(
            "clipboard_enabled".to_string(),
            Value::from(false).try_to_owned().map_err(broken)?,
        );
        Ok((0, results))
    }

    async fn notify_pointer_motion(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        self.act(
            session_handle.as_str(),
            POINTER,
            crate::remote::Deed::Motion(dx, dy),
        )
    }

    async fn notify_pointer_motion_absolute(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        stream: u32,
        x: f64,
        y: f64,
    ) -> zbus::fdo::Result<()> {
        let place = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_handle.as_str())
            .and_then(|session| {
                session
                    .outputs
                    .iter()
                    .find(|(node, ..)| *node == stream)
                    .cloned()
            });
        let Some((_, output, size)) = place else {
            return Err(zbus::fdo::Error::InvalidArgs(
                "No display is shared with this session as that stream".to_string(),
            ));
        };
        self.act(
            session_handle.as_str(),
            POINTER,
            crate::remote::Deed::Place {
                output,
                at: (x, y),
                size,
            },
        )
    }

    async fn notify_pointer_button(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        button: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.act(
            session_handle.as_str(),
            POINTER,
            crate::remote::Deed::Button(button.max(0) as u32, state != 0),
        )
    }

    async fn notify_pointer_axis(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        self.act(
            session_handle.as_str(),
            POINTER,
            crate::remote::Deed::Turn(dx, dy),
        )
    }

    async fn notify_pointer_axis_discrete(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        axis: u32,
        steps: i32,
    ) -> zbus::fdo::Result<()> {
        self.act(
            session_handle.as_str(),
            POINTER,
            crate::remote::Deed::Notches(axis, steps),
        )
    }

    async fn notify_keyboard_keycode(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        keycode: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.act(
            session_handle.as_str(),
            KEYBOARD,
            crate::remote::Deed::Key(keycode.max(0) as u32, state != 0),
        )
    }

    async fn notify_keyboard_keysym(
        &self,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        self.act(
            session_handle.as_str(),
            KEYBOARD,
            crate::remote::Deed::Sym(keysym.max(0) as u32, state != 0),
        )
    }

    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        KEYBOARD | POINTER
    }

    /// (1: without `ConnectToEIS`, which gevrek does not have.)
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// The applications that have windows, each with how it stands: 1 it
/// has a window, 2 one of its windows has the keyboard. (An application
/// without any is not known here: the portal takes it for one in the
/// background.) From the window manager's state.
pub fn apps_of(state: &serde_json::Value) -> HashMap<String, u32> {
    let mut apps: HashMap<String, u32> = HashMap::new();
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
            for window in workspace["detailed_windows"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if window["is_border_window"].as_bool().unwrap_or(false) {
                    continue;
                }
                let id = ["app_id", "class"]
                    .iter()
                    .filter_map(|key| window[*key].as_str())
                    .find(|id| !id.is_empty());
                let Some(id) = id else { continue };
                let stands = if window["is_focused"].as_bool().unwrap_or(false) {
                    2
                } else {
                    1
                };
                let known = apps.entry(id.to_string()).or_insert(stands);
                *known = (*known).max(stands);
            }
        }
    }
    apps
}

/// An entry that starts an application with the session, as a file of
/// the autostart folder says it.
pub fn autostart_entry(app: &str, commandline: &[String], activated_by_bus: bool) -> String {
    // (A word with a space or a mark the entry's reader takes for its
    // own is put in quotes, its quotes and backslashes marked.)
    let word = |word: &String| {
        if !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
        {
            word.clone()
        } else {
            let mut quoted = String::from("\"");
            for c in word.chars() {
                if matches!(c, '"' | '\\' | '`' | '$') {
                    quoted.push('\\');
                }
                quoted.push(c);
            }
            quoted.push('"');
            quoted
        }
    };
    let exec = commandline.iter().map(word).collect::<Vec<_>>().join(" ");
    let mut entry =
        format!("[Desktop Entry]\nType=Application\nName={app}\nExec={exec}\nX-Flatpak={app}\n");
    if activated_by_bus {
        entry.push_str("DBusActivatable=true\n");
    }
    entry
}

/// Where the session's autostart entries are: `autostart` in the
/// user's configuration folder.
fn autostart_folder() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|folder| !folder.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
        })
        .map(|folder| folder.join("autostart"))
}

/// The Background portal: which applications have windows, a word to
/// the user when one goes on without any, and an application started
/// with the session where its user asked it to be.
struct Background {
    tell: Tell,
    apps: Arc<Mutex<HashMap<String, u32>>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Background")]
impl Background {
    fn get_app_state(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        self.apps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(app, stands)| {
                Value::from(*stands)
                    .try_to_owned()
                    .map(|stands| (app.clone(), stands))
                    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
            })
            .collect()
    }

    /// An application goes on running with no window: the user is told
    /// so, and it is let (1). (Forbidding one from here is not there:
    /// its own window or the system's tools end it.)
    fn notify_background(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        app_id: &str,
        name: &str,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let who = if name.is_empty() { app_id } else { name };
        (self.tell)(Asked::Notify {
            key: format!("background:{app_id}"),
            app: who.to_string(),
            title: format!("{who} runs in the background"),
            body: "It has no window open and goes on running.".to_string(),
            urgency: crate::notifications::Urgency::Low,
            buttons: Vec::new(),
            default_action: None,
            picture: None,
        });
        let allow = Value::from(1u32)
            .try_to_owned()
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Ok((0, HashMap::from([("result".to_string(), allow)])))
    }

    /// Start an application with the session from now on, or no longer:
    /// its entry in the autostart folder is written or removed. Returns
    /// whether it is started with the session afterwards.
    fn enable_autostart(
        &self,
        app_id: &str,
        enable: bool,
        commandline: Vec<String>,
        flags: u32,
    ) -> bool {
        // (A name that would leave the folder is no application's.)
        if app_id.is_empty() || app_id.contains('/') || app_id.starts_with('.') {
            return false;
        }
        // (Nor one with a line's end in it, or a word of its command with
        // one: an entry is read line by line, and what stood after the
        // break would be a line of its own, a second `Exec=` among them.)
        if app_id.chars().any(char::is_control)
            || commandline
                .iter()
                .any(|word| word.chars().any(char::is_control))
        {
            log::warn!("no autostart entry for {app_id:?}: its name or its command has a control character");
            return false;
        }
        let Some(folder) = autostart_folder() else {
            return false;
        };
        let file = folder.join(format!("{app_id}.desktop"));
        if !enable {
            match std::fs::remove_file(&file) {
                Ok(()) => log::debug!("{app_id} no longer starts with the session"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::debug!("{} was not removed: {error}", file.display()),
            }
            return false;
        }
        if commandline.is_empty() {
            return false;
        }
        let written = std::fs::create_dir_all(&folder).and_then(|()| {
            std::fs::write(&file, autostart_entry(app_id, &commandline, flags & 1 != 0))
        });
        match written {
            Ok(()) => true,
            Err(error) => {
                log::warn!("{}: {error}", file.display());
                false
            }
        }
    }

    #[zbus(signal)]
    async fn running_applications_changed(
        emitter: &zbus::object_server::SignalContext<'_>,
    ) -> zbus::Result<()>;
}

struct Screenshot {
    tell: Tell,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    /// A picture of the screen, written to a file the application is
    /// told. The answer waits for the user where they are to choose;
    /// 1 says that they left it, 2 that no picture could be had.
    async fn screenshot(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let interactive = options
            .get("interactive")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        let (answer, answered) = async_channel::bounded(1);
        (self.tell)(Asked::Picture(PictureAsk {
            interactive,
            answer,
        }));
        match answered.recv().await {
            Ok(Some(file)) => {
                let uri = Value::from(uri_of(&file))
                    .try_to_owned()
                    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
                Ok((0, HashMap::from([("uri".to_string(), uri)])))
            }
            Ok(None) => Ok((1, HashMap::new())),
            Err(_) => Ok((2, HashMap::new())),
        }
    }

    /// A colour the user picks from the screen: red, green and blue.
    async fn pick_color(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let (answer, answered) = async_channel::bounded(1);
        (self.tell)(Asked::Colour(ColourAsk { answer }));
        match answered.recv().await {
            Ok(Some(colour)) => {
                let colour = Value::from(colour)
                    .try_to_owned()
                    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
                Ok((0, HashMap::from([("color".to_string(), colour)])))
            }
            Ok(None) => Ok((1, HashMap::new())),
            Err(_) => Ok((2, HashMap::new())),
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

/// A request to share the screen, waiting for the user's choice.
#[derive(Debug, Clone)]
pub struct ShareAsk {
    /// The portal's session the sharing belongs to.
    pub session: String,
    /// The application that asks; empty where the portal does not know.
    pub app: String,
    /// Displays may be chosen.
    pub displays: bool,
    /// Windows may be chosen.
    pub windows: bool,
    /// The pointer is to be in the pictures.
    pub pointer: bool,
    /// The pointer is to go beside the pictures (where it is, and its
    /// own picture), not into them.
    pub beside: bool,
    /// More than one thing may be chosen.
    pub several: bool,
    /// The application would have the choice remembered, so that it
    /// need not ask again.
    pub may_remember: bool,
    /// The displays that were chosen and remembered for this
    /// application before: shared again without asking, while every one
    /// of them is there. Empty: the user is asked.
    pub remembered: Vec<String>,
    /// What was chosen and is shared now; `None`: refused.
    pub answer: async_channel::Sender<Option<Answer>>,
}

/// The user's answer to a request to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The streams, one for each thing that is shared.
    pub streams: Vec<Shared>,
    /// The displays to share again without asking, when the user said
    /// that the choice is to be remembered.
    pub remember: Option<Vec<String>>,
}

/// What a remembered choice is known by among the portal's stores of
/// such, and the version of how it is written.
const REMEMBERED_BY: &str = "gevrek";
const REMEMBERED_VERSION: u32 = 1;

/// The displays of a remembered choice, from what the portal kept for
/// the application (`restore_data`): none where it is another desktop's,
/// or written another way.
pub fn remembered(kept: Option<&OwnedValue>) -> Vec<String> {
    let Some(Ok((by, version, displays))) = kept
        .and_then(|value| value.try_clone().ok())
        .map(<(String, u32, OwnedValue)>::try_from)
    else {
        return Vec::new();
    };
    if by != REMEMBERED_BY || version != REMEMBERED_VERSION {
        return Vec::new();
    }
    // (The third part is a value of any kind: the list is inside it.)
    let mut inside: &Value<'_> = &displays;
    while let Value::Value(deeper) = inside {
        inside = deeper;
    }
    inside
        .try_clone()
        .ok()
        .and_then(|list| Vec::<String>::try_from(list).ok())
        .unwrap_or_default()
}

/// A choice to be remembered, as the portal keeps it.
fn to_remember(displays: &[String]) -> zbus::fdo::Result<OwnedValue> {
    Value::from((
        REMEMBERED_BY,
        REMEMBERED_VERSION,
        Value::from(displays.to_vec()),
    ))
    .try_to_owned()
    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

impl PartialEq for ShareAsk {
    /// The same request: of the same session.
    fn eq(&self, other: &Self) -> bool {
        self.session == other.session
    }
}

/// A stream that an application is told of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shared {
    /// The stream's node in PipeWire.
    pub node: u32,
    pub size: (u32, u32),
    /// A display (else a window).
    pub display: bool,
    /// The display's name, where it is one.
    pub output: Option<String>,
}

/// The devices an application may ask to control, in the portal's bits.
pub const KEYBOARD: u32 = 1;
pub const POINTER: u32 = 2;

/// A request to control the desktop, waiting for the user's word.
#[derive(Debug, Clone)]
pub struct ControlAsk {
    /// The application that asks; empty where the portal does not know.
    pub app: String,
    /// What it asks for: `KEYBOARD`, `POINTER`, or both.
    pub devices: u32,
    /// Whether the user lets it.
    pub answer: async_channel::Sender<bool>,
}

impl PartialEq for ControlAsk {
    fn eq(&self, other: &Self) -> bool {
        self.app == other.app && self.devices == other.devices
    }
}

/// The devices of a `SelectDevices`, from its option: what gevrek can
/// make up of what is asked for (nothing said: both).
pub fn devices_of(types: Option<u32>) -> u32 {
    match types.map(|types| types & (KEYBOARD | POINTER)) {
        Some(0) | None => KEYBOARD | POINTER,
        Some(types) => types,
    }
}

/// The kinds of sources, in the portal's bits.
const DISPLAYS: u32 = 1;
const WINDOWS: u32 = 2;
/// The pointer's ways: not in the pictures, drawn into them, or handed
/// on beside them (where it is, and its own picture).
const POINTER_HIDDEN: u32 = 1;
const POINTER_DRAWN: u32 = 2;
const POINTER_BESIDE: u32 = 4;

/// What an application chose for a session before it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub displays: bool,
    pub windows: bool,
    pub pointer: bool,
    /// The pointer goes beside the pictures, not into them.
    pub beside: bool,
    pub several: bool,
    /// How long a choice may be remembered, in the portal's numbers: 0
    /// not, 1 while the application runs, 2 until it is taken back.
    pub persist: u32,
    pub remembered: Vec<String>,
    /// Sources were chosen for the session at all (`SelectSources`): a
    /// session that controls the desktop need not see it.
    pub selected: bool,
    /// The devices the application asks to control (`SelectDevices`).
    pub devices: u32,
    /// The devices the user let it control.
    pub granted: u32,
    /// The displays that are shared with it: the stream's node, the
    /// display's name and the stream's size (where the pointer is put
    /// at a place of a stream).
    pub outputs: Vec<(u32, String, (u32, u32))>,
}

impl Default for Wanted {
    /// What the portal says holds where nothing is said: displays, and
    /// the pointer not in the pictures.
    fn default() -> Self {
        Self {
            displays: true,
            windows: false,
            pointer: false,
            beside: false,
            several: false,
            persist: 0,
            remembered: Vec::new(),
            selected: false,
            devices: 0,
            granted: 0,
            outputs: Vec::new(),
        }
    }
}

/// What a `SelectSources` asks for, from its options.
pub fn wanted(types: Option<u32>, cursor_mode: Option<u32>) -> Wanted {
    let types = types.filter(|types| types & (DISPLAYS | WINDOWS) != 0);
    Wanted {
        displays: types.is_none_or(|types| types & DISPLAYS != 0),
        windows: types.is_some_and(|types| types & WINDOWS != 0),
        pointer: cursor_mode == Some(POINTER_DRAWN),
        beside: cursor_mode == Some(POINTER_BESIDE),
        ..Wanted::default()
    }
}

/// The streams of an answer to `Start`, in the portal's words: each
/// with its node, its size and what kind of thing it shows.
fn streams_of(shared: &[Shared]) -> zbus::fdo::Result<OwnedValue> {
    let broken = |error: zbus::zvariant::Error| zbus::fdo::Error::Failed(error.to_string());
    let streams: Vec<(u32, HashMap<String, Value<'_>>)> = shared
        .iter()
        .map(|shared| {
            let mut about: HashMap<String, Value<'_>> = HashMap::new();
            about.insert(
                "size".to_string(),
                Value::from((shared.size.0 as i32, shared.size.1 as i32)),
            );
            about.insert(
                "source_type".to_string(),
                Value::from(if shared.display { DISPLAYS } else { WINDOWS }),
            );
            (shared.node, about)
        })
        .collect();
    Value::from(streams).try_to_owned().map_err(broken)
}

struct ScreenCast {
    tell: Tell,
    /// What each session asked for, by the session's name.
    sessions: Arc<Mutex<HashMap<String, Wanted>>>,
}

fn number_of(options: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    options.get(key).and_then(|value| u32::try_from(value).ok())
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    /// A session begins: it is an object of its own on the bus, which
    /// the portal closes when the application is done.
    async fn create_session(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let name = session_handle.to_string();
        server
            .at(
                &session_handle,
                CastSession {
                    tell: Arc::clone(&self.tell),
                    sessions: Arc::clone(&self.sessions),
                    name: name.clone(),
                },
            )
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name, Wanted::default());
        Ok((0, HashMap::new()))
    }

    /// What the application would take: displays, windows, the pointer.
    async fn select_sources(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let asked = Wanted {
            selected: true,
            several: options
                .get("multiple")
                .and_then(|value| bool::try_from(value).ok())
                .unwrap_or(false),
            persist: number_of(&options, "persist_mode").unwrap_or(0).min(2),
            remembered: remembered(options.get("restore_data")),
            ..wanted(
                number_of(&options, "types"),
                number_of(&options, "cursor_mode"),
            )
        };
        match self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(session_handle.as_str())
        {
            // (What a session that controls the desktop asked for stays.)
            Some(kept) => {
                *kept = Wanted {
                    devices: kept.devices,
                    granted: kept.granted,
                    ..asked
                }
            }
            None => return Ok((2, HashMap::new())),
        }
        Ok((0, HashMap::new()))
    }

    /// The user chooses, and the application is told the stream. The
    /// answer waits for the choice; 1 says that the user refused.
    async fn start(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        session_handle: zbus::zvariant::ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let asked = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_handle.as_str())
            .cloned();
        let Some(asked) = asked else {
            return Ok((2, HashMap::new()));
        };
        let (answer, chosen) = async_channel::bounded(1);
        (self.tell)(Asked::Share(ShareAsk {
            session: session_handle.to_string(),
            app: app_id.to_string(),
            displays: asked.displays,
            windows: asked.windows,
            pointer: asked.pointer,
            beside: asked.beside,
            several: asked.several,
            may_remember: asked.persist > 0,
            remembered: asked.remembered,
            answer,
        }));
        let Ok(Some(answer)) = chosen.recv().await else {
            return Ok((1, HashMap::new()));
        };
        let mut results = HashMap::new();
        results.insert("streams".to_string(), streams_of(&answer.streams)?);
        // Remembered only where the user said so, and for as long as the
        // application asked; else the user is asked again next time.
        let kept = answer.remember.filter(|_| asked.persist > 0);
        results.insert(
            "persist_mode".to_string(),
            Value::from(if kept.is_some() { asked.persist } else { 0 })
                .try_to_owned()
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?,
        );
        if let Some(displays) = kept {
            results.insert("restore_data".to_string(), to_remember(&displays)?);
        }
        Ok((0, results))
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        DISPLAYS | WINDOWS
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        POINTER_HIDDEN | POINTER_DRAWN | POINTER_BESIDE
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

/// A session of screen sharing on the bus: closed, the sharing ends.
struct CastSession {
    tell: Tell,
    sessions: Arc<Mutex<HashMap<String, Wanted>>>,
    name: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl CastSession {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.name);
        (self.tell)(Asked::ShareEnd(self.name.clone()));
        if let Err(error) = server.remove::<Self, _>(self.name.as_str()).await {
            log::debug!("a portal session {} was not taken off the bus: {error}", self.name);
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// How pressing a notification of the portal is (its `priority`).
fn urgency_of(priority: Option<&str>) -> crate::notifications::Urgency {
    use crate::notifications::Urgency;
    match priority {
        Some("low") => Urgency::Low,
        Some("urgent") => Urgency::Critical,
        _ => Urgency::Normal,
    }
}

/// The file an address of the portal names: a `file://` address with
/// what is written as `%20` and the like read back. `None` for another
/// kind of address, or one that names no place from the root.
pub fn file_of(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // (An address may name the machine before the path: only this one.)
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut letters = path.bytes();
    while let Some(letter) = letters.next() {
        if letter != b'%' {
            bytes.push(letter);
            continue;
        }
        let pair = [letters.next()?, letters.next()?];
        bytes.push(u8::from_str_radix(std::str::from_utf8(&pair).ok()?, 16).ok()?);
    }
    String::from_utf8(bytes).ok().map(Into::into)
}

/// The picture a notification of the portal comes with: an icon by its
/// names (the first is taken), or a PNG file's bytes.
fn picture_of(icon: Option<&OwnedValue>) -> Option<crate::notifications::Picture> {
    use crate::notifications::{pixels_sent, Picture};
    let (kind, what) = <(String, OwnedValue)>::try_from(icon?.try_clone().ok()?).ok()?;
    match kind.as_str() {
        "themed" => Vec::<String>::try_from(what)
            .ok()?
            .into_iter()
            .next()
            .map(Picture::Named),
        "bytes" => {
            let bytes = Vec::<u8>::try_from(what).ok()?;
            let read = image::load_from_memory(&bytes).ok()?.to_rgba8();
            let (width, height) = (read.width() as i32, read.height() as i32);
            pixels_sent(width, height, width * 4, 4, read.as_raw()).map(Picture::Pixels)
        }
        _ => None,
    }
}

type Tell = Arc<dyn Fn(Asked) + Send + Sync>;

/// Of what an application asks to be held off, the part gevrek holds
/// off: the screen going dark and locking for want of input (8; the
/// others are logging out, another user and sleep).
const HOLDS_IDLE: u32 = 8;

struct Inhibit {
    tell: Tell,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    /// Hold something off until the request that is made here is closed.
    async fn inhibit(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        _window: &str,
        flags: u32,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        let name = handle.to_string();
        let holds = flags & HOLDS_IDLE != 0;
        server
            .at(
                &handle,
                Held {
                    tell: Arc::clone(&self.tell),
                    name: name.clone(),
                    holds,
                },
            )
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        if holds {
            (self.tell)(Asked::Awake(name, true));
        }
        Ok(())
    }
}

/// A request of the Inhibit portal while it holds: closed, it lets go.
struct Held {
    tell: Tell,
    name: String,
    holds: bool,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Held {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer) {
        if self.holds {
            (self.tell)(Asked::Awake(self.name.clone(), false));
        }
        if let Err(error) = server.remove::<Self, _>(self.name.as_str()).await {
            log::debug!("a portal session {} was not taken off the bus: {error}", self.name);
        }
    }
}

struct Notification {
    tell: Tell,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Notification")]
impl Notification {
    fn add_notification(&self, app_id: &str, id: &str, notification: HashMap<String, OwnedValue>) {
        let text = |name: &str| {
            notification
                .get(name)
                .and_then(|value| <&str>::try_from(value).ok())
                .map(str::to_string)
        };
        (self.tell)(Asked::Notify {
            key: format!("{app_id}\n{id}"),
            app: app_id.to_string(),
            title: text("title").unwrap_or_default(),
            body: text("body").unwrap_or_default(),
            urgency: urgency_of(text("priority").as_deref()),
            buttons: notification
                .get("buttons")
                .and_then(|value| value.try_clone().ok())
                .and_then(|value| Vec::<HashMap<String, OwnedValue>>::try_from(value).ok())
                .unwrap_or_default()
                .iter()
                .filter_map(|button| {
                    let word = |name: &str| {
                        button
                            .get(name)
                            .and_then(|value| <&str>::try_from(value).ok())
                            .map(str::to_string)
                    };
                    Some((word("action")?, word("label")?))
                })
                .collect(),
            default_action: text("default-action"),
            picture: picture_of(notification.get("icon")),
        });
    }

    fn remove_notification(&self, app_id: &str, id: &str) {
        (self.tell)(Asked::Withdraw(format!("{app_id}\n{id}")));
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
}

struct Wallpaper {
    tell: Tell,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl Wallpaper {
    /// The answer: 0 when the picture is taken, 2 when it cannot be.
    #[zbus(name = "SetWallpaperURI")]
    fn set_wallpaper_uri(
        &self,
        _handle: zbus::zvariant::ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        uri: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> u32 {
        match file_of(uri).filter(|file| file.is_file()) {
            Some(file) => {
                (self.tell)(Asked::Wallpaper(file));
                0
            }
            None => {
                log::warn!("an application asked for a wallpaper that is no file here: {uri}");
                2
            }
        }
    }
}

fn matches(wanted: &[String], namespace: &str) -> bool {
    wanted.is_empty()
        || wanted.iter().any(|pattern| {
            pattern.is_empty()
                || pattern == namespace
                // `org.freedesktop.*` is every namespace that starts so.
                || pattern.strip_suffix('*').is_some_and(|start| namespace.starts_with(start))
        })
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut all = HashMap::new();
        if matches(&namespaces, APPEARANCE) {
            let look = *self
                .look
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            all.insert(
                APPEARANCE.to_string(),
                look.settings()
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect(),
            );
        }
        all
    }

    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        let look = *self
            .look
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (namespace == APPEARANCE)
            .then(|| {
                look.settings()
                    .into_iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value)
            })
            .flatten()
            .ok_or_else(|| {
                zbus::fdo::Error::UnknownProperty(format!(
                    "{namespace} {key} is not a setting gevrek has"
                ))
            })
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
}

/// The running backend: tell it when the desktop's look changes.
pub struct Portal {
    connection: zbus::blocking::Connection,
    look: Arc<Mutex<Look>>,
    sessions: Arc<Mutex<HashMap<String, Wanted>>>,
    /// The applications that have windows (`apps_of`).
    apps: Arc<Mutex<HashMap<String, u32>>>,
    /// The sessions of applications' shortcuts, each with its application.
    shortcut_sessions: Arc<Mutex<HashMap<String, String>>>,
}

impl Portal {
    /// Answer on the session's bus as gevrek's portal backend.
    /// `tell` is called (on the bus's thread) with what applications ask.
    pub fn serve(look: Look, tell: impl Fn(Asked) + Send + Sync + 'static) -> anyhow::Result<Self> {
        let look = Arc::new(Mutex::new(look));
        let tell: Tell = Arc::new(move |asked: Asked| {
            log::debug!("the portal is asked {}", asked.told());
            tell(asked)
        });
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        let kept = Arc::clone(&sessions);
        let apps = Arc::new(Mutex::new(HashMap::new()));
        let shortcut_sessions = Arc::new(Mutex::new(HashMap::new()));
        let connection = zbus::blocking::connection::Builder::session()?
            .name(NAME)?
            .serve_at(
                PATH,
                Settings {
                    look: Arc::clone(&look),
                },
            )?
            .serve_at(
                PATH,
                Notification {
                    tell: Arc::clone(&tell),
                },
            )?
            .serve_at(
                PATH,
                Inhibit {
                    tell: Arc::clone(&tell),
                },
            )?
            .serve_at(
                PATH,
                ScreenCast {
                    tell: Arc::clone(&tell),
                    sessions: Arc::clone(&sessions),
                },
            )?
            .serve_at(
                PATH,
                Screenshot {
                    tell: Arc::clone(&tell),
                },
            )?
            .serve_at(
                PATH,
                GlobalShortcuts {
                    tell: Arc::clone(&tell),
                    sessions: Arc::clone(&shortcut_sessions),
                },
            )?
            .serve_at(
                PATH,
                Background {
                    tell: Arc::clone(&tell),
                    apps: Arc::clone(&apps),
                },
            )?
            .serve_at(
                PATH,
                RemoteDesktop {
                    tell: Arc::clone(&tell),
                    sessions: Arc::clone(&sessions),
                    devices: Arc::new(Mutex::new(None)),
                },
            )?
            .serve_at(PATH, Wallpaper { tell })?
            .build()
            .map_err(|error| {
                anyhow::anyhow!("the portal backend could not take its name on the bus: {error}")
            })?;
        Ok(Self {
            connection,
            look,
            sessions: kept,
            apps,
            shortcut_sessions,
        })
    }

    /// The keys of an application's shortcut went down, or were let go:
    /// every session of that application is told.
    pub fn shortcut(&self, app: &str, id: &str, released: bool) {
        let sessions: Vec<String> = self
            .shortcut_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|(_, of)| of.as_str() == app)
            .map(|(session, _)| session.clone())
            .collect();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        for session in sessions {
            let Ok(path) = zbus::zvariant::ObjectPath::try_from(session.as_str()) else {
                continue;
            };
            let sent = self.connection.emit_signal(
                Option::<&str>::None,
                PATH,
                "org.freedesktop.impl.portal.GlobalShortcuts",
                if released { "Deactivated" } else { "Activated" },
                &(path, id, time, HashMap::<String, Value<'_>>::new()),
            );
            if let Err(error) = sent {
                log::debug!("an application was not told of its shortcut: {error}");
            }
        }
    }

    /// The applications that have windows are these now: the portal is
    /// told when they are others than before.
    pub fn set_apps(&self, now: HashMap<String, u32>) {
        {
            let mut apps = self
                .apps
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *apps == now {
                return;
            }
            *apps = now;
        }
        let sent = self.connection.emit_signal(
            Option::<&str>::None,
            PATH,
            "org.freedesktop.impl.portal.Background",
            "RunningApplicationsChanged",
            &(),
        );
        if let Err(error) = sent {
            log::debug!("RunningApplicationsChanged: {error}");
        }
    }

    /// The sharing of a session ended by itself (the window was closed,
    /// the display taken away): the portal is told that the session is
    /// closed, and it is no longer on the bus.
    pub fn share_closed(&self, session: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session);
        let sent = self.connection.emit_signal(
            Option::<&str>::None,
            session,
            "org.freedesktop.impl.portal.Session",
            "Closed",
            &(),
        );
        if let Err(error) = sent {
            log::warn!("the portal was not told that the sharing ended: {error}");
        }
        if let Err(error) = self
            .connection
            .object_server()
            .remove::<CastSession, _>(session)
        {
            log::debug!("the sharing {session} was not taken off the bus: {error}");
        }
    }

    /// The user chose something a notification of the portal offers: its
    /// application is told, in its own words.
    pub fn chose(&self, app: &str, id: &str, action: &str) {
        let none: Vec<OwnedValue> = Vec::new();
        let sent = self.connection.emit_signal(
            Option::<&str>::None,
            PATH,
            "org.freedesktop.impl.portal.Notification",
            "ActionInvoked",
            &(app, id, action, none),
        );
        if let Err(error) = sent {
            log::warn!("{app} was not told what was chosen: {error}");
        }
    }

    /// The desktop looks different now: applications are told what changed.
    pub fn set(&self, new: Look) {
        let old = std::mem::replace(
            &mut *self
                .look
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            new,
        );
        if old == new {
            return;
        }
        let before = old.settings();
        for ((key, value), (_, was)) in new.settings().into_iter().zip(before) {
            if value == was {
                continue;
            }
            let sent = self.connection.emit_signal(
                Option::<&str>::None,
                PATH,
                "org.freedesktop.impl.portal.Settings",
                "SettingChanged",
                &(APPEARANCE, key, value),
            );
            if let Err(error) = sent {
                log::warn!("applications were not told of the new {key}: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_told_as_an_address_that_reads_back_as_the_file() {
        let file = std::path::Path::new(
            "/home/ada/.cache/gevrek/portal/Screenshot 2026-11-16 02-10-00 (2).png",
        );
        let uri = uri_of(file);
        assert_eq!(
            uri,
            "file:///home/ada/.cache/gevrek/portal/Screenshot%202026-11-16%2002-10-00%20%282%29.png"
        );
        assert_eq!(file_of(&uri).as_deref(), Some(file));
        // Letters that are not ASCII are written as their bytes.
        let turkish = std::path::Path::new("/home/ulaş/x.png");
        assert_eq!(uri_of(turkish), "file:///home/ula%C5%9F/x.png");
        assert_eq!(file_of(&uri_of(turkish)).as_deref(), Some(turkish));
    }

    #[test]
    fn an_address_of_a_file_is_read_as_its_path() {
        assert_eq!(
            file_of("file:///home/ada/Pictures/a%20b.png"),
            Some("/home/ada/Pictures/a b.png".into())
        );
        assert_eq!(
            file_of("file://localhost/tmp/x.jpg"),
            Some("/tmp/x.jpg".into())
        );
        assert_eq!(file_of("file:///tmp/%C3%A7.png"), Some("/tmp/ç.png".into()));
        // Another kind of address, another machine, a broken one.
        assert_eq!(file_of("https://example.org/a.png"), None);
        assert_eq!(file_of("file://elsewhere/tmp/x.jpg"), None);
        assert_eq!(file_of("file:///tmp/%zz.png"), None);
        assert_eq!(file_of("file:///tmp/%4"), None);
        assert_eq!(file_of("/tmp/x.png"), None);
    }

    #[test]
    fn what_may_be_shared_is_read_from_what_the_application_asks() {
        // Nothing said: displays, and the pointer not in the pictures.
        assert_eq!(wanted(None, None), Wanted::default());
        assert_eq!(wanted(Some(0), None), Wanted::default());
        let windows = wanted(Some(WINDOWS), Some(POINTER_DRAWN));
        assert!(!windows.displays && windows.windows && windows.pointer);
        let both = wanted(Some(DISPLAYS | WINDOWS), Some(POINTER_HIDDEN));
        assert!(both.displays && both.windows && !both.pointer);
        // The pointer beside the pictures: not drawn into them.
        let beside = wanted(Some(DISPLAYS), Some(4));
        assert!(beside.beside && !beside.pointer);
        assert!(!windows.beside && !both.beside);
    }

    #[test]
    fn a_stream_is_told_in_the_portals_words() {
        let one = Shared {
            node: 57,
            size: (1920, 1080),
            display: true,
            output: Some("DP-1".to_string()),
        };
        let other = Shared {
            node: 58,
            ..one.clone()
        };
        let told = streams_of(&[one, other]).expect("a value");
        assert_eq!(told.value_signature().as_str(), "a(ua{sv})");
    }

    #[test]
    fn the_applications_that_have_windows_are_told_with_how_they_stand() {
        let state = serde_json::json!({"monitors": [{"workspaces": [
            {"detailed_windows": [
                {"app_id": "org.editor", "class": "editor", "is_focused": false},
                {"app_id": "org.editor", "class": "editor", "is_focused": true},
                {"app_id": "", "class": "xterm"},
                {"app_id": "", "class": "", "is_border_window": true},
            ]},
            {"detailed_windows": [{"app_id": "org.mail"}, {"app_id": "", "class": ""}]},
        ]}]});
        let apps = apps_of(&state);
        assert_eq!(apps.len(), 3);
        assert_eq!(apps["org.editor"], 2, "one of its windows has the keyboard");
        assert_eq!((apps["xterm"], apps["org.mail"]), (1, 1));
        assert!(apps_of(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn an_autostart_entry_says_how_the_application_is_started() {
        let line = [
            "flatpak",
            "run",
            "--command=mail",
            "org.mail",
            "--name=My Mail",
            "a\"b",
        ]
        .map(str::to_string);
        let entry = autostart_entry("org.mail", &line, false);
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=org.mail\n"));
        assert!(
            entry.contains(
                "Exec=flatpak run --command=mail org.mail \"--name=My Mail\" \"a\\\"b\"\n"
            ),
            "{entry}"
        );
        assert!(!entry.contains("DBusActivatable"));
        assert!(autostart_entry("org.mail", &line, true).ends_with("DBusActivatable=true\n"));
    }

    #[test]
    fn an_application_controls_what_it_asks_for_of_what_there_is() {
        assert_eq!(devices_of(None), KEYBOARD | POINTER);
        assert_eq!(devices_of(Some(0)), KEYBOARD | POINTER);
        assert_eq!(devices_of(Some(KEYBOARD)), KEYBOARD);
        assert_eq!(devices_of(Some(POINTER)), POINTER);
        // A touch screen (4) cannot be made up: of all three, the two.
        assert_eq!(devices_of(Some(7)), KEYBOARD | POINTER);
        assert_eq!(devices_of(Some(4)), KEYBOARD | POINTER);
        // Nothing is granted, and nothing chosen, before the user says.
        let fresh = Wanted::default();
        assert_eq!(
            (fresh.granted, fresh.devices, fresh.selected),
            (0, 0, false)
        );
    }

    #[test]
    fn a_remembered_choice_is_read_back_as_it_was_kept() {
        let displays = vec!["DP-1".to_string(), "HDMI-A-1".to_string()];
        let kept = to_remember(&displays).expect("a value");
        assert_eq!(kept.value_signature().as_str(), "(suv)");
        assert_eq!(remembered(Some(&kept)), displays);
        // Another desktop's, or nothing: the user is asked.
        let other = Value::from(("GNOME", 1u32, Value::from(vec!["DP-1".to_string()])))
            .try_to_owned()
            .expect("a value");
        assert!(remembered(Some(&other)).is_empty());
        assert!(remembered(None).is_empty());
    }

    #[test]
    fn a_priority_of_the_portal_is_an_urgency() {
        use crate::notifications::Urgency;
        assert_eq!(urgency_of(Some("low")), Urgency::Low);
        assert_eq!(urgency_of(Some("normal")), Urgency::Normal);
        assert_eq!(urgency_of(Some("high")), Urgency::Normal);
        assert_eq!(urgency_of(Some("urgent")), Urgency::Critical);
        assert_eq!(urgency_of(None), Urgency::Normal);
    }

    #[test]
    fn a_request_names_the_settings_it_wants_by_their_start() {
        assert!(matches(&[], APPEARANCE));
        assert!(matches(
            &["org.freedesktop.appearance".to_string()],
            APPEARANCE
        ));
        assert!(matches(&["org.freedesktop.*".to_string()], APPEARANCE));
        assert!(matches(&[String::new()], APPEARANCE));
        assert!(!matches(
            &["org.gnome.desktop.interface".to_string()],
            APPEARANCE
        ));
    }

    #[test]
    fn the_look_is_told_in_the_portals_words() {
        let dark = Look {
            reduce_motion: false,
            contrast: false,
            dark: true,
            accent: (0.25, 0.875, 0.8125),
        };
        let settings = dark.settings();
        let value = |key: &str| {
            settings
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value)
                .expect(key)
        };
        assert_eq!(u32::try_from(value("color-scheme")).expect("a number"), 1);
        assert_eq!(u32::try_from(value("reduced-motion")).expect("a number"), 0);
        let light = Look {
            dark: false,
            reduce_motion: true,
            ..dark
        };
        let still = light
            .settings()
            .into_iter()
            .find(|(name, _)| *name == "reduced-motion")
            .expect("reduced-motion")
            .1;
        assert_eq!(u32::try_from(still).expect("a number"), 1);
        assert_eq!(u32::try_from(value("contrast")).expect("a number"), 0);
        let more = Look {
            contrast: true,
            ..dark
        };
        let wanted = more
            .settings()
            .into_iter()
            .find(|(name, _)| *name == "contrast")
            .expect("contrast")
            .1;
        assert_eq!(u32::try_from(wanted).expect("a number"), 1);
        let scheme = light
            .settings()
            .into_iter()
            .find(|(name, _)| *name == "color-scheme")
            .expect("the scheme")
            .1;
        assert_eq!(u32::try_from(scheme).expect("a number"), 2);
        // Three numbers, as the portal wants them: `(ddd)`.
        let accent = value("accent-color");
        assert_eq!(accent.value_signature().to_string(), "(ddd)");
        assert!(format!("{accent:?}").contains("0.875"));
    }
}
