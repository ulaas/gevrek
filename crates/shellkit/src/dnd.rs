//! Drag and drop between a surface and other programs: what both systems
//! have in common.
//!
//! A surface that asks for it ([`host::Control::accept_drops`]) is told of
//! what is dragged over it and dropped on it as [`DragEvent`]s, which its
//! drawing asks for ([`host::Control::drag_events`]), and says what it
//! would do with it ([`host::Control::answer_drag`]); the program the drag
//! comes from shows the pointer that goes with the answer. A surface can
//! also begin a drag of its own ([`host::Control::start_drag`]) and is
//! told how it ended ([`DragEvent::Ended`]). A surface that asks for none
//! of it is as it was.
//!
//! Nothing here calls a system: the kinds of data the two systems carry
//! files in (Windows' list of dropped files, the list of addresses of a
//! Wayland or X11 program), and the rules by which an action is chosen,
//! are written out so that they can be tested without a desktop. The
//! halves that do the talking are `win::dnd` (OLE) and `layer::dnd`
//! (`wl_data_device`).
//!
//! [`host::Control::accept_drops`]: crate::host::Control::accept_drops
//! [`host::Control::drag_events`]: crate::host::Control::drag_events
//! [`host::Control::answer_drag`]: crate::host::Control::answer_drag
//! [`host::Control::start_drag`]: crate::host::Control::start_drag

/// What may be done with what is dragged. (The same three as the zones'
/// own `DragActions`, field for field.)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DragActions {
    pub copy: bool,
    pub moving: bool,
    pub link: bool,
}

impl DragActions {
    pub const NONE: DragActions = DragActions {
        copy: false,
        moving: false,
        link: false,
    };
    pub const ALL: DragActions = DragActions {
        copy: true,
        moving: true,
        link: true,
    };
    pub const COPY: DragActions = DragActions {
        copy: true,
        moving: false,
        link: false,
    };
    pub const COPY_OR_MOVE: DragActions = DragActions {
        copy: true,
        moving: true,
        link: false,
    };

    pub fn allows(self, action: DragAction) -> bool {
        match action {
            DragAction::Copy => self.copy,
            DragAction::Move => self.moving,
            DragAction::Link => self.link,
        }
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    /// What both allow.
    pub fn both(self, other: DragActions) -> DragActions {
        DragActions {
            copy: self.copy && other.copy,
            moving: self.moving && other.moving,
            link: self.link && other.link,
        }
    }

    /// Only this one.
    pub fn only(action: DragAction) -> DragActions {
        DragActions {
            copy: action == DragAction::Copy,
            moving: action == DragAction::Move,
            link: action == DragAction::Link,
        }
    }

    /// As Windows counts them (`DROPEFFECT`): copy 1, move 2, link 4.
    pub fn effects(self) -> u32 {
        u32::from(self.copy) | u32::from(self.moving) << 1 | u32::from(self.link) << 2
    }

    /// From Windows' count; what else is in it (scrolling) is no action.
    pub fn from_effects(effects: u32) -> DragActions {
        DragActions {
            copy: effects & 1 != 0,
            moving: effects & 2 != 0,
            link: effects & 4 != 0,
        }
    }

    /// As a Wayland compositor counts them: copy 1, move 2. It knows no
    /// link (its third is "ask", which no surface of the shell does), so a
    /// link is left out.
    pub fn wayland(self) -> u32 {
        u32::from(self.copy) | u32::from(self.moving) << 1
    }

    pub fn from_wayland(actions: u32) -> DragActions {
        DragActions {
            copy: actions & 1 != 0,
            moving: actions & 2 != 0,
            link: false,
        }
    }
}

/// The one thing that is done with what was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DragAction {
    Copy,
    Move,
    Link,
}

impl DragAction {
    /// As Windows counts it (see [`DragActions::effects`]).
    pub fn effect(self) -> u32 {
        DragActions::only(self).effects()
    }

    /// The action of an effect that names exactly one; none otherwise.
    pub fn from_effect(effect: u32) -> Option<DragAction> {
        match effect & 7 {
            1 => Some(DragAction::Copy),
            2 => Some(DragAction::Move),
            4 => Some(DragAction::Link),
            _ => None,
        }
    }
}

/// The keys held while something is dragged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Held {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Held {
    /// From what Windows says with every move of a drag (`MK_SHIFT` 4,
    /// `MK_CONTROL` 8, `MK_ALT` 32).
    pub fn from_key_state(keys: u32) -> Held {
        Held {
            shift: keys & 0x04 != 0,
            ctrl: keys & 0x08 != 0,
            alt: keys & 0x20 != 0,
        }
    }

    /// What the keys ask for, as on every desktop: Ctrl copies, Shift
    /// moves, both together (or Alt) make a link. No key asks for nothing.
    pub fn wanted(self) -> Option<DragAction> {
        match (self.ctrl, self.shift, self.alt) {
            (true, true, _) | (false, false, true) => Some(DragAction::Link),
            (true, false, _) => Some(DragAction::Copy),
            (false, true, _) => Some(DragAction::Move),
            (false, false, false) => None,
        }
    }
}

/// What a surface would do with what is over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropAnswer {
    /// What it would take at all here; none: nothing can be dropped here.
    pub accept: DragActions,
    /// What it does when the keys ask for nothing (a zone moves within a
    /// disk and copies between two). Without one: copy before move before
    /// link, the one that loses nothing first.
    pub prefer: Option<DragAction>,
}

impl DropAnswer {
    /// Nothing can be dropped.
    pub const REFUSE: DropAnswer = DropAnswer {
        accept: DragActions::NONE,
        prefer: None,
    };

    pub fn accept(accept: DragActions) -> DropAnswer {
        DropAnswer {
            accept,
            prefer: None,
        }
    }

    pub fn preferring(accept: DragActions, prefer: DragAction) -> DropAnswer {
        DropAnswer {
            accept,
            prefer: Some(prefer),
        }
    }
}

/// The action that is taken, of what the source offers, what the keys ask
/// for and what the surface would do; none: no drop (the pointer says so).
///
/// A key that asks for something neither side can do does not fall back
/// on something else: whoever holds Ctrl to copy and sees a move happen
/// has lost a file's place. This is what Explorer does, too.
pub fn negotiate(offered: DragActions, held: Held, answer: DropAnswer) -> Option<DragAction> {
    let possible = offered.both(answer.accept);
    if let Some(wanted) = held.wanted() {
        return possible.allows(wanted).then_some(wanted);
    }
    answer
        .prefer
        .into_iter()
        .chain([DragAction::Copy, DragAction::Move, DragAction::Link])
        .find(|action| possible.allows(*action))
}

/// What a drag from a surface of ours does next, as Windows asks at every
/// change of a key or a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continue {
    On,
    Drop,
    Cancel,
}

/// The buttons in what Windows says of the keys (`MK_LBUTTON` 1,
/// `MK_RBUTTON` 2, `MK_MBUTTON` 16).
pub const BUTTONS: u32 = 0x01 | 0x02 | 0x10;

/// `began`: the buttons that were down when the drag began. Escape ends
/// it, and so does a second button (as everywhere on Windows); it is
/// dropped when the button that began it is let go.
pub fn continue_drag(escape: bool, keys: u32, began: u32) -> Continue {
    let down = keys & BUTTONS;
    let began = began & BUTTONS;
    if escape || down & !began != 0 {
        Continue::Cancel
    } else if down & began == 0 {
        Continue::Drop
    } else {
        Continue::On
    }
}

/// Data of a kind only one system knows, carried as it came: the names a
/// Windows folder has for things that are no files (the recycle bin, a
/// printer), which only the system's own calls can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeData {
    /// The system's name of the kind (`Shell IDList Array`).
    pub kind: String,
    pub bytes: Vec<u8>,
}

/// What is carried by a drag. (The zones' `DropPayload` is made of this
/// field by field: `paths`, `text`, `allowed`.)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DragPayload {
    /// Files and folders of this machine, as paths.
    pub paths: Vec<String>,
    /// What has an address and is no file here (a link out of a browser,
    /// a file on another machine).
    pub uris: Vec<String>,
    /// Text, when that is what is dragged.
    pub text: Option<String>,
    /// What only the system itself can read (see [`NativeData`]).
    pub native: Vec<NativeData>,
    /// What the source lets be done with it.
    pub allowed: DragActions,
}

impl DragPayload {
    /// Files, to be copied or moved as the target likes.
    pub fn files(paths: impl IntoIterator<Item = impl Into<String>>) -> DragPayload {
        DragPayload {
            paths: paths.into_iter().map(Into::into).collect(),
            allowed: DragActions::ALL,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
            && self.uris.is_empty()
            && self.text.is_none()
            && self.native.is_empty()
    }
}

/// What a drag that came over a surface offers, as far as it is known
/// before the drop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offered {
    /// The kinds of data, by the system's names for them (`text/uri-list`
    /// on Linux, `CF_HDROP` or a registered name on Windows).
    pub kinds: Vec<String>,
    /// Files are among it.
    pub files: bool,
    /// What the source lets be done with it.
    pub allowed: DragActions,
    /// What is carried, where it can be read while the drag is still
    /// under way: on Windows. A Wayland program hands its data over
    /// through a pipe, which is asked for at the drop.
    pub payload: Option<DragPayload>,
}

/// What happens to a surface that accepts drops, or began a drag. Places
/// are in the surface's own points, as egui's.
#[derive(Debug, Clone, PartialEq)]
pub enum DragEvent {
    /// Something dragged came over the surface.
    Entered {
        at: egui::Pos2,
        offered: Offered,
    },
    Moved {
        at: egui::Pos2,
    },
    /// It went away, or the drag was given up.
    Left,
    /// It was let go here. `action` is what the source was told would be
    /// done, and is now for the surface to do: a move is the surface's to
    /// carry out in full (the source is told to delete nothing).
    Dropped {
        at: egui::Pos2,
        payload: DragPayload,
        action: DragAction,
    },
    /// A drag this surface began is over.
    Ended {
        outcome: DragOutcome,
    },
}

/// How a drag out of a surface ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    /// Taken somewhere. `remove`: the target copied what it was given and
    /// leaves it to the source to take the originals away (a move as some
    /// Windows programs do it); otherwise the target did all of it.
    Taken { action: DragAction, remove: bool },
    /// Escape, a drop where nothing takes it, or a target that refused.
    Cancelled,
}

/// How a drag ended, from what Windows and the target say of it:
/// `dropped` (and not given up), the effect the target answered the drop
/// with, and the two notes a target may leave in what was dragged: what
/// it did as far as the source is concerned ("performed"), and what it
/// was to the person ("logical performed").
///
/// A target that moves files itself answers "nothing" and notes a move
/// that was "logical": Explorer does. A target that answers "move" has
/// copied and leaves the originals to the source, unless it notes that
/// nothing is left to do.
pub fn outcome(
    dropped: bool,
    effect: u32,
    performed: Option<u32>,
    logical: Option<u32>,
) -> DragOutcome {
    if !dropped {
        return DragOutcome::Cancelled;
    }
    let taken = |action, remove| DragOutcome::Taken { action, remove };
    match DragAction::from_effect(effect) {
        Some(DragAction::Move) => taken(DragAction::Move, performed != Some(0)),
        Some(action) => taken(action, false),
        None => match (
            performed.and_then(DragAction::from_effect),
            logical.and_then(DragAction::from_effect),
        ) {
            (Some(DragAction::Move), _) => taken(DragAction::Move, true),
            (_, Some(action)) | (Some(action), None) => taken(action, false),
            (None, None) => DragOutcome::Cancelled,
        },
    }
}

/// The picture that follows the pointer while something is dragged.
#[derive(Debug, Clone, PartialEq)]
pub struct DragImage {
    /// In pixels.
    pub width: u32,
    pub height: u32,
    /// Four bytes a pixel, red first, row after row from the top; colours
    /// as they are, not multiplied by how solid they are.
    pub rgba: Vec<u8>,
    /// The pixel of the picture that is under the pointer's tip.
    pub hotspot: (i32, i32),
    /// Pixels to a logical pixel the picture was made for (the scale of
    /// the display the drag begins on).
    pub scale: f32,
}

impl DragImage {
    /// Whether the bytes are as many as the size says.
    pub fn is_sound(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.rgba.len() == self.width as usize * self.height as usize * 4
    }

    /// Its size in logical pixels, at least one each way.
    pub fn logical_size(&self) -> (i32, i32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            ((self.width as f32 / scale).round() as i32).max(1),
            ((self.height as f32 / scale).round() as i32).max(1),
        )
    }

    /// The hotspot in logical pixels.
    pub fn logical_hotspot(&self) -> (i32, i32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (self.hotspot.0 as f32 / scale).round() as i32,
            (self.hotspot.1 as f32 / scale).round() as i32,
        )
    }
}

/// A drag to begin.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DragStart {
    /// What is dragged and what may be done with it.
    pub payload: DragPayload,
    pub image: Option<DragImage>,
}

/// A picture as both systems want it for what follows the pointer: blue
/// first, and every colour multiplied by how solid the pixel is.
pub fn premultiplied_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        let alpha = u32::from(pixel[3]);
        // (Rounded, so that a solid pixel keeps its colour exactly.)
        let scaled = |colour: u8| ((u32::from(colour) * alpha + 127) / 255) as u8;
        out.extend_from_slice(&[
            scaled(pixel[2]),
            scaled(pixel[1]),
            scaled(pixel[0]),
            pixel[3],
        ]);
    }
    out
}

/// A place given in the pixels of a surface, in the surface's points.
pub fn points(x: i32, y: i32, scale: f32) -> egui::Pos2 {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    egui::pos2(x as f32 / scale, y as f32 / scale)
}

/// How paths are written on a system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// `/home/ada/a.txt`
    Unix,
    /// `C:\Users\ada\a.txt`, `\\server\share\a.txt`
    Windows,
}

impl PathStyle {
    /// Of the system the program is built for.
    #[cfg(windows)]
    pub const HERE: PathStyle = PathStyle::Windows;
    #[cfg(not(windows))]
    pub const HERE: PathStyle = PathStyle::Unix;
}

/// Addresses of files (`file://`), and lists of them (`text/uri-list`):
/// how Wayland and X11 programs say which files are dragged.
pub mod uri {
    use super::PathStyle;

    /// What stands for itself in an address; every other byte is written
    /// as `%` and two hexadecimal digits. (Fewer than the rules allow:
    /// what is escaped without need is read back the same by everyone, and
    /// what is left bare against a reader's expectations is not.)
    fn stands(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
    }

    fn escape(text: &str, out: &mut String) {
        for byte in text.bytes() {
            if stands(byte) {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }

    /// `%41` read back as `A`. Nothing where the result is no text (a
    /// file name in an encoding of long ago): a wrong path is worse than
    /// none.
    pub fn unescape(text: &str) -> Option<String> {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%' {
                let pair = bytes.get(at + 1..at + 3)?;
                let digit = |byte: u8| char::from(byte).to_digit(16);
                out.push((digit(pair[0])? * 16 + digit(pair[1])?) as u8);
                at += 3;
            } else {
                out.push(bytes[at]);
                at += 1;
            }
        }
        String::from_utf8(out).ok()
    }

    /// The address of a file. Nothing for what is no full path.
    pub fn of_path(path: &str, style: PathStyle) -> Option<String> {
        let mut out = String::from("file://");
        match style {
            PathStyle::Unix => {
                if !path.starts_with('/') {
                    return None;
                }
                escape(path, &mut out);
            }
            PathStyle::Windows => {
                // (A path that says "as it is written" says nothing more
                // to anyone else: `\\?\C:\a`, `\\?\UNC\server\share\a`.)
                let path = match path.strip_prefix(r"\\?\UNC\") {
                    Some(rest) => format!(r"\\{rest}"),
                    None => path.strip_prefix(r"\\?\").unwrap_or(path).to_string(),
                };
                let forward = path.replace('\\', "/");
                if let Some(shared) = forward.strip_prefix("//") {
                    // Another machine's: its name is the address's host.
                    if shared.is_empty() || shared.starts_with('/') {
                        return None;
                    }
                    escape(shared, &mut out);
                } else {
                    let mut letters = forward.chars();
                    let drive = letters.next().filter(char::is_ascii_alphabetic)?;
                    if letters.next() != Some(':') || letters.next() != Some('/') {
                        return None;
                    }
                    out.push('/');
                    out.push(drive);
                    out.push(':');
                    escape(&forward[2..], &mut out);
                }
            }
        }
        Some(out)
    }

    /// The path of a file's address, when it is a file of this machine
    /// (`local_host`: its name, where known; `localhost` and no name are
    /// always this machine). On Windows another machine's file has a path
    /// too (`\\server\share\a.txt`); on Linux it has none.
    pub fn to_path(uri: &str, style: PathStyle, local_host: Option<&str>) -> Option<String> {
        let (scheme, rest) = uri.split_once(':')?;
        if !scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        // What comes after the path is not part of it.
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        // `file:/a` (no host at all), `file:///a`, `file://host/a`.
        let (host, path) = match rest.strip_prefix("//") {
            Some(after) => {
                let slash = after.find('/')?;
                (&after[..slash], &after[slash..])
            }
            None => ("", rest),
        };
        if !path.starts_with('/') {
            return None;
        }
        let here = host.is_empty()
            || host.eq_ignore_ascii_case("localhost")
            || local_host.is_some_and(|name| host.eq_ignore_ascii_case(name));
        let path = unescape(path)?;
        if path.contains('\0') {
            return None;
        }
        match style {
            PathStyle::Unix => here.then_some(path),
            PathStyle::Windows => {
                let back = path.replace('/', "\\");
                if !here {
                    return Some(format!(r"\\{}{back}", unescape(host)?));
                }
                // `/C:/a` is `C:\a`; `//server/share` after an empty host
                // is the other way some programs write a shared folder.
                let drive = back.as_bytes();
                if drive.len() >= 3 && drive[1].is_ascii_alphabetic() && drive[2] == b':' {
                    Some(back[1..].to_string())
                } else if back.starts_with(r"\\") {
                    Some(back)
                } else {
                    None
                }
            }
        }
    }

    /// A list of addresses as it is sent: one to a line, every line ended
    /// the way the rules of the Internet end lines (`\r\n`).
    pub fn list(uris: &[String]) -> String {
        let mut out = String::new();
        for uri in uris {
            out.push_str(uri);
            out.push_str("\r\n");
        }
        out
    }

    /// The addresses of a list as it was received: lines ended either
    /// way, empty ones and remarks (`#`) left out, and nothing after a
    /// zero byte (some programs send the end of their text along).
    pub fn parse_list(bytes: &[u8]) -> Vec<String> {
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end])
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect()
    }
}

/// The kinds of data a Wayland or X11 program carries files and text in,
/// and what is sent and read in each.
pub mod mime {
    use super::{uri, DragPayload, PathStyle};

    pub const URI_LIST: &str = "text/uri-list";
    /// What GNOME's file manager (and those made like it) reads first.
    pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
    pub const TEXT_UTF8: &str = "text/plain;charset=utf-8";
    pub const TEXT: &str = "text/plain";

    /// Every address a payload has: its files' first, then the others.
    fn uris(payload: &DragPayload, style: PathStyle) -> Vec<String> {
        payload
            .paths
            .iter()
            .filter_map(|path| uri::of_path(path, style))
            .chain(payload.uris.iter().cloned())
            .collect()
    }

    /// The kinds a drag of this payload is offered in, the most telling
    /// first: a target takes the first it knows.
    pub fn kinds(payload: &DragPayload, style: PathStyle) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        let addresses = !uris(payload, style).is_empty();
        if addresses {
            kinds.push(URI_LIST);
            if !payload.paths.is_empty() {
                kinds.push(GNOME_FILES);
            }
        }
        if addresses || payload.text.is_some() {
            kinds.push(TEXT_UTF8);
            kinds.push(TEXT);
        }
        kinds
    }

    /// What is sent to a target that asks for one of the kinds; nothing
    /// for a kind that was not offered.
    pub fn bytes(payload: &DragPayload, kind: &str, style: PathStyle) -> Option<Vec<u8>> {
        let addresses = uris(payload, style);
        match kind {
            URI_LIST if !addresses.is_empty() => Some(uri::list(&addresses).into_bytes()),
            // What is to be done with them, then an address to a line.
            GNOME_FILES if !payload.paths.is_empty() => {
                Some(format!("copy\n{}", addresses.join("\n")).into_bytes())
            }
            TEXT_UTF8 | TEXT => match &payload.text {
                Some(text) => Some(text.clone().into_bytes()),
                // Dropped into a text: the files' paths, as a file
                // manager gives them.
                None if !addresses.is_empty() => Some(
                    payload
                        .paths
                        .iter()
                        .chain(payload.uris.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into_bytes(),
                ),
                None => None,
            },
            _ => None,
        }
    }

    /// The kind to ask a source for, of those it offers: files before
    /// text.
    pub fn pick(offered: &[String]) -> Option<&'static str> {
        [URI_LIST, TEXT_UTF8, TEXT, "UTF8_STRING", "STRING"]
            .into_iter()
            .find(|kind| offered.iter().any(|known| known == kind))
    }

    /// Whether files can be among what is offered.
    pub fn has_files(offered: &[String]) -> bool {
        offered.iter().any(|kind| kind == URI_LIST)
    }

    /// What a source sent in a kind, taken into a payload: the addresses
    /// of files of this machine as paths, the others as they are.
    pub fn read(
        payload: &mut DragPayload,
        kind: &str,
        bytes: &[u8],
        style: PathStyle,
        local_host: Option<&str>,
    ) {
        if kind == URI_LIST {
            for address in uri::parse_list(bytes) {
                match uri::to_path(&address, style, local_host) {
                    Some(path) => payload.paths.push(path),
                    None => payload.uris.push(address),
                }
            }
        } else {
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
            if !text.is_empty() {
                payload.text = Some(text);
            }
        }
    }
}

/// Windows' list of dropped files (`CF_HDROP`): a short head, then every
/// path with a zero after it, and one more zero after the last.
pub mod hdrop {
    /// The head: where the paths begin (4 bytes), a point nobody reads
    /// (8), whether that point is outside the window's inside (4), and
    /// whether the paths are in Windows' wide letters (4).
    pub const HEAD: usize = 20;

    /// The list for these paths, in wide letters. A path with a zero in
    /// it would end the list early and is left out.
    pub fn encode(paths: &[String]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEAD + 2);
        out.extend_from_slice(&(HEAD as u32).to_le_bytes());
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&1u32.to_le_bytes());
        for path in paths
            .iter()
            .filter(|path| !path.is_empty() && !path.contains('\0'))
        {
            for unit in path.encode_utf16().chain(std::iter::once(0)) {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// Whether a list is in wide letters; nothing for what is no list.
    pub fn is_wide(bytes: &[u8]) -> Option<bool> {
        let flag = bytes.get(16..20)?;
        Some(u32::from_le_bytes([flag[0], flag[1], flag[2], flag[3]]) != 0)
    }

    /// The paths of a list in wide letters. Nothing for one that is cut
    /// short, or in the letters of the machine's older programs (those
    /// only the system reads right: `DragQueryFileW`). A list whose last
    /// zero is missing still gives the paths that are whole.
    pub fn decode(bytes: &[u8]) -> Option<Vec<String>> {
        if !is_wide(bytes)? {
            return None;
        }
        let begin = bytes.get(0..4)?;
        let begin = u32::from_le_bytes([begin[0], begin[1], begin[2], begin[3]]) as usize;
        let body = bytes.get(begin..)?;
        let mut paths = Vec::new();
        let mut path: Vec<u16> = Vec::new();
        for pair in body.as_chunks::<2>().0 {
            let unit = u16::from_le_bytes(*pair);
            if unit != 0 {
                path.push(unit);
            } else if path.is_empty() {
                // The second zero in a row: the end.
                break;
            } else {
                paths.push(String::from_utf16_lossy(&path));
                path.clear();
            }
        }
        Some(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFERS: [DragActions; 4] = [
        DragActions::NONE,
        DragActions::COPY,
        DragActions::COPY_OR_MOVE,
        DragActions::ALL,
    ];

    fn held(shift: bool, ctrl: bool, alt: bool) -> Held {
        Held { shift, ctrl, alt }
    }

    #[test]
    fn the_keys_ask_as_on_every_desktop() {
        assert_eq!(held(false, false, false).wanted(), None);
        assert_eq!(held(false, true, false).wanted(), Some(DragAction::Copy));
        assert_eq!(held(true, false, false).wanted(), Some(DragAction::Move));
        assert_eq!(held(true, true, false).wanted(), Some(DragAction::Link));
        assert_eq!(held(false, false, true).wanted(), Some(DragAction::Link));
        // With Alt as well, Ctrl and Shift still say what they say.
        assert_eq!(held(false, true, true).wanted(), Some(DragAction::Copy));
        assert_eq!(held(true, false, true).wanted(), Some(DragAction::Move));
        // As Windows tells them: Shift 4, Ctrl 8, Alt 32; buttons are none.
        assert_eq!(Held::from_key_state(0x01 | 0x08), held(false, true, false));
        assert_eq!(Held::from_key_state(0x04 | 0x20), held(true, false, true));
        assert_eq!(Held::from_key_state(0x13), Held::default());
    }

    #[test]
    fn what_is_done_is_what_source_keys_and_surface_all_allow() {
        let none = Held::default();
        let (shift, ctrl, both) = (
            held(true, false, false),
            held(false, true, false),
            held(true, true, false),
        );
        let all = DropAnswer::accept(DragActions::ALL);
        // offered, keys, answer -> the action
        let table = [
            // No key: the one that loses nothing, unless the surface
            // prefers another.
            (DragActions::ALL, none, all, Some(DragAction::Copy)),
            (
                DragActions::ALL,
                none,
                DropAnswer::preferring(DragActions::ALL, DragAction::Move),
                Some(DragAction::Move),
            ),
            // What is preferred but not offered is not done.
            (
                DragActions::COPY,
                none,
                DropAnswer::preferring(DragActions::ALL, DragAction::Move),
                Some(DragAction::Copy),
            ),
            (
                DragActions {
                    copy: false,
                    moving: true,
                    link: true,
                },
                none,
                all,
                Some(DragAction::Move),
            ),
            (
                DragActions::only(DragAction::Link),
                none,
                all,
                Some(DragAction::Link),
            ),
            // Keys.
            (DragActions::ALL, ctrl, all, Some(DragAction::Copy)),
            (DragActions::ALL, shift, all, Some(DragAction::Move)),
            (DragActions::ALL, both, all, Some(DragAction::Link)),
            (
                DragActions::ALL,
                held(false, false, true),
                all,
                Some(DragAction::Link),
            ),
            // A key that asks for what cannot be done gets nothing, not
            // something else.
            (DragActions::COPY, shift, all, None),
            (DragActions::COPY_OR_MOVE, both, all, None),
            (
                DragActions::ALL,
                shift,
                DropAnswer::accept(DragActions::COPY),
                None,
            ),
            (
                DragActions::ALL,
                ctrl,
                DropAnswer::preferring(DragActions::COPY_OR_MOVE, DragAction::Move),
                Some(DragAction::Copy),
            ),
            // Nothing in common, or a surface that refuses.
            (
                DragActions::COPY,
                none,
                DropAnswer::accept(DragActions::only(DragAction::Move)),
                None,
            ),
            (DragActions::ALL, none, DropAnswer::REFUSE, None),
            (DragActions::ALL, ctrl, DropAnswer::REFUSE, None),
            (DragActions::NONE, none, all, None),
        ];
        for (offered, keys, answer, wanted) in table {
            assert_eq!(
                negotiate(offered, keys, answer),
                wanted,
                "{offered:?} {keys:?} {answer:?}"
            );
        }
        // Whatever is asked: never an action one side did not allow.
        for offered in OFFERS {
            for accept in OFFERS {
                for bits in 0..8u8 {
                    let keys = held(bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                    for prefer in [
                        None,
                        Some(DragAction::Copy),
                        Some(DragAction::Move),
                        Some(DragAction::Link),
                    ] {
                        let answer = DropAnswer { accept, prefer };
                        if let Some(action) = negotiate(offered, keys, answer) {
                            assert!(offered.allows(action) && accept.allows(action));
                            if let Some(wanted) = keys.wanted() {
                                assert_eq!(action, wanted);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn actions_are_counted_as_each_system_counts_them() {
        assert_eq!(DragActions::ALL.effects(), 7);
        assert_eq!(DragActions::COPY_OR_MOVE.effects(), 3);
        assert_eq!(DragActions::from_effects(7), DragActions::ALL);
        // (Scrolling is in Windows' count and is no action.)
        assert_eq!(
            DragActions::from_effects(0x8000_0002),
            DragActions::only(DragAction::Move)
        );
        for actions in OFFERS {
            assert_eq!(DragActions::from_effects(actions.effects()), actions);
        }
        assert_eq!(DragAction::from_effect(4), Some(DragAction::Link));
        assert_eq!(DragAction::from_effect(0), None);
        assert_eq!(DragAction::from_effect(3), None);
        assert_eq!(DragAction::Move.effect(), 2);
        // A Wayland compositor knows no link; its 4 is "ask".
        assert_eq!(DragActions::ALL.wayland(), 3);
        assert_eq!(DragActions::from_wayland(7), DragActions::COPY_OR_MOVE);
        assert_eq!(DragActions::only(DragAction::Link).wayland(), 0);
    }

    #[test]
    fn a_drag_of_ours_goes_on_until_its_button_is_let_go() {
        let (left, right) = (0x01, 0x02);
        assert_eq!(continue_drag(false, left, left), Continue::On);
        assert_eq!(continue_drag(false, left | 0x08, left), Continue::On);
        assert_eq!(continue_drag(false, 0, left), Continue::Drop);
        assert_eq!(continue_drag(false, 0x08, left), Continue::Drop);
        assert_eq!(continue_drag(true, left, left), Continue::Cancel);
        // A second button gives it up.
        assert_eq!(continue_drag(false, left | right, left), Continue::Cancel);
        assert_eq!(continue_drag(false, right, right), Continue::On);
    }

    #[test]
    fn how_a_drag_ended_is_read_from_what_the_target_left() {
        let taken = |action, remove| DragOutcome::Taken { action, remove };
        // Given up, whatever else is said.
        assert_eq!(outcome(false, 1, Some(2), Some(2)), DragOutcome::Cancelled);
        // Dropped where nothing took it.
        assert_eq!(outcome(true, 0, None, None), DragOutcome::Cancelled);
        assert_eq!(outcome(true, 0, Some(0), Some(0)), DragOutcome::Cancelled);
        assert_eq!(outcome(true, 1, None, None), taken(DragAction::Copy, false));
        assert_eq!(outcome(true, 4, None, None), taken(DragAction::Link, false));
        // A move the target left half done: the originals are the source's.
        assert_eq!(outcome(true, 2, None, None), taken(DragAction::Move, true));
        assert_eq!(
            outcome(true, 2, Some(2), Some(2)),
            taken(DragAction::Move, true)
        );
        assert_eq!(
            outcome(true, 0, Some(2), None),
            taken(DragAction::Move, true)
        );
        // A move the target did itself.
        assert_eq!(
            outcome(true, 2, Some(0), Some(2)),
            taken(DragAction::Move, false)
        );
        assert_eq!(
            outcome(true, 0, Some(0), Some(2)),
            taken(DragAction::Move, false)
        );
        assert_eq!(
            outcome(true, 0, None, Some(2)),
            taken(DragAction::Move, false)
        );
        assert_eq!(
            outcome(true, 0, Some(1), None),
            taken(DragAction::Copy, false)
        );
        assert_eq!(
            outcome(true, 0, None, Some(1)),
            taken(DragAction::Copy, false)
        );
    }

    #[test]
    fn a_place_in_pixels_is_a_place_in_points_at_every_scale() {
        assert_eq!(points(100, 50, 1.0), egui::pos2(100.0, 50.0));
        assert_eq!(points(100, 50, 2.0), egui::pos2(50.0, 25.0));
        assert_eq!(points(150, 75, 1.5), egui::pos2(100.0, 50.0));
        assert_eq!(points(125, -25, 1.25), egui::pos2(100.0, -20.0));
        let at = points(301, 7, 1.75);
        assert!((at.x - 172.0).abs() < 1e-4 && (at.y - 4.0).abs() < 1e-4);
        // (A scale nobody told is one.)
        assert_eq!(points(3, 4, 0.0), egui::pos2(3.0, 4.0));
    }

    #[test]
    fn a_drag_picture_is_measured_in_logical_pixels() {
        let image = DragImage {
            width: 96,
            height: 48,
            rgba: vec![0; 96 * 48 * 4],
            hotspot: (12, 6),
            scale: 1.5,
        };
        assert!(image.is_sound());
        assert_eq!(image.logical_size(), (64, 32));
        assert_eq!(image.logical_hotspot(), (8, 4));
        let short = DragImage {
            rgba: vec![0; 5],
            ..image
        };
        assert!(!short.is_sound());
    }

    #[test]
    fn a_picture_is_turned_blue_first_and_multiplied() {
        let rgba = [255, 128, 0, 255, 255, 128, 0, 128, 10, 20, 30, 0];
        assert_eq!(
            premultiplied_bgra(&rgba),
            [0, 128, 255, 255, 0, 64, 128, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_path_and_its_address_are_each_others() {
        let cases = [
            ("/home/ada/a.txt", "file:///home/ada/a.txt"),
            (
                "/home/ada/my file (1).txt",
                "file:///home/ada/my%20file%20%281%29.txt",
            ),
            (
                "/tmp/çay & şeker.txt",
                "file:///tmp/%C3%A7ay%20%26%20%C5%9Feker.txt",
            ),
            ("/tmp/100%#?.txt", "file:///tmp/100%25%23%3F.txt"),
            ("/tmp/日本語", "file:///tmp/%E6%97%A5%E6%9C%AC%E8%AA%9E"),
            ("/", "file:///"),
        ];
        for (path, address) in cases {
            assert_eq!(
                uri::of_path(path, PathStyle::Unix).as_deref(),
                Some(address)
            );
            assert_eq!(
                uri::to_path(address, PathStyle::Unix, None).as_deref(),
                Some(path)
            );
        }
        assert_eq!(uri::of_path("a.txt", PathStyle::Unix), None);
        let cases = [
            (r"C:\Users\ada\a.txt", "file:///C:/Users/ada/a.txt"),
            (
                r"D:\Çay Bahçesi\ö.txt",
                "file:///D:/%C3%87ay%20Bah%C3%A7esi/%C3%B6.txt",
            ),
            (r"\\server\share\a b.txt", "file://server/share/a%20b.txt"),
        ];
        for (path, address) in cases {
            assert_eq!(
                uri::of_path(path, PathStyle::Windows).as_deref(),
                Some(address)
            );
            assert_eq!(
                uri::to_path(address, PathStyle::Windows, None).as_deref(),
                Some(path)
            );
        }
        // Paths written "as they are" are addressed like any other.
        assert_eq!(
            uri::of_path(r"\\?\C:\a\b.txt", PathStyle::Windows).as_deref(),
            Some("file:///C:/a/b.txt")
        );
        assert_eq!(
            uri::of_path(r"\\?\UNC\server\share\b.txt", PathStyle::Windows).as_deref(),
            Some("file://server/share/b.txt")
        );
        assert_eq!(uri::of_path(r"a\b.txt", PathStyle::Windows), None);
        assert_eq!(uri::of_path(r"\a\b.txt", PathStyle::Windows), None);
    }

    #[test]
    fn an_address_is_a_path_only_for_a_file_of_this_machine() {
        let unix = |address, host| uri::to_path(address, PathStyle::Unix, host);
        assert_eq!(unix("file://localhost/a/b", None).as_deref(), Some("/a/b"));
        assert_eq!(unix("FILE://LocalHost/a/b", None).as_deref(), Some("/a/b"));
        assert_eq!(
            unix("file://lovelace/a/b", Some("Lovelace")).as_deref(),
            Some("/a/b")
        );
        assert_eq!(unix("file://babbage/a/b", Some("lovelace")), None);
        assert_eq!(unix("file://babbage/a/b", None), None);
        // One slash, as some programs write it.
        assert_eq!(unix("file:/a/b%20c", None).as_deref(), Some("/a/b c"));
        // Small and capital digits, and what is after the path.
        assert_eq!(
            unix("file:///a/%c3%A7?x=1#top", None).as_deref(),
            Some("/a/ç")
        );
        // No file, no path, or no text.
        assert_eq!(unix("https://example.org/a", None), None);
        assert_eq!(unix("file://host", None), None);
        assert_eq!(unix("file:a/b", None), None);
        assert_eq!(unix("file:///a/%ZZ", None), None);
        assert_eq!(unix("file:///a/%4", None), None);
        assert_eq!(unix("file:///a/%FF", None), None);
        assert_eq!(unix("file:///a/%00b", None), None);
        let windows = |address| uri::to_path(address, PathStyle::Windows, None);
        assert_eq!(windows("file://localhost/C:/a").as_deref(), Some(r"C:\a"));
        assert_eq!(windows("file:///c:/a%20b/").as_deref(), Some(r"c:\a b\"));
        assert_eq!(
            windows("file:////server/share/a").as_deref(),
            Some(r"\\server\share\a")
        );
        assert_eq!(windows("file:///a/b"), None);
    }

    #[test]
    fn a_list_of_addresses_is_read_however_its_lines_end() {
        let sent = uri::list(&["file:///a".to_string(), "file:///b%20c".to_string()]);
        assert_eq!(sent, "file:///a\r\nfile:///b%20c\r\n");
        assert_eq!(
            uri::parse_list(sent.as_bytes()),
            ["file:///a", "file:///b%20c"]
        );
        let received = b"# from a file manager\r\nfile:///a\r\n\r\n  file:///b \nhttps://example.org/\r\n\0junk";
        assert_eq!(
            uri::parse_list(received),
            ["file:///a", "file:///b", "https://example.org/"]
        );
        assert!(uri::parse_list(b"").is_empty());
        assert!(uri::parse_list(b"\r\n#only\r\n").is_empty());
    }

    #[test]
    fn files_are_offered_in_the_kinds_file_managers_read() {
        let payload = DragPayload::files(["/home/ada/a b.txt", "/home/ada/ç"]);
        assert_eq!(
            mime::kinds(&payload, PathStyle::Unix),
            [
                mime::URI_LIST,
                mime::GNOME_FILES,
                mime::TEXT_UTF8,
                mime::TEXT
            ]
        );
        let bytes = |kind| {
            String::from_utf8(mime::bytes(&payload, kind, PathStyle::Unix).unwrap()).unwrap()
        };
        assert_eq!(
            bytes(mime::URI_LIST),
            "file:///home/ada/a%20b.txt\r\nfile:///home/ada/%C3%A7\r\n"
        );
        assert_eq!(
            bytes(mime::GNOME_FILES),
            "copy\nfile:///home/ada/a%20b.txt\nfile:///home/ada/%C3%A7"
        );
        assert_eq!(bytes(mime::TEXT), "/home/ada/a b.txt\n/home/ada/ç");
        assert_eq!(mime::bytes(&payload, "image/png", PathStyle::Unix), None);
        // Text alone is offered as text alone.
        let text = DragPayload {
            text: Some("çay".to_string()),
            ..Default::default()
        };
        assert_eq!(
            mime::kinds(&text, PathStyle::Unix),
            [mime::TEXT_UTF8, mime::TEXT]
        );
        assert_eq!(
            mime::bytes(&text, mime::TEXT_UTF8, PathStyle::Unix).unwrap(),
            "çay".as_bytes()
        );
        assert_eq!(mime::bytes(&text, mime::URI_LIST, PathStyle::Unix), None);
        assert!(mime::kinds(&DragPayload::default(), PathStyle::Unix).is_empty());
    }

    #[test]
    fn what_a_source_sends_is_read_into_paths_addresses_and_text() {
        let offered = |kinds: &[&str]| {
            kinds
                .iter()
                .map(|kind| kind.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            mime::pick(&offered(&["text/plain", "text/uri-list", "image/png"])),
            Some(mime::URI_LIST)
        );
        assert_eq!(
            mime::pick(&offered(&["STRING", "text/plain"])),
            Some(mime::TEXT)
        );
        assert_eq!(mime::pick(&offered(&["image/png"])), None);
        assert!(mime::has_files(&offered(&["text/uri-list"])));
        assert!(!mime::has_files(&offered(&["text/plain"])));

        let mut payload = DragPayload::default();
        mime::read(
            &mut payload,
            mime::URI_LIST,
            b"file:///home/ada/a%20b.txt\r\nfile://babbage/x\r\nhttps://example.org/a\r\n",
            PathStyle::Unix,
            Some("lovelace"),
        );
        assert_eq!(payload.paths, ["/home/ada/a b.txt"]);
        assert_eq!(payload.uris, ["file://babbage/x", "https://example.org/a"]);
        assert_eq!(payload.text, None);
        let mut payload = DragPayload::default();
        mime::read(
            &mut payload,
            mime::TEXT_UTF8,
            "şeker\0".as_bytes(),
            PathStyle::Unix,
            None,
        );
        assert_eq!(payload.text.as_deref(), Some("şeker"));
        assert!(payload.paths.is_empty());
        // What was sent comes back as it was.
        let sent = DragPayload::files(["/a/b c", "/ç"]);
        let mut back = DragPayload::default();
        mime::read(
            &mut back,
            mime::URI_LIST,
            &mime::bytes(&sent, mime::URI_LIST, PathStyle::Unix).unwrap(),
            PathStyle::Unix,
            None,
        );
        assert_eq!(back.paths, sent.paths);
    }

    #[test]
    fn windows_list_of_files_is_written_and_read_back() {
        let paths = vec![
            r"C:\a.txt".to_string(),
            r"D:\Çay Bahçesi\日本語 𝄞.txt".to_string(),
        ];
        let bytes = hdrop::encode(&paths);
        // The head: the paths begin after it, and are in wide letters.
        assert_eq!(&bytes[0..4], &20u32.to_le_bytes());
        assert_eq!(&bytes[4..16], &[0; 12]);
        assert_eq!(&bytes[16..20], &1u32.to_le_bytes());
        // The first path, each letter two bytes, and its zero.
        let first: Vec<u8> = r"C:\a.txt"
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(&bytes[20..20 + first.len()], &first[..]);
        // Two zeros at the end: the last path's and the list's.
        assert_eq!(&bytes[bytes.len() - 4..], &[0; 4]);
        assert_eq!(hdrop::decode(&bytes), Some(paths));
        // A path longer than Windows' old limit is a path like any other.
        let long = format!(r"C:\{}\file.txt", "long-folder-name\\".repeat(30));
        assert!(long.len() > 400);
        let both = vec![long.clone(), format!(r"\\?\{long}")];
        assert_eq!(hdrop::decode(&hdrop::encode(&both)), Some(both));
        // No paths: a head and the list's zero.
        let empty = hdrop::encode(&[]);
        assert_eq!(empty.len(), hdrop::HEAD + 2);
        assert_eq!(hdrop::decode(&empty), Some(Vec::new()));
        // What would end the list early is left out.
        let odd = vec!["a\0b".to_string(), String::new(), r"C:\c".to_string()];
        assert_eq!(
            hdrop::decode(&hdrop::encode(&odd)),
            Some(vec![r"C:\c".to_string()])
        );
    }

    #[test]
    fn a_list_that_is_cut_short_or_not_wide_is_not_misread() {
        let whole = hdrop::encode(&[r"C:\a".to_string(), r"C:\b".to_string()]);
        // The last zeros missing: the paths that are whole.
        assert_eq!(
            hdrop::decode(&whole[..whole.len() - 4]),
            Some(vec![r"C:\a".to_string()])
        );
        assert_eq!(hdrop::decode(&whole[..10]), None);
        assert_eq!(hdrop::decode(&[]), None);
        // The paths begin where the head says, not where they usually do.
        let mut shifted = whole.clone();
        shifted[0..4].copy_from_slice(&24u32.to_le_bytes());
        shifted.splice(20..20, [0xAA; 4]);
        assert_eq!(
            hdrop::decode(&shifted),
            Some(vec![r"C:\a".to_string(), r"C:\b".to_string()])
        );
        let mut beyond = whole.clone();
        beyond[0..4].copy_from_slice(&4000u32.to_le_bytes());
        assert_eq!(hdrop::decode(&beyond), None);
        // In the older letters: said, and left to the system.
        let mut narrow = whole;
        narrow[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(hdrop::is_wide(&narrow), Some(false));
        assert_eq!(hdrop::decode(&narrow), None);
    }
}
