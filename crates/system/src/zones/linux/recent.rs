//! The files the person opened lately, as the desktop keeps them:
//! `$XDG_DATA_HOME/recently-used.xbel`, which GTK's file chooser and the
//! programs that use its recent manager write (the Desktop Bookmark
//! specification: an XBEL document with one `<bookmark>` for each file).
//! Qt's programs keep lists of their own and do not write here; what
//! they opened is not in the list.
//!
//! Each bookmark has the file's address (`href`) and three times:
//! `added`, `modified` and `visited`. The list a zone shows is the files
//! that are still there, the last visited first, each with that time as
//! its fact `accessed`.
//!
//! The document is read by hand, as the other files of the desktop are
//! here: only the opening tags of the bookmarks are looked at, and of
//! those only their attributes.

use super::{uris, System};
use gevrek_zones::item::Item;
use std::path::{Path, PathBuf};

/// How many files a list of recent ones holds at most.
pub const MOST: usize = 100;

/// One bookmark: a file's address and when it was last opened (seconds
/// since 1970).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub href: String,
    pub visited: u64,
}

/// Where the record is.
pub fn file(data_home: &Path) -> PathBuf {
    data_home.join("recently-used.xbel")
}

/// The text of an attribute's value with its entities read (`&amp;`,
/// `&lt;`, `&gt;`, `&quot;`, `&apos;`, and letters by number).
fn plain(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let name = &rest[1..end];
        let letter = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|number| number.parse().ok()))
                .and_then(char::from_u32),
        };
        match letter {
            Some(letter) => out.push(letter),
            // (Not an entity anyone knows: left as it is written.)
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The attributes of an opening tag: `name="value"` or `name='value'`.
fn attributes(tag: &str) -> Vec<(&str, String)> {
    let mut found = Vec::new();
    let mut rest = tag;
    while let Some(equals) = rest.find('=') {
        let name = rest[..equals].split_whitespace().last().unwrap_or_default();
        let after = rest[equals + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|quote| *quote == '"' || *quote == '\'') else {
            break;
        };
        let Some(end) = after[1..].find(quote) else {
            break;
        };
        found.push((name, plain(&after[1..=end])));
        rest = &after[end + 2..];
    }
    found
}

/// A time as the record writes it (`2026-11-19T09:12:33.123456Z`, or
/// with a distance from Greenwich) as seconds since 1970.
fn seconds(time: &str) -> Option<u64> {
    let stamp: jiff::Timestamp = time.trim().parse().ok()?;
    u64::try_from(stamp.as_second()).ok()
}

/// The bookmarks of a record, in the order they are written in. One
/// without an address or without any time is left out; where it was
/// never visited, the time it was added or changed counts.
pub fn bookmarks(text: &str) -> Vec<Bookmark> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<bookmark") {
        rest = &rest[at + "<bookmark".len()..];
        // (`<bookmark:applications>` and its kin are other tags.)
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(end) = rest.find('>') else {
            break;
        };
        let said = attributes(&rest[..end]);
        let value = |name: &str| said.iter().find(|(known, _)| *known == name).map(|(_, value)| value.as_str());
        let visited = ["visited", "modified", "added"].into_iter().find_map(|name| value(name).and_then(seconds));
        if let (Some(href), Some(visited)) = (value("href"), visited) {
            found.push(Bookmark { href: href.to_string(), visited });
        }
        rest = &rest[end..];
    }
    found
}

/// The files opened lately, the last opened first: those of the record
/// that are files of this machine and still there (as a zone that shows
/// their folder has them), no more than `most`.
pub fn read(system: &System, most: usize) -> Vec<Item> {
    let Ok(text) = std::fs::read_to_string(file(&system.places.data_home)) else {
        return Vec::new();
    };
    let mut marks = bookmarks(&text);
    marks.sort_by(|one, other| other.visited.cmp(&one.visited).then_with(|| one.href.cmp(&other.href)));
    let mut items: Vec<Item> = Vec::new();
    for mark in marks {
        if items.len() >= most {
            break;
        }
        let Some(mut item) = uris::path_of(&mark.href).and_then(|path| system.item(&path)) else {
            continue;
        };
        if items.iter().any(|known| known.id == item.id) {
            continue;
        }
        item.facts.accessed = Some(mark.visited);
        items.push(item);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0"
      xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info">
  <bookmark href="file:///home/ada/Documents/notes%20&amp;%20more.txt" added="2026-11-01T10:00:00Z" modified="2026-11-18T08:00:00Z" visited="2026-11-19T09:12:33.123456Z">
    <info>
      <metadata owner="http://freedesktop.org">
        <mime:mime-type type="text/plain"/>
        <bookmark:applications>
          <bookmark:application name="gedit" exec="&apos;gedit %u&apos;" modified="2026-11-19T09:12:33.123456Z" count="3"/>
        </bookmark:applications>
      </metadata>
    </info>
  </bookmark>
  <bookmark href='file:///home/ada/a.png' added='2026-11-02T10:00:00+01:00' modified='2026-11-02T10:00:00+01:00'/>
  <bookmark href="sftp://server/remote.txt" visited="2026-11-19T10:00:00Z"/>
  <bookmark added="2026-11-02T10:00:00Z"/>
  <bookmark href="file:///home/ada/no-time.txt"/>
</xbel>
"#;

    #[test]
    fn the_record_is_read_by_its_bookmarks_attributes() {
        let marks = bookmarks(RECORD);
        assert_eq!(
            marks,
            [
                // The entity is read; the time is the visit's, to the second.
                Bookmark { href: "file:///home/ada/Documents/notes%20&%20more.txt".to_string(), visited: 1_795_079_553 },
                // Never visited: when it was changed, an hour east of Greenwich.
                Bookmark { href: "file:///home/ada/a.png".to_string(), visited: 1_793_610_000 },
                Bookmark { href: "sftp://server/remote.txt".to_string(), visited: 1_795_082_400 },
            ]
        );
        assert_eq!(bookmarks(""), []);
        assert_eq!(bookmarks("<xbel><bookmark href=\"file:///a\" visited=\"not a time\"/>"), []);
        assert_eq!(bookmarks("<bookmark href=\"file:///cut off"), []);
        assert_eq!(plain("a &lt;b&gt; &#65;&#x42; &unknown; &"), "a <b> AB &unknown; &");
    }

    #[test]
    fn recent_files_are_those_that_are_there_the_last_opened_first() {
        let root = super::super::testing::folder("recent");
        let places = super::super::Places::under(&root);
        let docs = places.home.join("Documents");
        std::fs::create_dir_all(&docs).expect("folders");
        std::fs::create_dir_all(&places.data_home).expect("folders");
        for name in ["old.txt", "new file.txt", "twice.txt"] {
            std::fs::write(docs.join(name), b"x").expect("written");
        }
        let address = |name: &str| uris::uri_of(&docs.join(name));
        let record = format!(
            "<xbel>\n<bookmark href=\"{}\" visited=\"2026-11-10T00:00:00Z\"/>\n<bookmark href=\"{}\" visited=\"2026-11-19T00:00:00Z\"/>\n<bookmark href=\"{}\" visited=\"2026-11-18T00:00:00Z\"/>\n<bookmark href=\"{}\" visited=\"2026-11-12T00:00:00Z\"/>\n<bookmark href=\"{}\" visited=\"2026-11-15T00:00:00Z\"/>\n<bookmark href=\"sftp://server/far.txt\" visited=\"2026-11-19T10:00:00Z\"/>\n</xbel>\n",
            address("old.txt"),
            address("new file.txt"),
            address("gone.txt"),
            address("twice.txt"),
            address("twice.txt"),
        );
        std::fs::write(file(&places.data_home), record).expect("written");
        let system = System::at(places);
        let items = read(&system, MOST);
        let seen: Vec<(&str, Option<u64>)> = items.iter().map(|item| (item.name.as_str(), item.facts.accessed)).collect();
        assert_eq!(
            seen,
            [("new file.txt", Some(1_795_046_400)), ("twice.txt", Some(1_794_700_800)), ("old.txt", Some(1_794_268_800))]
        );
        // The item is the file itself, as a zone of its folder has it.
        assert_eq!(items[0].id, uris::item_id(&docs.join("new file.txt")));
        assert_eq!(items[0].facts.extension.as_deref(), Some("txt"));
        assert_eq!(read(&system, 1).len(), 1);
        // No record: no recent files.
        std::fs::remove_file(file(&system.places.data_home)).expect("removed");
        assert_eq!(read(&system, MOST), []);
        let _ = std::fs::remove_dir_all(&root);
    }
}
