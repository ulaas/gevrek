//! The tray: the icons programs put on the panel while they run in the
//! background, each with what a press on it does and a menu.
//!
//! The shell is the watcher programs register their icon with
//! (`org.kde.StatusNotifierWatcher`) and the host that shows them, and a
//! menu is asked of the program as `com.canonical.dbusmenu`: its lines
//! with their pictures, marks and keys, and its submenus, each opened in
//! the menu's place with a line that leads back. The wheel over an icon
//! and the middle button are told to its program. An icon whose program
//! calls it passive stands in the list of all icons only; one that wants
//! attention is marked. Icons of the older kind (windows embedded in the
//! panel, X11's way) are shown where a program that carries them over
//! runs (`xembedsniproxy`, see `older` below).
//!
//! Everything said over the bus is said on the tray's own thread: a
//! program that does not answer holds that thread, never the drawing.

use std::sync::Arc;

/// An icon on the panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// What tells this icon from every other: the program's name on the
    /// bus and the place of its icon there.
    pub key: String,
    /// Its lasting name: the same when its program is started again
    /// (which `key` is not). What the person's choice of the icons that
    /// stand on the panel is kept by.
    pub name: String,
    /// What the program calls it.
    pub title: String,
    /// What is said when the pointer rests on it.
    pub hint: String,
    /// Its picture, when it gave one that could be read.
    pub picture: Option<Arc<gevrek_system::icons::Picture>>,
    /// Another number when the picture is another one.
    pub stamp: u64,
    /// A press opens the menu (the program does nothing else on a press).
    pub menu_only: bool,
    /// Its program has nothing to say now: it is kept off the panel and
    /// stands in the list of all icons.
    pub passive: bool,
    /// Its program wants to be looked at.
    pub attention: bool,
}

impl Item {
    /// What stands for the icon where it has no picture.
    pub fn letter(&self) -> String {
        self.title
            .chars()
            .chain(self.hint.chars())
            .find(|letter| letter.is_alphanumeric())
            .map_or_else(|| "?".to_string(), |letter| letter.to_uppercase().collect())
    }
}

/// How a line of a menu is marked when it can be switched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Check,
    Radio,
}

/// One line of an icon's menu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Entry {
    /// The program's number for it: what a press sends back.
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    /// A line between two groups, not something to press.
    pub separator: bool,
    /// Switchable, and whether it is on.
    pub toggle: Option<(Toggle, bool)>,
    /// A press opens a submenu.
    pub submenu: bool,
    /// Its picture, when the program gave one that could be read.
    pub picture: Option<Arc<gevrek_system::icons::Picture>>,
    /// The keys that do the same in the program, as it writes them
    /// (`Ctrl+S`); empty when it names none.
    pub keys: String,
}

/// A menu as the program describes it: a line and the lines under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    pub toggle: Option<(Toggle, bool)>,
    pub picture: Option<Arc<gevrek_system::icons::Picture>>,
    pub keys: String,
    /// It says it has lines under it, whether or not they were sent yet
    /// (some programs fill a submenu only when it is opened).
    pub submenu: bool,
    pub children: Vec<Node>,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            id: 0,
            label: String::new(),
            // What a program does not say of a line: it is there and
            // can be pressed.
            enabled: true,
            visible: true,
            separator: false,
            toggle: None,
            picture: None,
            keys: String::new(),
            submenu: false,
            children: Vec::new(),
        }
    }
}

impl Node {
    /// The line with this number, wherever under this one it is.
    pub fn find(&self, id: i32) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    /// The menu that is shown after going down `path` (the numbers of
    /// the submenus opened, one after another): the deepest of them that
    /// is still there, with how many steps of the path held.
    pub fn at(&self, path: &[i32]) -> (&Node, usize) {
        let mut here = self;
        let mut held = 0;
        for id in path {
            match here.children.iter().find(|child| child.id == *id) {
                Some(child) => here = child,
                None => break,
            }
            held += 1;
        }
        (here, held)
    }
}

/// The keys a program names for a line of its menu, written as one
/// reads them: each way of pressing joined by `+`, several ways by a
/// comma.
pub fn keys_of(ways: &[Vec<String>]) -> String {
    let written: Vec<String> = ways
        .iter()
        .filter(|way| !way.is_empty())
        .map(|way| {
            way.iter()
                .map(|key| match key.as_str() {
                    "Control" => "Ctrl",
                    other => other,
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect();
    written.join(", ")
}

/// What a program registers its icon with: the name on the bus and the
/// icon's place there. The argument is a name (the icon is at the usual
/// place) or a place (the name is the caller's).
pub fn registered(argument: &str, sender: &str) -> Option<(String, String)> {
    let argument = argument.trim();
    if argument.starts_with('/') {
        return (!sender.is_empty()).then(|| (sender.to_string(), argument.to_string()));
    }
    let name = if argument.is_empty() {
        sender
    } else {
        argument
    };
    (!name.is_empty()).then(|| (name.to_string(), "/StatusNotifierItem".to_string()))
}

/// Pixels as an icon carries them over the bus (how solid, red, green,
/// blue) as the pixels of a picture (red, green, blue, how solid).
/// `None` when there are not as many as the size says.
pub fn pixels(width: i32, height: i32, sent: &[u8]) -> Option<Vec<u8>> {
    let count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    if count == 0 || sent.len() != count.checked_mul(4)? {
        return None;
    }
    Some(
        sent.as_chunks::<4>()
            .0
            .iter()
            .flat_map(|[solid, red, green, blue]| [*red, *green, *blue, *solid])
            .collect(),
    )
}

/// Which of the sizes an icon comes in to take for `wanted` pixels: the
/// smallest that is large enough, else the largest there is.
pub fn nearest(sizes: &[(i32, i32)], wanted: i32) -> Option<usize> {
    let side = |index: &usize| sizes[*index].0.min(sizes[*index].1);
    let usable = (0..sizes.len()).filter(|index| side(index) > 0);
    usable
        .clone()
        .filter(|index| side(index) >= wanted)
        .min_by_key(side)
        .or_else(|| usable.max_by_key(side))
}

/// A menu's label without the mark for its key (`_Open` reads `Open`; two
/// underscores are one that is meant).
pub fn plain(label: &str) -> String {
    let mut said = String::with_capacity(label.len());
    let mut letters = label.chars().peekable();
    while let Some(letter) = letters.next() {
        if letter == '_' {
            if letters.peek() == Some(&'_') {
                letters.next();
                said.push('_');
            }
            continue;
        }
        said.push(letter);
    }
    said
}

/// The lines of one menu in the order they are shown: what is right
/// under `menu`. Lines the program hides are left out, and so is a
/// separator with nothing to separate.
pub fn lines(menu: &Node) -> Vec<Entry> {
    let mut lines: Vec<Entry> = Vec::new();
    for child in menu.children.iter().filter(|child| child.visible) {
        if child.separator {
            // Not first, and not after another.
            if lines.last().is_some_and(|last| !last.separator) {
                lines.push(Entry {
                    id: child.id,
                    separator: true,
                    ..Default::default()
                });
            }
            continue;
        }
        lines.push(Entry {
            id: child.id,
            label: plain(&child.label),
            enabled: child.enabled,
            separator: false,
            toggle: child.toggle,
            submenu: child.submenu || !child.children.is_empty(),
            picture: child.picture.clone(),
            keys: child.keys.clone(),
        });
    }
    // Nor last.
    while lines.last().is_some_and(|last| last.separator) {
        lines.pop();
    }
    lines
}

/// What the tray tells the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum FromTray {
    /// The icons there are now, in the order they came.
    Items(Vec<Item>),
    /// The menu that was asked for, whole: its lines and theirs. Sent
    /// again when a submenu was opened (its program may fill it then).
    Menu { key: String, menu: Node },
}

/// What the shell asks of an icon's program.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// The icon was pressed.
    Activate { key: String },
    /// Its menu is wanted: it comes back as [`FromTray::Menu`].
    Menu { key: String },
    /// A line of its menu was pressed.
    Press { key: String, id: i32 },
    /// A submenu of its menu is being opened: the program is told, and
    /// the menu comes back once more as it is then.
    Open { key: String, id: i32 },
    /// The wheel was turned over the icon: by so many notches (away
    /// from the user is more than nothing), up and down or sideways.
    Scroll {
        key: String,
        notches: i32,
        sideways: bool,
    },
    /// The middle button was pressed on the icon.
    Middle { key: String },
}

/// The program that carries icons of the older kind over to the tray,
/// where one is installed: X11 programs embed a little window in the
/// panel (XEmbed), which a Wayland panel has no place for; this stands
/// in between and registers each as an icon of the kind the tray shows.
pub const OLDER: &str = "xembedsniproxy";

/// Whether that program is to run: wanted by the configuration, a tray
/// to register with, a display of X11's to look for such icons on, and
/// the program itself.
pub fn older_wanted(setting: bool, tray: bool, x11_display: Option<&str>, found: bool) -> bool {
    setting && tray && x11_display.is_some_and(|display| !display.is_empty()) && found
}

/// The program that carries the older icons over, while it runs. Dropped,
/// it is ended.
#[cfg(target_os = "linux")]
pub struct Older(std::process::Child);

#[cfg(target_os = "linux")]
impl Older {
    /// Whether the program is installed.
    pub fn found() -> bool {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|folder| folder.join(OLDER).is_file())
        })
    }

    pub fn start() -> Option<Self> {
        std::process::Command::new(OLDER)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| log::warn!("the older icons are not shown: {OLDER}: {error}"))
            .ok()
            .map(Self)
    }

    /// It runs still (it ends by itself where there is no X11 display).
    pub fn runs(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

#[cfg(target_os = "linux")]
impl Drop for Older {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// What the wheel is told to a program as: a notch is 120, as the wheels
/// of mice count.
pub fn wheel_units(notches: i32) -> i32 {
    notches.saturating_mul(120)
}

#[cfg(target_os = "linux")]
pub use bus::{serve, Tray};

#[cfg(target_os = "linux")]
mod bus {
    use super::*;
    use std::collections::HashMap;
    use std::sync::mpsc::{channel, Sender};
    use std::sync::Mutex;
    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedValue, Value};

    const WATCHER: &str = "org.kde.StatusNotifierWatcher";
    const WATCHER_PATH: &str = "/StatusNotifierWatcher";
    const ITEM: &str = "org.kde.StatusNotifierItem";
    const MENU: &str = "com.canonical.dbusmenu";
    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
    /// The side, in pixels, an icon's picture is asked for at: enough
    /// for a panel on a display scaled to twice its size.
    const SIDE: i32 = 44;

    /// What the tray's thread is given to do.
    enum Work {
        /// A program registered an icon.
        Register {
            service: String,
            path: String,
            owner: String,
        },
        /// An icon's program says something of it changed.
        Changed {
            owner: String,
            path: String,
        },
        /// A name left the bus.
        Gone(String),
        Ask(Ask),
    }

    /// An icon the tray holds, and where its program is.
    struct Known {
        service: String,
        path: String,
        /// The connection the icon came from: what goes when it goes.
        owner: String,
        /// Where its menu is, when it has one.
        menu: String,
        item: Item,
    }

    /// The watcher: what programs register their icon with.
    struct Watcher {
        work: Mutex<Sender<Work>>,
        /// The icons that are registered, as other hosts are told them.
        items: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(
            &self,
            service: &str,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) {
            let sender = header
                .sender()
                .map(|name| name.to_string())
                .unwrap_or_default();
            let Some((service, path)) = registered(service, &sender) else {
                log::debug!("tray: {sender} registered {service:?}, which is no icon's name");
                return;
            };
            if let Ok(work) = self.work.lock() {
                let _ = work.send(Work::Register {
                    service,
                    path,
                    owner: sender,
                });
            }
        }

        /// The shell is the host; another one may say it is there too.
        fn register_status_notifier_host(&self, _service: &str) {}

        #[zbus(property)]
        fn registered_status_notifier_items(&self) -> Vec<String> {
            self.items
                .lock()
                .map(|items| items.clone())
                .unwrap_or_default()
        }

        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn protocol_version(&self) -> i32 {
            0
        }
    }

    /// The shell's end of the tray.
    pub struct Tray {
        work: Sender<Work>,
    }

    impl Tray {
        pub fn ask(&self, ask: Ask) {
            let _ = self.work.send(Work::Ask(ask));
        }
    }

    /// Be the tray on the session's bus, on threads of its own. `tell` is
    /// called with what there is to show. An error when the bus is not
    /// there or another tray has the watcher's name.
    pub fn serve(tell: impl Fn(FromTray) + Send + 'static) -> anyhow::Result<Tray> {
        let (work, todo) = channel::<Work>();
        let listed = Arc::new(Mutex::new(Vec::new()));
        let connection = zbus::blocking::connection::Builder::session()?
            .name(WATCHER)?
            .serve_at(
                WATCHER_PATH,
                Watcher {
                    work: Mutex::new(work.clone()),
                    items: Arc::clone(&listed),
                },
            )?
            .build()
            .map_err(|error| {
                anyhow::anyhow!("the tray could not take its name on the bus: {error}")
            })?;
        // Programs that looked for a host before now find one.
        if let Err(error) = connection.emit_signal(
            Option::<&str>::None,
            WATCHER_PATH,
            WATCHER,
            "StatusNotifierHostRegistered",
            &(),
        ) {
            log::debug!("tray: programs were not told that there is a host: {error}");
        }

        // What icons say of themselves, and who leaves the bus.
        let changes = zbus::blocking::MessageIterator::for_match_rule(
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface(ITEM)?
                .build(),
            &connection,
            Some(64),
        )?;
        let news = work.clone();
        std::thread::Builder::new()
            .name("gevrek-tray-changes".to_string())
            .spawn(move || {
                for message in changes.flatten() {
                    let header = message.header();
                    let (Some(owner), Some(path)) = (header.sender(), header.path()) else {
                        continue;
                    };
                    let sent = news.send(Work::Changed {
                        owner: owner.to_string(),
                        path: path.to_string(),
                    });
                    if sent.is_err() {
                        break;
                    }
                }
            })?;
        let leaving = zbus::blocking::MessageIterator::for_match_rule(
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.freedesktop.DBus")?
                .interface("org.freedesktop.DBus")?
                .member("NameOwnerChanged")?
                .build(),
            &connection,
            Some(256),
        )?;
        let news = work.clone();
        std::thread::Builder::new()
            .name("gevrek-tray-names".to_string())
            .spawn(move || {
                for message in leaving.flatten() {
                    let Ok((name, _old, new)) =
                        message.body().deserialize::<(String, String, String)>()
                    else {
                        continue;
                    };
                    if new.is_empty() && news.send(Work::Gone(name)).is_err() {
                        break;
                    }
                }
            })?;

        // Icons that were there before the tray was: those that go by
        // the usual name are found without being registered again.
        for (service, owner) in already_there(&connection) {
            let _ = work.send(Work::Register {
                service,
                path: "/StatusNotifierItem".to_string(),
                owner,
            });
        }

        std::thread::Builder::new()
            .name("gevrek-tray".to_string())
            .spawn(move || {
                let mut known: Vec<Known> = Vec::new();
                let mut stamp = 0u64;
                for work in todo {
                    let before: Vec<Item> = known.iter().map(|icon| icon.item.clone()).collect();
                    match work {
                        Work::Register {
                            service,
                            path,
                            owner,
                        } => {
                            let key = format!("{service}{path}");
                            stamp += 1;
                            let Some(icon) =
                                read(&connection, &service, &path, &owner, &key, stamp)
                            else {
                                continue;
                            };
                            match known.iter_mut().find(|icon| icon.item.key == key) {
                                Some(place) => *place = icon,
                                None => {
                                    log::debug!("tray: {key} is added");
                                    known.push(icon);
                                    let _ = connection.emit_signal(
                                        Option::<&str>::None,
                                        WATCHER_PATH,
                                        WATCHER,
                                        "StatusNotifierItemRegistered",
                                        &(key.as_str(),),
                                    );
                                }
                            }
                        }
                        Work::Changed { owner, path } => {
                            for place in known
                                .iter_mut()
                                .filter(|icon| icon.owner == owner && icon.path == path)
                            {
                                stamp += 1;
                                if let Some(icon) = read(
                                    &connection,
                                    &place.service,
                                    &place.path,
                                    &owner,
                                    &place.item.key,
                                    stamp,
                                ) {
                                    // The same picture keeps its number:
                                    // nothing is made anew for it.
                                    let same = icon.item.picture == place.item.picture;
                                    let old = place.item.stamp;
                                    *place = icon;
                                    if same {
                                        place.item.stamp = old;
                                    }
                                }
                            }
                        }
                        Work::Gone(name) => {
                            let (gone, kept): (Vec<Known>, Vec<Known>) = known
                                .into_iter()
                                .partition(|icon| icon.owner == name || icon.service == name);
                            known = kept;
                            for icon in gone {
                                log::debug!(
                                    "tray: {} is removed: its program left the bus",
                                    icon.item.key
                                );
                                let _ = connection.emit_signal(
                                    Option::<&str>::None,
                                    WATCHER_PATH,
                                    WATCHER,
                                    "StatusNotifierItemUnregistered",
                                    &(icon.item.key.as_str(),),
                                );
                            }
                        }
                        Work::Ask(ask) => {
                            if let Some(menu) = answer(&connection, &known, ask) {
                                tell(menu);
                            }
                        }
                    }
                    let now: Vec<Item> = known.iter().map(|icon| icon.item.clone()).collect();
                    if now != before {
                        if let Ok(mut listed) = listed.lock() {
                            *listed = now.iter().map(|item| item.key.clone()).collect();
                        }
                        tell(FromTray::Items(now));
                    }
                }
            })?;
        Ok(Tray { work })
    }

    /// The icons on the bus under the name most programs give theirs,
    /// each with the connection it belongs to.
    fn already_there(connection: &Connection) -> Vec<(String, String)> {
        const BUS: &str = "org.freedesktop.DBus";
        let names: Vec<String> = connection
            .call_method(
                Some(BUS),
                "/org/freedesktop/DBus",
                Some(BUS),
                "ListNames",
                &(),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize().ok())
            .unwrap_or_default();
        names
            .into_iter()
            .filter(|name| name.starts_with("org.kde.StatusNotifierItem-"))
            .filter_map(|name| {
                let owner: String = connection
                    .call_method(
                        Some(BUS),
                        "/org/freedesktop/DBus",
                        Some(BUS),
                        "GetNameOwner",
                        &(name.as_str(),),
                    )
                    .ok()?
                    .body()
                    .deserialize()
                    .ok()?;
                Some((name, owner))
            })
            .collect()
    }

    /// Ask a program for its icon. `None` when it does not answer as one.
    fn read(
        connection: &Connection,
        service: &str,
        path: &str,
        owner: &str,
        key: &str,
        stamp: u64,
    ) -> Option<Known> {
        let mut said: HashMap<String, OwnedValue> = connection
            .call_method(Some(service), path, Some(PROPERTIES), "GetAll", &(ITEM,))
            .map_err(|error| log::debug!("tray: {service}{path} is not an icon: {error}"))
            .ok()?
            .body()
            .deserialize()
            .ok()?;
        let mut words = |name: &str| {
            said.remove(name)
                .and_then(|value| String::try_from(value).ok())
                .unwrap_or_default()
        };
        let title = words("Title");
        let id = words("Id");
        let status = words("Status");
        let (passive, attention) = (status == "Passive", status == "NeedsAttention");
        let usual = words("IconName");
        let wanted = words("AttentionIconName");
        // What wants attention shows the picture it has for that.
        let icon_name = if attention && !wanted.is_empty() {
            wanted
        } else {
            usual
        };
        let theme_path = words("IconThemePath");
        let menu_only = said
            .get("ItemIsMenu")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        // The menu's place is a place, or (from some) written as words.
        let menu = said
            .remove("Menu")
            .and_then(|value| {
                zbus::zvariant::OwnedObjectPath::try_from(value.try_clone().ok()?)
                    .map(|path| path.to_string())
                    .ok()
                    .or_else(|| String::try_from(value).ok())
            })
            .filter(|path| path.starts_with('/') && path != "/NO_DBUSMENU")
            .unwrap_or_default();
        type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
        let tip = said
            .remove("ToolTip")
            .and_then(|value| <(String, Pixmaps, String, String)>::try_from(value).ok())
            .map(|(_, _, title, _)| title)
            .unwrap_or_default();
        let pixmaps = said
            .remove("IconPixmap")
            .and_then(|value| Pixmaps::try_from(value).ok())
            .unwrap_or_default();
        // The picture it names, as the launcher finds an application's;
        // else the one it sends.
        let named = (!icon_name.is_empty())
            .then(|| {
                let side = SIDE as u32;
                let own = [std::path::PathBuf::from(&theme_path)];
                let from_own = (!theme_path.is_empty())
                    .then(|| {
                        ["png", "svg"]
                            .iter()
                            .map(|ending| own[0].join(format!("{icon_name}.{ending}")))
                            .find(|file| file.is_file())
                            .or_else(|| {
                                gevrek_system::icons::find_in(&own, &[], None, &icon_name, side)
                            })
                    })
                    .flatten()
                    .and_then(|file| gevrek_system::icons::load(&file, side));
                from_own.or_else(|| gevrek_system::icons::picture(&icon_name, side))
            })
            .flatten();
        let sent = || {
            let sizes: Vec<(i32, i32)> = pixmaps
                .iter()
                .map(|(width, height, _)| (*width, *height))
                .collect();
            let (width, height, data) = &pixmaps[nearest(&sizes, SIDE)?];
            Some(gevrek_system::icons::Picture {
                width: *width as u32,
                height: *height as u32,
                pixels: pixels(*width, *height, data)?,
            })
        };
        // Neither found: the program's own picture, by what it calls
        // itself (better than a letter for a sign no theme here has).
        let own = || {
            let side = SIDE as u32;
            [id.as_str(), id.to_lowercase().as_str()]
                .into_iter()
                .filter(|name| !name.is_empty())
                .find_map(|name| gevrek_system::icons::picture(name, side))
        };
        let picture = named.or_else(sent).or_else(own).map(Arc::new);
        // (What the program calls its icon among programs, `Id`, is the
        // same at every start; one without it is known by its title.)
        let name = if id.is_empty() {
            title.clone()
        } else {
            id.clone()
        };
        let title = if title.is_empty() { id } else { title };
        let hint = if tip.is_empty() { title.clone() } else { tip };
        Some(Known {
            service: service.to_string(),
            path: path.to_string(),
            owner: owner.to_string(),
            menu,
            item: Item {
                key: key.to_string(),
                name,
                title,
                hint,
                picture,
                stamp,
                menu_only,
                passive,
                attention,
            },
        })
    }

    /// Carry out what the shell asks of an icon; a menu when one was
    /// asked for and the program has one.
    fn answer(connection: &Connection, known: &[Known], ask: Ask) -> Option<FromTray> {
        let of = |key: &str| known.iter().find(|icon| icon.item.key == key);
        // Where on the screen the press was is not known to a panel: the
        // program is told the corner.
        let place = (0i32, 0i32);
        match ask {
            Ask::Activate { key } => {
                let icon = of(&key)?;
                if let Err(error) = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.path.as_str(),
                    Some(ITEM),
                    "Activate",
                    &place,
                ) {
                    log::debug!("tray: {key} did not take the press: {error}");
                }
                None
            }
            Ask::Menu { key } => {
                let icon = of(&key)?;
                if icon.menu.is_empty() {
                    // No menu to show here: the program may show its own.
                    let _ = connection.call_method(
                        Some(icon.service.as_str()),
                        icon.path.as_str(),
                        Some(ITEM),
                        "ContextMenu",
                        &place,
                    );
                    return None;
                }
                // Some programs fill the menu only when told it is about
                // to be shown.
                let _ = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.menu.as_str(),
                    Some(MENU),
                    "AboutToShow",
                    &(0i32,),
                );
                let reply = connection
                    .call_method(
                        Some(icon.service.as_str()),
                        icon.menu.as_str(),
                        Some(MENU),
                        "GetLayout",
                        &(0i32, -1i32, Vec::<&str>::new()),
                    )
                    .map_err(|error| log::warn!("tray: {key} gave no menu: {error}"))
                    .ok()?;
                let (_, (id, said, children)): (u32, Layout) = reply
                    .body()
                    .deserialize()
                    .map_err(|error| log::warn!("tray: the menu of {key} is not one: {error}"))
                    .ok()?;
                let menu = node(id, &said, children.iter().map(|child| &**child));
                Some(FromTray::Menu { key, menu })
            }
            Ask::Open { key, id } => {
                let icon = of(&key)?;
                if icon.menu.is_empty() {
                    return None;
                }
                // The program is told that the submenu opens (some fill
                // it only then), and the menu is read once more.
                let _ = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.menu.as_str(),
                    Some(MENU),
                    "AboutToShow",
                    &(id,),
                );
                let _ = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.menu.as_str(),
                    Some(MENU),
                    "Event",
                    &(id, "opened", Value::I32(0), 0u32),
                );
                let reply = connection
                    .call_method(
                        Some(icon.service.as_str()),
                        icon.menu.as_str(),
                        Some(MENU),
                        "GetLayout",
                        &(0i32, -1i32, Vec::<&str>::new()),
                    )
                    .ok()?;
                let (_, (id, said, children)): (u32, Layout) = reply.body().deserialize().ok()?;
                let menu = node(id, &said, children.iter().map(|child| &**child));
                Some(FromTray::Menu { key, menu })
            }
            Ask::Scroll {
                key,
                notches,
                sideways,
            } => {
                let icon = of(&key)?;
                let way = if sideways { "horizontal" } else { "vertical" };
                if let Err(error) = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.path.as_str(),
                    Some(ITEM),
                    "Scroll",
                    &(wheel_units(notches), way),
                ) {
                    log::debug!("tray: {key} did not take the wheel: {error}");
                }
                None
            }
            Ask::Middle { key } => {
                let icon = of(&key)?;
                if let Err(error) = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.path.as_str(),
                    Some(ITEM),
                    "SecondaryActivate",
                    &place,
                ) {
                    log::debug!("tray: {key} did not take the middle button: {error}");
                }
                None
            }
            Ask::Press { key, id } => {
                let icon = of(&key)?;
                if let Err(error) = connection.call_method(
                    Some(icon.service.as_str()),
                    icon.menu.as_str(),
                    Some(MENU),
                    "Event",
                    &(id, "clicked", Value::I32(0), 0u32),
                ) {
                    log::warn!("tray: {key} did not take the choice: {error}");
                }
                None
            }
        }
    }

    /// A line of a menu as it comes over the bus: its number, what is
    /// said of it, the lines under it.
    type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

    fn node<'a>(
        id: i32,
        said: &HashMap<String, OwnedValue>,
        children: impl Iterator<Item = &'a Value<'a>>,
    ) -> Node {
        let words = |name: &str| {
            said.get(name)
                .and_then(|value| <&str>::try_from(value).ok())
                .unwrap_or_default()
        };
        let flag = |name: &str| {
            said.get(name)
                .and_then(|value| bool::try_from(value).ok())
                .unwrap_or(true)
        };
        let on = said
            .get("toggle-state")
            .and_then(|value| i32::try_from(value).ok())
            == Some(1);
        Node {
            id,
            label: words("label").to_string(),
            enabled: flag("enabled"),
            visible: flag("visible"),
            separator: words("type") == "separator",
            toggle: match words("toggle-type") {
                "checkmark" => Some((Toggle::Check, on)),
                "radio" => Some((Toggle::Radio, on)),
                _ => None,
            },
            picture: picture_of(said, words("icon-name")).map(Arc::new),
            keys: said
                .get("shortcut")
                .and_then(|value| value.try_clone().ok())
                .and_then(|value| Vec::<Vec<String>>::try_from(value).ok())
                .map(|ways| keys_of(&ways))
                .unwrap_or_default(),
            submenu: words("children-display") == "submenu",
            children: children.filter_map(child).collect(),
        }
    }

    /// The side a menu line's picture is read at, in pixels.
    const LINE_PICTURE: u32 = 32;

    /// The picture of a menu's line: the one it sends (a PNG file's
    /// bytes), else the one it names, as the launcher finds such.
    fn picture_of(
        said: &HashMap<String, OwnedValue>,
        named: &str,
    ) -> Option<gevrek_system::icons::Picture> {
        let sent = said
            .get("icon-data")
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| Vec::<u8>::try_from(value).ok())
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|picture| {
                let picture = picture.to_rgba8();
                gevrek_system::icons::Picture {
                    width: picture.width(),
                    height: picture.height(),
                    pixels: picture.into_raw(),
                }
            });
        sent.or_else(|| {
            (!named.is_empty())
                .then(|| gevrek_system::icons::picture(named, LINE_PICTURE))
                .flatten()
        })
    }

    /// A line under another: sent as a value of any kind, to be read as
    /// a line again.
    fn child(value: &Value<'_>) -> Option<Node> {
        // (Wrapped once, or by some programs more than once.)
        let mut value = value;
        while let Value::Value(inner) = value {
            value = inner;
        }
        let Value::Structure(line) = value else {
            return None;
        };
        let [Value::I32(id), Value::Dict(said), Value::Array(children)] = line.fields() else {
            return None;
        };
        let said: HashMap<String, OwnedValue> = said.try_clone().ok()?.try_into().ok()?;
        Some(node(*id, &said, children.iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_is_registered_by_name_or_by_place() {
        assert_eq!(
            registered("org.kde.StatusNotifierItem-12-1", ":1.7"),
            Some((
                "org.kde.StatusNotifierItem-12-1".into(),
                "/StatusNotifierItem".into()
            ))
        );
        // A place: the name is the caller's.
        assert_eq!(
            registered("/org/ayatana/NotificationItem/app", ":1.7"),
            Some((":1.7".into(), "/org/ayatana/NotificationItem/app".into()))
        );
        assert_eq!(
            registered("", ":1.7"),
            Some((":1.7".into(), "/StatusNotifierItem".into()))
        );
        assert_eq!(registered("", ""), None);
        assert_eq!(registered("/somewhere", ""), None);
    }

    #[test]
    fn pixels_from_the_bus_have_their_solidness_last() {
        // Two pixels: a solid red, a half-solid blue.
        let sent = [255, 200, 10, 20, 128, 1, 2, 250];
        assert_eq!(
            pixels(2, 1, &sent),
            Some(vec![200, 10, 20, 255, 1, 2, 250, 128])
        );
        // Not as many as the size says, or no size.
        assert_eq!(pixels(2, 2, &sent), None);
        assert_eq!(pixels(0, 0, &[]), None);
        assert_eq!(pixels(-1, 1, &sent), None);
    }

    #[test]
    fn the_size_taken_is_the_smallest_that_is_enough() {
        let sizes = [(16, 16), (64, 64), (48, 48), (0, 0)];
        assert_eq!(nearest(&sizes, 44), Some(2));
        assert_eq!(nearest(&sizes, 64), Some(1));
        // None is enough: the largest.
        assert_eq!(nearest(&sizes, 128), Some(1));
        assert_eq!(nearest(&[], 44), None);
        assert_eq!(nearest(&[(0, 0)], 44), None);
    }

    #[test]
    fn a_label_loses_the_mark_for_its_key() {
        assert_eq!(plain("_Open"), "Open");
        assert_eq!(plain("Save _As…"), "Save As…");
        assert_eq!(plain("snake__case"), "snake_case");
        assert_eq!(plain("Quit"), "Quit");
    }

    #[test]
    fn a_menu_is_laid_out_in_lines() {
        let line = |id: i32, label: &str| Node {
            id,
            label: label.to_string(),
            ..Default::default()
        };
        let separator = |id: i32| Node {
            id,
            separator: true,
            ..Default::default()
        };
        let root = Node {
            children: vec![
                separator(1),
                line(2, "_Open"),
                Node {
                    toggle: Some((Toggle::Check, true)),
                    ..line(3, "Rotate")
                },
                Node {
                    visible: false,
                    ..line(4, "Hidden")
                },
                separator(5),
                separator(6),
                Node {
                    children: vec![
                        Node {
                            enabled: false,
                            ..line(8, "One")
                        },
                        Node {
                            toggle: Some((Toggle::Radio, false)),
                            ..line(9, "Two")
                        },
                    ],
                    ..line(7, "More")
                },
                separator(10),
            ],
            ..Default::default()
        };
        let shown = lines(&root);
        let read: Vec<(i32, &str)> = shown
            .iter()
            .map(|entry| (entry.id, entry.label.as_str()))
            .collect();
        // The menu itself: the submenu's lines are not among its own.
        assert_eq!(read, [(2, "Open"), (3, "Rotate"), (5, ""), (7, "More")]);
        assert!(shown[2].separator);
        assert_eq!(shown[1].toggle, Some((Toggle::Check, true)));
        assert!(shown[3].submenu && !shown[0].submenu);
        // The submenu, gone down into.
        let (more, held) = root.at(&[7]);
        assert_eq!((more.id, held), (7, 1));
        let under = lines(more);
        assert_eq!(under.len(), 2);
        assert!(!under[0].enabled);
        assert_eq!(under[1].toggle, Some((Toggle::Radio, false)));
        // A submenu that is gone (the program made its menu anew): as
        // far as the path still holds.
        assert_eq!(root.at(&[7, 99]).1, 1);
        assert_eq!(root.at(&[42]).1, 0);
        assert_eq!(root.find(9).map(|line| line.label.as_str()), Some("Two"));
        assert!(lines(&Node::default()).is_empty());
    }

    #[test]
    fn the_keys_of_a_line_are_written_as_one_reads_them() {
        let way = |keys: &[&str]| keys.iter().map(|key| key.to_string()).collect::<Vec<_>>();
        assert_eq!(keys_of(&[way(&["Control", "S"])]), "Ctrl+S");
        assert_eq!(
            keys_of(&[way(&["Control", "Shift", "Q"]), way(&["F4"])]),
            "Ctrl+Shift+Q, F4"
        );
        assert_eq!(keys_of(&[]), "");
        assert_eq!(keys_of(&[Vec::new()]), "");
    }

    #[test]
    fn the_older_icons_program_runs_only_where_it_can_do_something() {
        assert!(older_wanted(true, true, Some(":0"), true));
        // Switched off; no tray of gevrek's; no X11; not installed.
        assert!(!older_wanted(false, true, Some(":0"), true));
        assert!(!older_wanted(true, false, Some(":0"), true));
        assert!(!older_wanted(true, true, None, true));
        assert!(!older_wanted(true, true, Some(""), true));
        assert!(!older_wanted(true, true, Some(":0"), false));
    }

    #[test]
    fn a_notch_of_the_wheel_is_told_as_mice_count_it() {
        assert_eq!(wheel_units(1), 120);
        assert_eq!(wheel_units(-2), -240);
    }

    #[test]
    fn an_icon_without_a_picture_has_a_letter() {
        let icon = |title: &str, hint: &str| Item {
            key: String::new(),
            name: String::new(),
            title: title.to_string(),
            hint: hint.to_string(),
            picture: None,
            stamp: 0,
            menu_only: false,
            passive: false,
            attention: false,
        };
        assert_eq!(icon("walle", "").letter(), "W");
        assert_eq!(icon("", "- sync").letter(), "S");
        assert_eq!(icon("", "").letter(), "?");
    }
}
