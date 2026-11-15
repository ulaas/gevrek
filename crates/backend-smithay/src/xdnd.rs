//! Drag and drop between Wayland programs and X11 programs.
//!
//! Xwayland carries the clipboard across with the window manager's
//! help, and a drag not at all: what a Wayland program drags is nothing
//! to an X11 window under the pointer. gevrek stands in.
//!
//! **From a Wayland program onto an X11 window** (this file's first
//! half): while such a drag is over an X11 window, gevrek is the drag's
//! source as X11 programs know one (XDND): it owns the drag's selection
//! on the X server, tells the window that the drag enters, where it is,
//! that it leaves, and that it is dropped, and hands over the data when
//! the window asks for it. The data is read from the Wayland program
//! once, when the drag first comes over an X11 window, in every kind
//! the program offers (a few, and not without end): the Wayland side
//! ends the drag as not taken when it is let go over a window that is
//! not Wayland's, and its program may be asked for nothing afterwards.
//! What is dropped is always a copy.
//!
//! All of it is done on a line to the X server of its own, on a thread
//! of its own, which lives as long as one drag: whatever goes wrong
//! there ends that drag and nothing else.

use std::io::Read;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, PropMode,
    SelectionNotifyEvent, SelectionRequestEvent, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

/// The most kinds of data that are read from a drag, and the most of
/// each.
pub const MOST_KINDS: usize = 12;
const MOST_BYTES: usize = 32 << 20;
/// How long the Wayland program is waited for, for each kind.
const PATIENCE: Duration = Duration::from_millis(1500);
/// How long the X11 window is given to say that it takes a drop.
const FOR_AN_ANSWER: Duration = Duration::from_millis(500);
/// How long the X11 window is waited for after the drop.
const AFTER_DROP: Duration = Duration::from_secs(8);

/// What the drag does, told to its thread.
#[derive(Debug)]
pub enum Told {
    /// It is over this X11 window, at this place of the X screen.
    Over { window: u32, at: (i32, i32) },
    /// It is over no X11 window any more.
    Away,
    /// It was let go (over the window it was last over, if any).
    Dropped,
    /// It ended otherwise.
    Ended,
}

/// A drag that came over an X11 window: the way to its thread, and the
/// window it is over.
pub struct Out {
    tell: mpsc::Sender<Told>,
    pub over: Option<u32>,
}

impl Out {
    /// Begin: `kinds` are the kinds of data the drag offers, each with
    /// the end of a pipe its program writes that data into.
    pub fn begin(display: u32, kinds: Vec<(String, OwnedFd)>) -> Option<Self> {
        let (tell, told) = mpsc::channel();
        std::thread::Builder::new()
            .name("gevrek-xdnd".to_string())
            .spawn(move || {
                if let Err(error) = source(display, kinds, told) {
                    log::debug!("a drag onto an X11 window: {error}");
                }
            })
            .ok()?;
        Some(Self { tell, over: None })
    }

    pub fn tell(&mut self, told: Told) {
        match &told {
            Told::Over { window, .. } => self.over = Some(*window),
            Told::Away => self.over = None,
            _ => {}
        }
        let _ = self.tell.send(told);
    }
}

/// The names X11 programs ask for a kind of data by: its own, and for
/// text the older names too.
pub fn names_of(kind: &str) -> Vec<&str> {
    match kind {
        "text/plain;charset=utf-8" => vec![kind, "UTF8_STRING"],
        "text/plain" => vec![kind, "STRING", "TEXT"],
        _ => vec![kind],
    }
}

/// Read what a program writes into a pipe, to its end; not longer than
/// it is waited for, and not more than there may be.
fn read_all(pipe: OwnedFd) -> Vec<u8> {
    let (done, read) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("gevrek-xdnd-read".to_string())
        .spawn(move || {
            let mut data = Vec::new();
            let mut file = std::fs::File::from(pipe);
            let mut piece = [0u8; 65536];
            while let Ok(count) = file.read(&mut piece) {
                if count == 0 || data.len() + count > MOST_BYTES {
                    break;
                }
                data.extend_from_slice(&piece[..count]);
            }
            let _ = done.send(data);
        });
    match reader {
        Ok(_) => read.recv_timeout(PATIENCE).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// The thread of one drag: gevrek as the drag's source for X11 windows.
fn source(
    display: u32,
    kinds: Vec<(String, OwnedFd)>,
    told: mpsc::Receiver<Told>,
) -> anyhow::Result<()> {
    let (x, screen) = x11rb::connect(Some(&format!(":{display}")))?;
    let root = x
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| anyhow::anyhow!("no X screen"))?
        .root;
    let atom = |name: &str| -> anyhow::Result<u32> {
        Ok(x.intern_atom(false, name.as_bytes())?.reply()?.atom)
    };
    let selection = atom("XdndSelection")?;
    let (aware, enter, position, status, leave, drop, finished) = (
        atom("XdndAware")?,
        atom("XdndEnter")?,
        atom("XdndPosition")?,
        atom("XdndStatus")?,
        atom("XdndLeave")?,
        atom("XdndDrop")?,
        atom("XdndFinished")?,
    );
    let (copy, type_list, targets) = (
        atom("XdndActionCopy")?,
        atom("XdndTypeList")?,
        atom("TARGETS")?,
    );
    // The data, by every name it is asked for.
    let mut data: Vec<(u32, Vec<u8>)> = Vec::new();
    for (kind, pipe) in kinds {
        let bytes = read_all(pipe);
        for name in names_of(&kind) {
            let name = atom(name)?;
            if !data.iter().any(|(known, _)| *known == name) {
                data.push((name, bytes.clone()));
            }
        }
    }
    let kinds: Vec<u32> = data.iter().map(|(name, _)| *name).collect();
    // A window that is never shown: the drag's source, and the owner of
    // its selection.
    let own = x.generate_id()?;
    x.create_window(
        0,
        own,
        root,
        -10,
        -10,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )?;
    x.change_property32(PropMode::REPLACE, own, type_list, AtomEnum::ATOM, &kinds)?;
    x.set_selection_owner(own, selection, x11rb::CURRENT_TIME)?;
    x.flush()?;
    let message = |to: u32, kind: u32, numbers: [u32; 5]| -> anyhow::Result<()> {
        x.send_event(
            false,
            to,
            EventMask::NO_EVENT,
            ClientMessageEvent::new(32, to, kind, numbers),
        )?;
        Ok(())
    };
    // The version of the protocol a window speaks; none: it takes no drags.
    let speaks = |window: u32| -> Option<u32> {
        let reply = x
            .get_property(false, window, aware, AtomEnum::ANY, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let version = reply.value32()?.next();
        version.map(|version| version.min(5))
    };
    // The window the drag is over (with its version), whether it said
    // that it would take the drop, and when the drop was.
    let mut over: Option<(u32, u32)> = None;
    let mut takes = false;
    let mut dropped: Option<Instant> = None;
    let mut let_go: Option<Instant> = None;
    let mut ended = false;
    loop {
        loop {
            match told.try_recv() {
                Ok(Told::Over { window, at }) => {
                    if over.map(|(known, _)| known) != Some(window) {
                        if let Some((old, _)) = over.take() {
                            message(old, leave, [own, 0, 0, 0, 0])?;
                        }
                        takes = false;
                        if let Some(version) = speaks(window) {
                            let more = u32::from(kinds.len() > 3);
                            let kind = |index: usize| kinds.get(index).copied().unwrap_or(0);
                            message(
                                window,
                                enter,
                                [own, (version << 24) | more, kind(0), kind(1), kind(2)],
                            )?;
                            over = Some((window, version));
                        }
                    }
                    if let Some((window, _)) = over {
                        let place = ((at.0.max(0) as u32) << 16) | (at.1.max(0) as u32 & 0xffff);
                        message(window, position, [own, 0, place, x11rb::CURRENT_TIME, copy])?;
                    }
                }
                Ok(Told::Away) => {
                    if let Some((old, _)) = over.take() {
                        message(old, leave, [own, 0, 0, 0, 0])?;
                    }
                    takes = false;
                }
                Ok(Told::Dropped) => {
                    log::debug!("xdnd: let go (over {over:?}, taken: {takes})");
                    let_go = Some(Instant::now());
                }
                Ok(Told::Ended) => {
                    ended = true;
                    break;
                }
                // (Nobody tells any more: the end, unless it was let go
                // over a window, which is then waited for.)
                Err(mpsc::TryRecvError::Disconnected) => {
                    ended = let_go.is_none();
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        x.flush()?;
        while let Some(event) = x.poll_for_event()? {
            match event {
                Event::ClientMessage(said) if said.type_ == status => {
                    takes = said.data.as_data32()[1] & 1 != 0;
                    log::debug!(
                        "xdnd: the window says it {} the drop",
                        if takes { "takes" } else { "does not take" }
                    );
                }
                Event::ClientMessage(said) if said.type_ == finished => {
                    log::debug!("xdnd: the window is done with the drop");
                    ended = true;
                }
                Event::SelectionRequest(asked) if asked.selection == selection => {
                    log::debug!("xdnd: asked for the data as {}", asked.target);
                    answer(&x, &asked, &data, targets)?;
                }
                _ => {}
            }
        }
        // Let go: dropped where the window said it takes the drop (it is
        // given a moment to say so), else the drag leaves it.
        if let (Some(since), None) = (let_go, dropped) {
            match over {
                Some((window, _)) if takes => {
                    message(window, drop, [own, 0, x11rb::CURRENT_TIME, 0, 0])?;
                    dropped = Some(Instant::now());
                }
                Some(_) if since.elapsed() < FOR_AN_ANSWER => {}
                _ => ended = true,
            }
        }
        if ended || dropped.is_some_and(|at| at.elapsed() > AFTER_DROP) {
            break;
        }
        x.flush()?;
        std::thread::sleep(Duration::from_millis(6));
    }
    if let (Some((old, _)), None) = (over, dropped) {
        let _ = message(old, leave, [own, 0, 0, 0, 0]);
    }
    let _ = x.destroy_window(own);
    let _ = x.flush();
    Ok(())
}

/// Hand the data over to a window that asks for it: the kinds there
/// are (`TARGETS`), or one of them, written on the window in pieces.
fn answer(
    x: &impl Connection,
    asked: &SelectionRequestEvent,
    data: &[(u32, Vec<u8>)],
    targets: u32,
) -> anyhow::Result<()> {
    // (An older program names no place to write to: the kind's own name.)
    let place = if asked.property == x11rb::NONE {
        asked.target
    } else {
        asked.property
    };
    let mut written = x11rb::NONE;
    if asked.target == targets {
        let kinds: Vec<u32> = data.iter().map(|(name, _)| *name).collect();
        x.change_property32(
            PropMode::REPLACE,
            asked.requestor,
            place,
            AtomEnum::ATOM,
            &kinds,
        )?;
        written = place;
    } else if let Some((_, bytes)) = data.iter().find(|(name, _)| *name == asked.target) {
        let mut mode = PropMode::REPLACE;
        // (In pieces: one request to the X server holds only so much.)
        let mut pieces = bytes.chunks(60_000).peekable();
        if pieces.peek().is_none() {
            x.change_property8(mode, asked.requestor, place, asked.target, &[])?;
        }
        for piece in pieces {
            x.change_property8(mode, asked.requestor, place, asked.target, piece)?;
            mode = PropMode::APPEND;
        }
        written = place;
    }
    x.send_event(
        false,
        asked.requestor,
        EventMask::NO_EVENT,
        SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: asked.time,
            requestor: asked.requestor,
            selection: asked.selection,
            target: asked.target,
            property: written,
        },
    )?;
    x.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------
// From an X11 program onto a Wayland window.
//
// An X11 program that drags looks for a window under the pointer that
// takes drags. Over a Wayland window there is none it could know of, so
// gevrek puts one there for as long as the pointer is over something
// that is not X11's: a window nobody sees, as large as the X screen,
// that says it takes drags. What the X11 program tells that window is
// carried over: when the drag enters it, gevrek begins a drag of its own
// on the Wayland side with the kinds of data the X11 program offers;
// the Wayland window under the pointer is offered them as from any
// Wayland program, and what it asks for is fetched from the X11 program
// (the drag's selection, converted). The X11 program learns of the
// button being let go from gevrek, tells the unseen window that the
// drag is dropped, and is told when the Wayland side is done.

/// What the X11 side of such a drag tells the compositor.
#[derive(Debug)]
pub enum FromX {
    /// An X11 program drags something (or the drag is over).
    Dragging(bool),
    /// The drag came over the unseen window, with these kinds of data.
    Entered(Vec<String>),
    /// It left it again (it is over an X11 window, or was given up).
    Left,
}

/// What the compositor tells the X11 side.
#[derive(Debug)]
pub enum ToX {
    /// The pointer is over something that is not X11's: the unseen
    /// window is to be there (or no longer).
    Stand(bool),
    /// A Wayland program asks for the data in this kind: to be written
    /// into this pipe.
    Fetch(String, OwnedFd),
    /// The Wayland side is done with the drag: taken, or not.
    Done(bool),
}

/// The kinds of data a Wayland window is offered for what an X11
/// program offers, each with the name it is fetched by from the X11
/// program. Texts under their older X11 names are offered as texts.
pub fn offered(names: &[String]) -> Vec<(String, String)> {
    let mut kinds: Vec<(String, String)> = Vec::new();
    let mut add = |kind: &str, by: &str| {
        if !kinds.iter().any(|(known, _)| known == kind) {
            kinds.push((kind.to_string(), by.to_string()));
        }
    };
    for name in names {
        if name.contains('/') {
            add(name, name);
        }
    }
    for name in names {
        match name.as_str() {
            "UTF8_STRING" => add("text/plain;charset=utf-8", name),
            "STRING" | "TEXT" => add("text/plain", name),
            _ => {}
        }
    }
    kinds
}

/// Watch an X server for drags of its programs, for as long as it is
/// there. `tell` is called (on the thread made here) with what happens;
/// what is returned takes what the compositor has to say.
pub fn watch(display: u32, tell: impl Fn(FromX) + Send + 'static) -> Option<mpsc::Sender<ToX>> {
    let (to_x, said) = mpsc::channel();
    std::thread::Builder::new()
        .name("gevrek-xdnd-watch".to_string())
        .spawn(move || {
            if let Err(error) = target(display, &tell, said) {
                log::debug!("drags of X11 programs are no longer watched: {error}");
            }
            tell(FromX::Dragging(false));
        })
        .ok()?;
    Some(to_x)
}

/// What is being fetched from the X11 program: where it is written to.
struct Fetching {
    pipe: std::fs::File,
    /// It comes in pieces (the X server's way with large data).
    pieces: bool,
}

/// The thread that is gevrek as a window that takes drags.
fn target(display: u32, tell: &impl Fn(FromX), said: mpsc::Receiver<ToX>) -> anyhow::Result<()> {
    use std::io::Write;
    use x11rb::protocol::xfixes::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::{ConfigureWindowAux, StackMode};
    let (x, screen) = x11rb::connect(Some(&format!(":{display}")))?;
    let root = x
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| anyhow::anyhow!("no X screen"))?
        .root;
    let atom = |name: &str| -> anyhow::Result<u32> {
        Ok(x.intern_atom(false, name.as_bytes())?.reply()?.atom)
    };
    let selection = atom("XdndSelection")?;
    let (aware, enter, position, status, leave, drop, finished) = (
        atom("XdndAware")?,
        atom("XdndEnter")?,
        atom("XdndPosition")?,
        atom("XdndStatus")?,
        atom("XdndLeave")?,
        atom("XdndDrop")?,
        atom("XdndFinished")?,
    );
    let (copy, type_list, place, incr) = (
        atom("XdndActionCopy")?,
        atom("XdndTypeList")?,
        atom("_GEVREK_DND")?,
        atom("INCR")?,
    );
    x.xfixes_query_version(5, 0)?.reply()?;
    x.xfixes_select_selection_input(
        root,
        selection,
        xfixes::SelectionEventMask::SET_SELECTION_OWNER
            | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
            | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
    )?;
    // The unseen window: it takes no part in what is drawn, and the
    // window manager leaves it alone.
    let own = x.generate_id()?;
    x.create_window(
        0,
        own,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    x.change_property32(PropMode::REPLACE, own, aware, AtomEnum::ATOM, &[5])?;
    x.flush()?;
    // The X11 program that drags (its window, as it told the unseen
    // one), whether it said the drag is dropped, and what is fetched.
    let mut dragging = false;
    let mut standing = false;
    let mut from: Option<u32> = None;
    let mut by_name: Vec<(String, u32)> = Vec::new();
    let mut dropped: Option<Instant> = None;
    let mut done: Option<bool> = None;
    let mut waiting: Vec<(String, OwnedFd)> = Vec::new();
    let mut fetching: Option<Fetching> = None;
    let mut fetched = Instant::now();
    let message = |to: u32, kind: u32, numbers: [u32; 5]| -> anyhow::Result<()> {
        x.send_event(
            false,
            to,
            EventMask::NO_EVENT,
            ClientMessageEvent::new(32, to, kind, numbers),
        )?;
        Ok(())
    };
    loop {
        // Nothing is dragged: the X server is waited for. While
        // something is, both sides are looked at in turns.
        let event = if dragging || standing || dropped.is_some() {
            x.poll_for_event()?
        } else {
            Some(x.wait_for_event()?)
        };
        match event {
            Some(Event::XfixesSelectionNotify(now)) if now.selection == selection => {
                dragging = now.owner != x11rb::NONE;
                if !dragging && dropped.is_none() {
                    from = None;
                }
                tell(FromX::Dragging(dragging));
            }
            Some(Event::ClientMessage(told)) if told.type_ == enter => {
                let numbers = told.data.as_data32();
                let source = numbers[0];
                let mut kinds: Vec<u32> = numbers[2..5]
                    .iter()
                    .copied()
                    .filter(|kind| *kind != x11rb::NONE)
                    .collect();
                if numbers[1] & 1 != 0 {
                    if let Ok(reply) = x
                        .get_property(false, source, type_list, AtomEnum::ATOM, 0, 256)?
                        .reply()
                    {
                        if let Some(listed) = reply.value32() {
                            kinds = listed.collect();
                        }
                    }
                }
                by_name.clear();
                for kind in kinds {
                    if let Ok(name) = x.get_atom_name(kind)?.reply() {
                        by_name.push((String::from_utf8_lossy(&name.name).into_owned(), kind));
                    }
                }
                from = Some(source);
                dropped = None;
                done = None;
                tell(FromX::Entered(
                    by_name.iter().map(|(name, _)| name.clone()).collect(),
                ));
            }
            Some(Event::ClientMessage(told)) if told.type_ == position => {
                // It is taken (what the Wayland window under the pointer
                // makes of it is its own to decide at the drop).
                let source = told.data.as_data32()[0];
                message(source, status, [own, 1 | 2, 0, 0, copy])?;
            }
            Some(Event::ClientMessage(told)) if told.type_ == leave => {
                if dropped.is_none() {
                    from = None;
                    tell(FromX::Left);
                }
            }
            Some(Event::ClientMessage(told)) if told.type_ == drop => {
                from = Some(told.data.as_data32()[0]);
                dropped = Some(Instant::now());
                fetched = Instant::now();
            }
            Some(Event::SelectionNotify(came)) if came.requestor == own => {
                if let Some(mut going) = fetching.take() {
                    if came.property != x11rb::NONE {
                        let reply = x
                            .get_property(true, own, came.property, AtomEnum::ANY, 0, u32::MAX / 8)?
                            .reply()?;
                        if reply.type_ == incr {
                            going.pieces = true;
                            fetching = Some(going);
                        } else {
                            let _ = going.pipe.write_all(&reply.value);
                        }
                    }
                    fetched = Instant::now();
                }
            }
            Some(Event::PropertyNotify(changed))
                if changed.window == own
                    && changed.atom == place
                    && changed.state == x11rb::protocol::xproto::Property::NEW_VALUE =>
            {
                if let Some(mut going) = fetching.take() {
                    if going.pieces {
                        let reply = x
                            .get_property(true, own, place, AtomEnum::ANY, 0, u32::MAX / 8)?
                            .reply()?;
                        // (A piece of nothing is the end.)
                        if !reply.value.is_empty() {
                            let _ = going.pipe.write_all(&reply.value);
                            fetching = Some(going);
                        }
                        fetched = Instant::now();
                    } else {
                        fetching = Some(going);
                    }
                }
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(4)),
        }
        loop {
            match said.try_recv() {
                Ok(ToX::Stand(there)) if there != standing => {
                    standing = there;
                    if there {
                        // As large as the X screen is now, above all.
                        let whole = x.get_geometry(root)?.reply()?;
                        x.configure_window(
                            own,
                            &ConfigureWindowAux::new()
                                .x(0)
                                .y(0)
                                .width(u32::from(whole.width))
                                .height(u32::from(whole.height))
                                .stack_mode(StackMode::ABOVE),
                        )?;
                        x.map_window(own)?;
                    } else {
                        x.unmap_window(own)?;
                    }
                }
                Ok(ToX::Stand(_)) => {}
                Ok(ToX::Fetch(kind, pipe)) => waiting.push((kind, pipe)),
                Ok(ToX::Done(taken)) => done = Some(taken),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        // The next thing a Wayland program asked for.
        if fetching.is_none() && !waiting.is_empty() {
            let (kind, pipe) = waiting.remove(0);
            let name = by_name
                .iter()
                .find(|(name, _)| *name == kind)
                .map(|(_, name)| *name);
            if let (Some(name), true) = (name, from.is_some()) {
                x.convert_selection(own, selection, name, place, x11rb::CURRENT_TIME)?;
                fetching = Some(Fetching {
                    pipe: std::fs::File::from(pipe),
                    pieces: false,
                });
                fetched = Instant::now();
            }
            // (Else the pipe is closed with nothing in it.)
        }
        // A fetch the X11 program does not answer is given up.
        if fetching.is_some() && fetched.elapsed() > Duration::from_secs(4) {
            fetching = None;
        }
        // Dropped, and the Wayland side done (or long enough waited
        // for): the X11 program is told, and the drag is over.
        if let (Some(since), Some(source)) = (dropped, from) {
            let idle = fetching.is_none() && waiting.is_empty();
            let over = match done {
                Some(_) => idle && fetched.elapsed() > Duration::from_millis(150),
                None => since.elapsed() > AFTER_DROP,
            };
            if over {
                let taken = done.unwrap_or(false);
                message(
                    source,
                    finished,
                    [own, u32::from(taken), if taken { copy } else { 0 }, 0, 0],
                )?;
                dropped = None;
                done = None;
                from = None;
                if standing {
                    standing = false;
                    x.unmap_window(own)?;
                }
            }
        }
        x.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_of_data_is_asked_for_by_its_own_name_and_texts_by_the_older_ones() {
        assert_eq!(names_of("text/uri-list"), ["text/uri-list"]);
        assert_eq!(
            names_of("text/plain;charset=utf-8"),
            ["text/plain;charset=utf-8", "UTF8_STRING"]
        );
        assert_eq!(names_of("text/plain"), ["text/plain", "STRING", "TEXT"]);
    }

    #[test]
    fn what_an_x11_program_offers_is_offered_to_wayland_windows_by_their_names() {
        let names = [
            "TARGETS",
            "UTF8_STRING",
            "STRING",
            "text/uri-list",
            "TIMESTAMP",
        ]
        .map(String::from);
        let pair = |kind: &str, by: &str| (kind.to_string(), by.to_string());
        assert_eq!(
            offered(&names),
            [
                pair("text/uri-list", "text/uri-list"),
                pair("text/plain;charset=utf-8", "UTF8_STRING"),
                pair("text/plain", "STRING"),
            ]
        );
        // What is offered under its own name is fetched by that.
        let both = ["text/plain", "TEXT"].map(String::from);
        assert_eq!(offered(&both), [pair("text/plain", "text/plain")]);
        assert!(offered(&["TARGETS".to_string()]).is_empty());
    }
}
