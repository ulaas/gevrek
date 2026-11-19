//! Files as addresses (`file:///home/ada/a%20b.txt`), and what a drag or
//! a copy carries between programs: `text/uri-list` (RFC 2483) and the
//! file managers' `x-special/gnome-copied-files`.
//!
//! An item's id is the address of its file. A name is bytes on Linux, not
//! text; the address carries every byte, so a file whose name is no valid
//! UTF-8 still has an id that leads back to it.
//!
//! The letters left as they are in an address are the ones GLib leaves
//! (`g_filename_to_uri`), which Qt agrees with for file names: the
//! thumbnail of a file is found by the MD5 of its address, so the address
//! has to be written the way the programs that made the thumbnail wrote
//! it.

use gevrek_zones::backend::{DragActions, DropPayload};
use gevrek_zones::ids::ItemId;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// A list of addresses, one to a line (RFC 2483).
pub const URI_LIST: &str = "text/uri-list";
/// What Nautilus, Nemo, Caja and Thunar put on the clipboard for files:
/// `copy` or `cut`, then one address to a line.
pub const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
/// Plain text, for a program that takes nothing else.
pub const TEXT: &str = "text/plain;charset=utf-8";

/// The kinds of data a drag out of a zone offers, the best first.
pub const OFFERED: [&str; 3] = [URI_LIST, GNOME_COPIED_FILES, TEXT];

/// Whether a byte stands for itself in the path of an address.
fn plain(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&byte)
}

/// Bytes with everything that does not stand for itself written as `%XX`.
pub fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if plain(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The bytes an escaped text stands for. `None` where a `%` is not
/// followed by two hexadecimal digits: such a text is not an address,
/// and guessing at it could name another file.
pub fn unescape(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let digits = text.get(at + 1..at + 3)?;
            if !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(digits, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    Some(out)
}

/// The address of a file. The path has to be absolute.
pub fn uri_of(path: &Path) -> String {
    format!("file://{}", escape(path.as_os_str().as_bytes()))
}

/// The file an address names: `file:///path`, `file://localhost/path` and
/// `file:/path`. `None` for an address of another kind, of another
/// machine, or one that is not written rightly.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..5)?;
    if !scheme.eq_ignore_ascii_case("file:") {
        return None;
    }
    let rest = &uri[5..];
    let path = match rest.strip_prefix("//") {
        Some(after) => {
            let slash = after.find('/')?;
            let host = &after[..slash];
            if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                return None;
            }
            &after[slash..]
        }
        None => rest,
    };
    // (What follows a `?` or `#` is not part of the file's name; a file
    // whose name has one in it has it escaped.)
    let path = path.split(['?', '#']).next().unwrap_or(path);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = unescape(path)?;
    if bytes.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

/// The id of the item a file is.
pub fn item_id(path: &Path) -> ItemId {
    ItemId::new(uri_of(path))
}

/// The file an item is.
pub fn path_of_item(item: &ItemId) -> Option<PathBuf> {
    path_of(item.as_str())
}

/// Files as `text/uri-list`: one address to a line, each line ended as
/// the RFC asks (CR LF).
pub fn uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|path| format!("{}\r\n", uri_of(path))).collect()
}

/// The addresses of a `text/uri-list`. Lines that begin with `#` are
/// comments; lines may end either way, since not every program writes CR
/// LF.
pub fn addresses(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_matches(['\r', ' ', '\t', '\0']))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Files as file managers put them on the clipboard: whether they were
/// cut, then one address to a line, with no line end after the last.
pub fn gnome_copied_files(paths: &[PathBuf], cut: bool) -> String {
    let mut lines = vec![if cut { "cut" } else { "copy" }.to_string()];
    lines.extend(paths.iter().map(|path| uri_of(path)));
    lines.join("\n")
}

/// What `x-special/gnome-copied-files` says: whether the files were cut,
/// and their addresses. `None` where the first line is neither word.
pub fn read_gnome_copied_files(text: &str) -> Option<(bool, Vec<String>)> {
    let mut lines = text.lines().map(|line| line.trim_matches(['\r', '\0']));
    let cut = match lines.next()? {
        "cut" => true,
        "copy" => false,
        _ => return None,
    };
    Some((cut, lines.filter(|line| !line.is_empty()).map(str::to_string).collect()))
}

/// Of the kinds of data a drag offers, the one to ask for: addresses
/// before text. `None` when it offers nothing a zone can take.
pub fn wanted_kind(offered: &[String]) -> Option<&str> {
    let named = |kind: &str| {
        offered
            .iter()
            .map(String::as_str)
            .find(|offer| offer.split(';').next().is_some_and(|base| base.trim().eq_ignore_ascii_case(kind)))
    };
    named(URI_LIST)
        .or_else(|| named(GNOME_COPIED_FILES))
        .or_else(|| offered.iter().map(String::as_str).find(|offer| offer.eq_ignore_ascii_case(TEXT)))
        .or_else(|| named("text/plain"))
        .or_else(|| named("UTF8_STRING"))
}

/// What a drag from another program carries, from the data it gave for
/// one kind. Addresses of files on this machine become paths; of anything
/// else (a web page, a file of another machine) they are passed on as
/// text, as is text itself. `allowed` is what the source lets be done; a
/// file manager's "cut" narrows it to moving.
pub fn payload(kind: &str, data: &[u8], allowed: DragActions) -> DropPayload {
    let text = String::from_utf8_lossy(data);
    let base = kind.split(';').next().unwrap_or(kind).trim();
    let mut allowed = allowed;
    let found = if base.eq_ignore_ascii_case(URI_LIST) {
        addresses(&text)
    } else if base.eq_ignore_ascii_case(GNOME_COPIED_FILES) {
        match read_gnome_copied_files(&text) {
            Some((cut, found)) => {
                allowed = if cut {
                    DragActions { moving: true, ..DragActions::default() }
                } else {
                    DragActions { copy: true, ..DragActions::default() }
                };
                found
            }
            None => Vec::new(),
        }
    } else {
        let text = text.trim_end_matches('\0').to_string();
        return DropPayload {
            text: (!text.is_empty()).then_some(text),
            allowed,
            ..DropPayload::default()
        };
    };
    let mut paths = Vec::new();
    let mut others = Vec::new();
    for address in found {
        match path_of(&address).and_then(|path| path.into_os_string().into_string().ok()) {
            Some(path) => paths.push(path),
            None => others.push(address),
        }
    }
    DropPayload {
        items: Vec::new(),
        paths,
        text: (!others.is_empty()).then(|| others.join("\n")),
        allowed,
    }
}

/// What a drag that began in a zone carries, as the core wants it: the
/// items themselves, and their files for a portal to take.
pub fn payload_of_items(items: &[ItemId], allowed: DragActions) -> DropPayload {
    DropPayload {
        items: items.to_vec(),
        paths: items
            .iter()
            .filter_map(path_of_item)
            .filter_map(|path| path.into_os_string().into_string().ok())
            .collect(),
        text: None,
        allowed,
    }
}

/// The data a drag of these items gives another program for one of the
/// kinds in [`OFFERED`]. `None` for a kind that was not offered.
pub fn data_for(kind: &str, items: &[ItemId]) -> Option<Vec<u8>> {
    let paths: Vec<PathBuf> = items.iter().filter_map(path_of_item).collect();
    let base = kind.split(';').next().unwrap_or(kind).trim();
    if base.eq_ignore_ascii_case(URI_LIST) {
        Some(uri_list(&paths).into_bytes())
    } else if base.eq_ignore_ascii_case(GNOME_COPIED_FILES) {
        Some(gnome_copied_files(&paths, false).into_bytes())
    } else if base.eq_ignore_ascii_case("text/plain") {
        // As text, files are their paths, one to a line.
        let lines: Vec<&[u8]> = paths.iter().map(|path| path.as_os_str().as_bytes()).collect();
        Some(lines.join(&b"\n"[..]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_and_its_address_lead_to_each_other() {
        for (path, uri) in [
            ("/home/ada/Desktop/a.txt", "file:///home/ada/Desktop/a.txt"),
            ("/home/ada/a b.txt", "file:///home/ada/a%20b.txt"),
            ("/home/ada/Masaüstü/çay.txt", "file:///home/ada/Masa%C3%BCst%C3%BC/%C3%A7ay.txt"),
            ("/tmp/100% #1?.txt", "file:///tmp/100%25%20%231%3F.txt"),
            // What GLib leaves alone is left alone.
            ("/tmp/a(1)+b,c;d=e@f:g!h$i&j'k~l", "file:///tmp/a(1)+b,c;d=e@f:g!h$i&j'k~l"),
            ("/tmp/[x]{y}\"z\"\\", "file:///tmp/%5Bx%5D%7By%7D%22z%22%5C"),
        ] {
            assert_eq!(uri_of(Path::new(path)), uri, "{path}");
            assert_eq!(path_of(uri), Some(PathBuf::from(path)), "{uri}");
        }
        // A name that is not text keeps every byte.
        let odd = PathBuf::from(OsString::from_vec(b"/tmp/caf\xe9.txt".to_vec()));
        assert_eq!(uri_of(&odd), "file:///tmp/caf%E9.txt");
        assert_eq!(path_of("file:///tmp/caf%E9.txt"), Some(odd.clone()));
        assert_eq!(path_of_item(&item_id(&odd)), Some(odd));
    }

    #[test]
    fn only_a_file_of_this_machine_is_a_path() {
        assert_eq!(path_of("file://localhost/etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        assert_eq!(path_of("FILE:///etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        assert_eq!(path_of("file:/etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        assert_eq!(path_of("file:///a%2fb"), Some(PathBuf::from("/a/b")));
        for not_a_file_here in [
            "file://server/share/a.txt",
            "https://example.org/a.txt",
            "file:relative",
            "file:///bad%2",
            "file:///bad%zz",
            "file:///bad%+1",
            "file:///nul%00",
            "/just/a/path",
            "",
        ] {
            assert_eq!(path_of(not_a_file_here), None, "{not_a_file_here}");
        }
    }

    #[test]
    fn a_list_of_addresses_goes_out_and_comes_back() {
        let paths = vec![
            PathBuf::from("/home/ada/Desktop/a b.txt"),
            PathBuf::from("/home/ada/Desktop/çay ☕.txt"),
        ];
        let list = uri_list(&paths);
        assert_eq!(
            list,
            "file:///home/ada/Desktop/a%20b.txt\r\nfile:///home/ada/Desktop/%C3%A7ay%20%E2%98%95.txt\r\n"
        );
        let all = DragActions::ALL;
        let back = payload(URI_LIST, list.as_bytes(), all);
        assert_eq!(back.paths, ["/home/ada/Desktop/a b.txt", "/home/ada/Desktop/çay ☕.txt"]);
        assert_eq!((back.text, back.allowed), (None, all));
        // Comments, plain line ends, an address that is no file of ours.
        let mixed = "# from a browser\nfile:///tmp/x.txt\nhttps://example.org/a%20b\n\n";
        let back = payload("text/uri-list;charset=utf-8", mixed.as_bytes(), all);
        assert_eq!(back.paths, ["/tmp/x.txt"]);
        assert_eq!(back.text.as_deref(), Some("https://example.org/a%20b"));
    }

    #[test]
    fn what_a_file_manager_copied_is_read_with_whether_it_was_cut() {
        let paths = vec![PathBuf::from("/home/ada/a b.txt"), PathBuf::from("/home/ada/c.txt")];
        let copied = gnome_copied_files(&paths, false);
        assert_eq!(copied, "copy\nfile:///home/ada/a%20b.txt\nfile:///home/ada/c.txt");
        let back = payload(GNOME_COPIED_FILES, copied.as_bytes(), DragActions::ALL);
        assert_eq!(back.paths, ["/home/ada/a b.txt", "/home/ada/c.txt"]);
        assert_eq!(back.allowed, DragActions { copy: true, moving: false, link: false });
        let cut = gnome_copied_files(&paths[..1], true);
        let back = payload(GNOME_COPIED_FILES, cut.as_bytes(), DragActions::ALL);
        assert_eq!(back.allowed, DragActions { copy: false, moving: true, link: false });
        assert_eq!(read_gnome_copied_files("paste\nfile:///x"), None);
    }

    #[test]
    fn text_is_passed_on_as_text() {
        let back = payload(TEXT, "hello there\0".as_bytes(), DragActions::ALL);
        assert_eq!(back.text.as_deref(), Some("hello there"));
        assert!(back.paths.is_empty());
    }

    #[test]
    fn of_what_a_drag_offers_addresses_are_asked_for_first() {
        let offers = |kinds: &[&str]| kinds.iter().map(|kind| kind.to_string()).collect::<Vec<_>>();
        let browser = offers(&["text/html", "text/plain", "text/uri-list"]);
        assert_eq!(wanted_kind(&browser), Some("text/uri-list"));
        let files = offers(&["x-special/gnome-copied-files", "text/plain;charset=utf-8"]);
        assert_eq!(wanted_kind(&files), Some("x-special/gnome-copied-files"));
        let text = offers(&["UTF8_STRING", "text/plain", "text/plain;charset=utf-8"]);
        assert_eq!(wanted_kind(&text), Some("text/plain;charset=utf-8"));
        assert_eq!(wanted_kind(&offers(&["image/png"])), None);
    }

    #[test]
    fn items_dragged_out_are_given_in_every_kind_offered() {
        let items = [
            item_id(Path::new("/home/ada/Desktop/a b.txt")),
            item_id(Path::new("/home/ada/Desktop/ö.txt")),
        ];
        let said = |kind: &str| String::from_utf8(data_for(kind, &items).expect("data")).expect("text");
        assert_eq!(
            said(URI_LIST),
            "file:///home/ada/Desktop/a%20b.txt\r\nfile:///home/ada/Desktop/%C3%B6.txt\r\n"
        );
        assert!(said(GNOME_COPIED_FILES).starts_with("copy\nfile:///home/ada/Desktop/a%20b.txt\n"));
        assert_eq!(said(TEXT), "/home/ada/Desktop/a b.txt\n/home/ada/Desktop/ö.txt");
        assert_eq!(data_for("image/png", &items), None);
        for kind in OFFERED {
            assert!(data_for(kind, &items).is_some(), "{kind}");
        }
        let carried = payload_of_items(&items, DragActions::ALL);
        assert_eq!(carried.items, items);
        assert_eq!(carried.paths, ["/home/ada/Desktop/a b.txt", "/home/ada/Desktop/ö.txt"]);
    }
}
