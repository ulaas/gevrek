//! The clipboard, kept.
//!
//! On Wayland what was copied belongs to the program it was copied from:
//! when that program closes, the clipboard is empty. This keeps a copy of
//! each text that is copied, puts the last one back when its owner goes,
//! and remembers the last few to choose from again (the launcher lists
//! them after a `:`). A picture that is copied is kept the same way while
//! this program runs (it is not remembered among the texts, nor for the
//! next run), and so is the text that is only selected (the primary
//! selection, pasted with the middle button).
//!
//! It listens through the clipboard-manager protocol (data-control) on a
//! connection and a thread of its own. What a password manager marks as
//! secret is not kept.

use smithay_client_toolkit::reexports::calloop::{self, EventLoop};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

/// How many texts are remembered, and how long one may be.
pub const REMEMBERED: usize = 30;
const LONGEST: usize = 256 * 1024;
/// How large a picture may be to be kept.
const LARGEST_PICTURE: usize = 32 * 1024 * 1024;
/// The kinds of content that are text, the best first.
const TEXT: [&str; 4] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
];
/// The kind of content a picture is offered as.
const PICTURE: &str = "image/png";
/// A kind only this program's own offers have: how it knows them again.
const OWN: &str = "application/x-gevrek-clipboard-keeper";
/// What password managers put on a copy that is not to be kept.
const SECRET: &str = "x-kde-passwordManagerHint";

/// The texts that were copied, the newest first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct History(Vec<String>);

impl History {
    /// Take a text in: at the front, once, and only if it is worth it.
    pub fn remember(&mut self, text: &str) {
        if text.trim().is_empty() || text.len() > LONGEST {
            return;
        }
        self.0.retain(|known| known != text);
        self.0.insert(0, text.to_string());
        self.0.truncate(REMEMBERED);
    }

    pub fn texts(&self) -> &[String] {
        &self.0
    }

    pub fn newest(&self) -> Option<&str> {
        self.0.first().map(String::as_str)
    }

    /// Where the texts are kept between two runs of the shell.
    fn file() -> std::path::PathBuf {
        gevrek_model::paths::state_dir().join("clipboard.json")
    }

    /// The texts of the last run, when there are any.
    pub fn load() -> Self {
        Self::read(&Self::file())
    }

    fn read(path: &std::path::Path) -> Self {
        let texts = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .unwrap_or_default();
        let mut history = Self::default();
        // (Through `remember`, oldest first: what it would not take in
        // today is left out, and no more are kept than it keeps.)
        for text in texts.iter().rev() {
            history.remember(text);
        }
        history
    }

    /// Keep the texts for the next run: a file only its owner may read
    /// (what was copied is nobody else's to see).
    pub fn save(&self) {
        self.write(&Self::file());
    }

    fn write(&self, path: &std::path::Path) {
        let Ok(text) = serde_json::to_string(&self.0) else {
            return;
        };
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(text.as_bytes()) {
                    log::debug!(
                        "the clipboard's texts are not kept: {}: {error}",
                        path.display()
                    );
                }
            }
            Err(error) => log::debug!("the clipboard's texts are not kept: {error}"),
        }
    }
}

/// What this program holds on the clipboard.
#[derive(Clone)]
enum Held {
    Text(String),
    /// A picture, as a PNG file's bytes.
    Picture(Arc<Vec<u8>>),
}

/// What was read from another program's offer.
enum Arrived {
    /// A text that was copied.
    Text(String),
    /// A picture that was copied (a PNG file's bytes).
    Picture(Vec<u8>),
    /// A text that was selected.
    Selected(String),
}

/// Which of an offer's kinds is read: a text where there is one, else a
/// picture; nothing of what a password manager marks as secret, and
/// nothing of this program's own offers.
fn kind_to_read(kinds: &[String]) -> Option<&'static str> {
    if kinds.iter().any(|kind| kind == SECRET || kind == OWN) {
        return None;
    }
    TEXT.iter()
        .copied()
        .find(|kind| kinds.iter().any(|known| known == kind))
        .or_else(|| kinds.iter().any(|kind| kind == PICTURE).then_some(PICTURE))
}

/// The keeper, as the shell holds it.
pub struct Clipboard {
    pub history: Arc<Mutex<History>>,
    put: calloop::channel::Sender<String>,
    pictures: calloop::channel::Sender<Vec<u8>>,
}

impl Clipboard {
    /// Make `text` what is on the clipboard.
    pub fn put(&self, text: &str) {
        let _ = self.put.send(text.to_string());
    }

    /// Make a picture (a PNG file's bytes) what is on the clipboard.
    pub fn put_picture(&self, png: Vec<u8>) {
        let _ = self.pictures.send(png);
    }
}

/// What one offer on the clipboard says it has.
#[derive(Default)]
struct Kinds(Mutex<Vec<String>>);

struct Keeper {
    manager: ZwlrDataControlManagerV1,
    device: ZwlrDataControlDeviceV1,
    history: Arc<Mutex<History>>,
    /// What this program offers on the clipboard now, while it is the owner.
    own: Option<(ZwlrDataControlSourceV1, Held)>,
    /// The same for the selection.
    own_selected: Option<(ZwlrDataControlSourceV1, String)>,
    /// What was copied last, by whomever: put back when its owner goes.
    last: Option<Held>,
    /// The text that was selected last.
    last_selected: Option<String>,
    /// What is read from other programs' offers arrives here.
    read: calloop::channel::Sender<Arrived>,
    /// The clipboard was announced empty and nothing has come since.
    empty: bool,
    /// The same for the selection.
    selected_empty: bool,
    /// To look again a moment after it was announced empty.
    timers: calloop::LoopHandle<'static, Keeper>,
    handle: QueueHandle<Keeper>,
}

/// Start keeping the clipboard. An error when the compositor has no
/// clipboard-manager protocol.
pub fn start() -> anyhow::Result<Clipboard> {
    let connection = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<Keeper>(&connection)?;
    let handle = queue.handle();
    let manager: ZwlrDataControlManagerV1 = globals
        .bind(&handle, 1..=2, ())
        .map_err(|_| anyhow::anyhow!("the compositor has no clipboard-manager protocol"))?;
    let seat: wl_seat::WlSeat = globals
        .bind(&handle, 1..=5, ())
        .map_err(|_| anyhow::anyhow!("there is no seat"))?;
    let device = manager.get_data_device(&seat, &handle, ());
    // (What was copied in the last run is there to choose from again.)
    let history = Arc::new(Mutex::new(History::load()));
    let (put, puts) = calloop::channel::channel::<String>();
    let (pictures, picture_puts) = calloop::channel::channel::<Vec<u8>>();
    let (read, reads) = calloop::channel::channel::<Arrived>();
    let shared = Arc::clone(&history);
    std::thread::Builder::new()
        .name("gevrek-clipboard".to_string())
        .spawn(move || {
            let mut event_loop = match EventLoop::<Keeper>::try_new() {
                Ok(event_loop) => event_loop,
                Err(error) => {
                    log::warn!("the clipboard is not kept: {error}");
                    return;
                }
            };
            let mut keeper = Keeper {
                manager,
                device,
                history: shared,
                own: None,
                own_selected: None,
                last: None,
                last_selected: None,
                read,
                empty: false,
                selected_empty: false,
                timers: event_loop.handle(),
                handle: handle.clone(),
            };
            if let Err(error) = WaylandSource::new(connection, queue).insert(event_loop.handle()) {
                log::warn!("the clipboard is not kept: {error}");
                return;
            }
            let offering = handle.clone();
            let _ = event_loop
                .handle()
                .insert_source(puts, move |event, _, keeper| {
                    if let calloop::channel::Event::Msg(text) = event {
                        keeper.remember(&text);
                        keeper.offer(Held::Text(text), &offering);
                    }
                });
            let offering = handle.clone();
            let _ = event_loop
                .handle()
                .insert_source(picture_puts, move |event, _, keeper| {
                    if let calloop::channel::Event::Msg(png) = event {
                        keeper.offer(Held::Picture(Arc::new(png)), &offering);
                    }
                });
            let _ = event_loop
                .handle()
                .insert_source(reads, |event, _, keeper| match event {
                    calloop::channel::Event::Msg(Arrived::Text(text)) => {
                        keeper.remember(&text);
                        keeper.last = Some(Held::Text(text));
                    }
                    calloop::channel::Event::Msg(Arrived::Picture(png)) => {
                        keeper.last = Some(Held::Picture(Arc::new(png)));
                    }
                    calloop::channel::Event::Msg(Arrived::Selected(text)) => {
                        keeper.last_selected = Some(text);
                    }
                    calloop::channel::Event::Closed => {}
                });
            if let Err(error) = event_loop.run(None, &mut keeper, |_| {}) {
                log::warn!("the clipboard is no longer kept: {error}");
            }
        })?;
    Ok(Clipboard {
        history,
        put,
        pictures,
    })
}

impl Keeper {
    /// Take a text into the history, and keep the history for the next
    /// run when that changed it.
    fn remember(&mut self, text: &str) {
        let mut history = self
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = history.clone();
        history.remember(text);
        if *history != before {
            history.save();
        }
    }

    /// Become the owner of the clipboard, with a text or a picture on it.
    fn offer(&mut self, held: Held, handle: &QueueHandle<Self>) {
        let source = self.manager.create_data_source(handle, ());
        match &held {
            Held::Text(_) => {
                for kind in TEXT {
                    source.offer(kind.to_string());
                }
            }
            Held::Picture(_) => source.offer(PICTURE.to_string()),
        }
        source.offer(OWN.to_string());
        self.empty = false;
        self.device.set_selection(Some(&source));
        self.last = Some(held.clone());
        if let Some((old, _)) = self.own.replace((source, held)) {
            old.destroy();
        }
    }

    /// Become the owner of the selection, with a text in it.
    fn offer_selected(&mut self, text: String, handle: &QueueHandle<Self>) {
        // (The selection came with the protocol's second version.)
        if self.device.version() < 2 {
            return;
        }
        let source = self.manager.create_data_source(handle, ());
        for kind in TEXT {
            source.offer(kind.to_string());
        }
        source.offer(OWN.to_string());
        self.selected_empty = false;
        self.device.set_primary_selection(Some(&source));
        if let Some((old, _)) = self.own_selected.replace((source, text)) {
            old.destroy();
        }
    }

    /// Read what another program offers, on a thread of its own (the
    /// other program writes when it gets to it).
    fn read_offer(
        &self,
        offer: &ZwlrDataControlOfferV1,
        kind: &'static str,
        selected: bool,
        connection: &Connection,
    ) {
        let Ok((mut reader, writer)) = std::io::pipe() else {
            return;
        };
        offer.receive(kind.to_string(), writer.as_fd());
        drop(writer);
        let _ = connection.flush();
        let read = self.read.clone();
        let _ = std::thread::Builder::new()
            .name("gevrek-clipboard-read".to_string())
            .spawn(move || {
                let most = if kind == PICTURE {
                    LARGEST_PICTURE
                } else {
                    LONGEST
                };
                let mut bytes = Vec::new();
                if reader
                    .by_ref()
                    .take(most as u64 + 1)
                    .read_to_end(&mut bytes)
                    .is_err()
                    || bytes.len() > most
                    || bytes.is_empty()
                {
                    return;
                }
                let arrived = if kind == PICTURE {
                    Some(Arrived::Picture(bytes))
                } else {
                    String::from_utf8(bytes).ok().map(|text| {
                        if selected {
                            Arrived::Selected(text)
                        } else {
                            Arrived::Text(text)
                        }
                    })
                };
                if let Some(arrived) = arrived {
                    let _ = read.send(arrived);
                }
            });
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for Keeper {
    fn event(
        keeper: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        connection: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let kinds_of = |offer: &ZwlrDataControlOfferV1| {
            offer
                .data::<Kinds>()
                .map(|kinds| kinds.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
                .unwrap_or_default()
        };
        let _ = handle;
        match event {
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) } => {
                keeper.empty = false;
                let kinds = kinds_of(&offer);
                // This program's own offer, announced back to it.
                if kinds.iter().any(|kind| kind == OWN) {
                    offer.destroy();
                    return;
                }
                // Another program copied something: this one is no longer
                // the owner, whether or not it was told so yet.
                if let Some((source, _)) = keeper.own.take() {
                    source.destroy();
                }
                match kind_to_read(&kinds) {
                    Some(kind) => keeper.read_offer(&offer, kind, false, connection),
                    // Nothing that is kept (a secret, a kind this program
                    // does not know): what was copied before is not put
                    // back in its place when it goes.
                    None => keeper.last = None,
                }
                offer.destroy();
            }
            zwlr_data_control_device_v1::Event::Selection { id: None } => {
                // Announced empty: the owner is gone and took the content
                // along, or another program is about to put something
                // there (a change of owner can pass through "empty").
                // Looked at again in a moment; if it is still empty then,
                // what was copied last is put back.
                keeper.empty = true;
                let timer =
                    calloop::timer::Timer::from_duration(std::time::Duration::from_millis(250));
                let _ = keeper.timers.insert_source(timer, |_, _, keeper| {
                    if keeper.empty && keeper.own.is_none() {
                        let last = keeper.last.clone().or_else(|| {
                            keeper
                                .history
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .newest()
                                .map(|text| Held::Text(text.to_string()))
                        });
                        if let Some(held) = last {
                            let handle = keeper.handle.clone();
                            keeper.offer(held, &handle);
                        }
                    }
                    calloop::timer::TimeoutAction::Drop
                });
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                keeper.selected_empty = false;
                let kinds = kinds_of(&offer);
                if kinds.iter().any(|kind| kind == OWN) {
                    offer.destroy();
                    return;
                }
                if let Some((source, _)) = keeper.own_selected.take() {
                    source.destroy();
                }
                // (Only a text: a selection is one.)
                match kind_to_read(&kinds).filter(|kind| *kind != PICTURE) {
                    Some(kind) => keeper.read_offer(&offer, kind, true, connection),
                    None => keeper.last_selected = None,
                }
                offer.destroy();
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: None } => {
                keeper.selected_empty = true;
                let timer =
                    calloop::timer::Timer::from_duration(std::time::Duration::from_millis(250));
                let _ = keeper.timers.insert_source(timer, |_, _, keeper| {
                    if keeper.selected_empty && keeper.own_selected.is_none() {
                        if let Some(text) = keeper.last_selected.clone() {
                            let handle = keeper.handle.clone();
                            keeper.offer_selected(text, &handle);
                        }
                    }
                    calloop::timer::TimeoutAction::Drop
                });
            }
            _ => {}
        }
    }

    event_created_child!(Keeper, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, Kinds::default()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, Kinds> for Keeper {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        kinds: &Kinds,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            kinds
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for Keeper {
    fn event(
        keeper: &mut Self,
        source: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // A program pastes: it is given the text.
            zwlr_data_control_source_v1::Event::Send { fd, .. } => {
                let copied = keeper.own.as_ref().filter(|(own, _)| own == source).map(
                    |(_, held)| match held {
                        Held::Text(text) => Arc::new(text.clone().into_bytes()),
                        Held::Picture(png) => Arc::clone(png),
                    },
                );
                let selected = keeper
                    .own_selected
                    .as_ref()
                    .filter(|(own, _)| own == source)
                    .map(|(_, text)| Arc::new(text.clone().into_bytes()));
                if let Some(bytes) = copied.or(selected) {
                    // Written on a thread of its own: a program that reads
                    // slowly must not hold the clipboard up.
                    // (How it went, never what it was.)
                    if let Err(error) = std::thread::Builder::new()
                        .name("gevrek-clipboard-write".to_string())
                        .spawn(move || {
                            let mut file = std::fs::File::from(fd);
                            if let Err(error) = file.write_all(&bytes) {
                                log::debug!("a program did not take all that was pasted: {error}");
                            }
                        })
                    {
                        log::debug!("what was pasted was not handed over: {error}");
                    }
                }
            }
            // Another program owns the clipboard now.
            zwlr_data_control_source_v1::Event::Cancelled => {
                if keeper.own.as_ref().is_some_and(|(own, _)| own == source) {
                    keeper.own = None;
                }
                if keeper
                    .own_selected
                    .as_ref()
                    .is_some_and(|(own, _)| own == source)
                {
                    keeper.own_selected = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($interface:ty, $data:ty) => {
        impl Dispatch<$interface, $data> for Keeper {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &$data,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}
ignore!(wl_registry::WlRegistry, GlobalListContents);
ignore!(wl_seat::WlSeat, ());
ignore!(ZwlrDataControlManagerV1, ());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_read_before_a_picture_and_a_secret_not_at_all() {
        let kinds =
            |names: &[&str]| -> Vec<String> { names.iter().map(|name| name.to_string()).collect() };
        assert_eq!(
            kind_to_read(&kinds(&["text/html", "text/plain", "image/png"])),
            Some("text/plain")
        );
        assert_eq!(
            kind_to_read(&kinds(&["image/png", "image/bmp"])),
            Some("image/png")
        );
        assert_eq!(kind_to_read(&kinds(&["image/bmp"])), None);
        assert_eq!(kind_to_read(&kinds(&["text/plain", SECRET])), None);
        assert_eq!(kind_to_read(&kinds(&["text/plain", OWN])), None);
        assert_eq!(kind_to_read(&[]), None);
    }

    #[test]
    fn what_is_copied_is_remembered_once_the_newest_first() {
        let mut history = History::default();
        history.remember("one");
        history.remember("two");
        history.remember("one");
        assert_eq!(history.texts(), ["one", "two"]);
        assert_eq!(history.newest(), Some("one"));
        // Nothing, and what is too long to be meant, is not kept.
        history.remember("   \n");
        history.remember(&"x".repeat(LONGEST + 1));
        assert_eq!(history.texts().len(), 2);
        for index in 0..REMEMBERED + 5 {
            history.remember(&format!("text {index}"));
        }
        assert_eq!(history.texts().len(), REMEMBERED);
        assert_eq!(
            history.newest(),
            Some(format!("text {}", REMEMBERED + 4).as_str())
        );
    }

    #[test]
    fn the_texts_are_kept_for_the_next_run_in_a_file_of_the_owners() {
        let folder = std::env::temp_dir().join(format!("gevrek-clipboard-{}", std::process::id()));
        let path = folder.join("deep").join("clipboard.json");
        let mut history = History::default();
        history.remember("first");
        history.remember("second");
        history.write(&path);
        let again = History::read(&path);
        assert_eq!(again.texts(), ["second".to_string(), "first".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "only its owner may read it");
        }
        // A file that is not what was written is nothing to read from.
        std::fs::write(&path, "not json").unwrap();
        assert!(History::read(&path).texts().is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// What was copied is a file anyone may have written: no more texts
    /// are taken from it than are kept, none longer than one may be, and
    /// what is no list of texts is none.
    #[test]
    fn texts_from_outside_are_lived_with() {
        let folder =
            std::env::temp_dir().join(format!("gevrek-clipboard-outside-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("clipboard.json");

        // More than are kept, one of them longer than a text may be,
        // some of them the same, some of them nothing.
        let mut many: Vec<String> = (0..REMEMBERED * 3)
            .map(|index| format!("text {index}"))
            .collect();
        many.push("x".repeat(LONGEST + 1));
        many.push("text 0".to_string());
        many.push("  \n".to_string());
        std::fs::write(&path, serde_json::to_string(&many).unwrap()).unwrap();
        let read = History::read(&path);
        assert_eq!(read.texts().len(), REMEMBERED);
        assert!(read
            .texts()
            .iter()
            .all(|text| text.len() <= LONGEST && !text.trim().is_empty()));
        assert_eq!(read.newest(), Some("text 0"));
        let mut once = read.texts().to_vec();
        once.sort();
        once.dedup();
        assert_eq!(once.len(), REMEMBERED, "each text once");

        for nonsense in [
            "",
            "null",
            "7",
            "{}",
            r#"{"0": "a"}"#,
            r#"["a", 1]"#,
            r#"["a", null]"#,
            r#"[["a"]]"#,
            r#"["a""#,
            "[\"\\ud800\"]",
        ] {
            std::fs::write(&path, nonsense).unwrap();
            assert!(History::read(&path).texts().is_empty(), "{nonsense}");
        }
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(History::read(&path).texts().is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
