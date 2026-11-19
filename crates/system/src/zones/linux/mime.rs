//! What kind of thing a file is (its MIME type), by the shared MIME-info
//! database: the files `update-mime-database` writes into `<data>/mime`.
//!
//! - `globs2` says which names mean which type (`50:text/plain:*.txt`),
//!   `magic` which bytes at the start of a file do. A name is asked first;
//!   the bytes only where the name says nothing, or more than one thing.
//!   (The specification would have strong magic looked at before any
//!   name. That means opening every file of a folder to list it, so here,
//!   as in GLib's quick answer, a name that decides is believed.)
//! - `aliases` and `subclasses` say which types are the same and which
//!   are a kind of another; `icons` and `generic-icons` name the icon of a
//!   type.
//!
//! The database is read by hand, as desktop entries and icon themes are
//! in this crate.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// A file of no known kind.
pub const UNKNOWN: &str = "application/octet-stream";
pub const TEXT: &str = "text/plain";
pub const FOLDER: &str = "inode/directory";
/// What a desktop entry is.
pub const DESKTOP_ENTRY: &str = "application/x-desktop";

/// How much of a file is read at most to tell its kind by its bytes.
const MOST_READ: usize = 128 * 1024;

#[derive(Debug, Clone)]
struct Glob {
    weight: u32,
    mime: String,
    pattern: String,
    /// The pattern in small letters, for a name that fits no pattern as
    /// it is written. None for a pattern that only counts as written
    /// (`*.C` is C++, `*.c` C).
    lower: Option<String>,
}

/// One test of a file's bytes, and the tests that must also hold.
#[derive(Debug, Clone)]
struct Rule {
    start: usize,
    value: Vec<u8>,
    mask: Option<Vec<u8>>,
    /// At how many places from `start` on the value may begin.
    range: usize,
    children: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Magic {
    priority: u32,
    mime: String,
    rules: Vec<Rule>,
}

/// The database, read into memory.
#[derive(Debug, Clone, Default)]
pub struct MimeDatabase {
    globs: Vec<Glob>,
    /// The strongest first.
    magic: Vec<Magic>,
    aliases: HashMap<String, String>,
    parents: HashMap<String, Vec<String>>,
    icons: HashMap<String, String>,
    generic_icons: HashMap<String, String>,
    /// How many bytes of a file the tests look at.
    extent: usize,
}

impl MimeDatabase {
    /// The database of these data directories (`<data>/mime` of each),
    /// the most important first. A directory that has none adds nothing.
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        let mut database = Self::default();
        // From the least important on, so that a more important one can
        // take back what a lesser one said.
        for dir in data_dirs.iter().rev() {
            let dir = dir.join("mime");
            let text = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
            match text("globs2") {
                Some(globs) => database.add_globs(&globs, true),
                // (A database from before weights were written.)
                None => database.add_globs(&text("globs").unwrap_or_default(), false),
            }
            if let Ok(magic) = std::fs::read(dir.join("magic")) {
                database.add_magic(&magic);
            }
            for (name, map) in [
                ("aliases", &mut database.aliases),
                ("icons", &mut database.icons),
                ("generic-icons", &mut database.generic_icons),
            ] {
                let separator = if name == "aliases" { ' ' } else { ':' };
                for line in text(name).unwrap_or_default().lines() {
                    if let Some((key, value)) = line.trim().split_once(separator) {
                        map.insert(key.to_string(), value.trim().to_string());
                    }
                }
            }
            for line in text("subclasses").unwrap_or_default().lines() {
                if let Some((kind, parent)) = line.trim().split_once(' ') {
                    let parents = database.parents.entry(kind.to_string()).or_default();
                    if !parents.iter().any(|known| known == parent) {
                        parents.push(parent.to_string());
                    }
                }
            }
        }
        database.magic.sort_by_key(|magic| std::cmp::Reverse(magic.priority));
        database.extent = database
            .magic
            .iter()
            .flat_map(|magic| magic.rules.iter())
            .map(Rule::extent)
            .max()
            .unwrap_or(0)
            .min(MOST_READ);
        database
    }

    /// The lines of a `globs2` file (`weight:type:pattern[:flags]`), or of
    /// a `globs` file (`type:pattern`, every weight 50).
    fn add_globs(&mut self, text: &str, weighted: bool) {
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split(':');
            let weight = if weighted {
                match parts.next().and_then(|weight| weight.parse::<u32>().ok()) {
                    Some(weight) => weight,
                    None => continue,
                }
            } else {
                50
            };
            let (Some(mime), Some(pattern)) = (parts.next(), parts.next()) else {
                continue;
            };
            if pattern == "__NOGLOBS__" {
                // This directory takes back what lesser ones said of the type.
                self.globs.retain(|glob| glob.mime != mime);
                continue;
            }
            let case_sensitive = parts
                .next()
                .is_some_and(|flags| flags.split(',').any(|flag| flag == "cs"));
            self.globs.push(Glob {
                weight,
                mime: mime.to_string(),
                pattern: pattern.to_string(),
                lower: (!case_sensitive).then(|| pattern.to_lowercase()),
            });
        }
    }

    /// The sections of a `magic` file. It is text and bytes mixed: each
    /// section begins `[priority:type]`, and each line under it is
    /// `[indent]>start=` then two bytes of length, the value, and
    /// optionally `&` a mask, `~` a word size, `+` a range. What cannot be
    /// read ends the reading: there is no telling where the next line
    /// begins.
    fn add_magic(&mut self, data: &[u8]) {
        let Some(mut rest) = data.strip_prefix(b"MIME-Magic\0\n") else {
            return;
        };
        while let Some(after) = rest.strip_prefix(b"[") {
            let Some(end) = after.iter().position(|&byte| byte == b'\n') else {
                return;
            };
            let head = String::from_utf8_lossy(&after[..end]);
            let Some((priority, mime)) = head.trim_end_matches(']').split_once(':') else {
                return;
            };
            let (Ok(priority), mime) = (priority.parse::<u32>(), mime.to_string()) else {
                return;
            };
            rest = &after[end + 1..];
            let mut flat: Vec<(usize, Rule)> = Vec::new();
            while !rest.is_empty() && rest[0] != b'[' {
                if let Some(after) = rest.strip_prefix(b"__NOMAGIC__\n") {
                    self.magic.retain(|magic| magic.mime != mime);
                    rest = after;
                    continue;
                }
                let Some((indent, rule, after)) = read_rule(rest) else {
                    return;
                };
                flat.push((indent, rule));
                rest = after;
            }
            let rules = nest(&mut flat.into_iter().peekable(), 0);
            if !rules.is_empty() {
                self.magic.push(Magic { priority, mime, rules });
            }
        }
    }

    /// The type another name for a type stands for.
    pub fn unaliased<'a>(&'a self, mime: &'a str) -> &'a str {
        self.aliases.get(mime).map_or(mime, String::as_str)
    }

    /// The types a file's name may mean: those of the heaviest patterns
    /// that fit, and of those the longest. More than one is left where the
    /// database does not decide (`*.m` is Objective-C and MATLAB). The
    /// name is tried as it is written first, and only when nothing fits
    /// it so, without regard to large and small letters.
    pub fn by_name(&self, name: &str) -> Vec<&str> {
        let as_written = self.best(|glob| fits(&glob.pattern, name));
        if !as_written.is_empty() {
            return as_written;
        }
        let lower = name.to_lowercase();
        self.best(|glob| glob.lower.as_ref().is_some_and(|pattern| fits(pattern, &lower)))
    }

    /// Of the patterns that fit, the types of the heaviest and longest.
    fn best(&self, fits: impl Fn(&Glob) -> bool) -> Vec<&str> {
        let mut best: Option<(u32, usize)> = None;
        let mut found: Vec<&str> = Vec::new();
        for glob in self.globs.iter().filter(|glob| fits(glob)) {
            let rank = (glob.weight, glob.pattern.len());
            if best.is_none_or(|known| rank > known) {
                best = Some(rank);
                found.clear();
            }
            if best == Some(rank) && !found.contains(&glob.mime.as_str()) {
                found.push(&glob.mime);
            }
        }
        found
    }

    /// The type the start of a file's bytes means, by the strongest test
    /// that holds.
    pub fn by_content(&self, data: &[u8]) -> Option<&str> {
        self.magic
            .iter()
            .find(|magic| magic.rules.iter().any(|rule| rule.holds(data)))
            .map(|magic| magic.mime.as_str())
    }

    /// The type of what is at `path`: of a folder and the like by what it
    /// is, of a file by its name, and by its bytes where the name does
    /// not decide. A link is what it leads to.
    pub fn of_file(&self, path: &Path) -> String {
        let Ok(data) = std::fs::metadata(path) else {
            // (A link that leads nowhere is still there itself.)
            return if path.symlink_metadata().is_ok() { "inode/symlink" } else { UNKNOWN }.to_string();
        };
        let kind = data.file_type();
        if kind.is_dir() {
            return FOLDER.to_string();
        }
        if !kind.is_file() {
            use std::os::unix::fs::FileTypeExt;
            return if kind.is_fifo() {
                "inode/fifo"
            } else if kind.is_socket() {
                "inode/socket"
            } else if kind.is_block_device() {
                "inode/blockdevice"
            } else if kind.is_char_device() {
                "inode/chardevice"
            } else {
                UNKNOWN
            }
            .to_string();
        }
        let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
        let named = self.by_name(&name);
        if let [one] = named[..] {
            return one.to_string();
        }
        if data.len() == 0 {
            return named.first().copied().unwrap_or("application/x-zerosize").to_string();
        }
        let start = start_of(path, self.extent.max(256));
        let sniffed = self.by_content(&start);
        if let Some(&first) = named.first() {
            // The name gave several: the bytes choose among them, and
            // where they point elsewhere, the name still counts for more.
            return sniffed
                .and_then(|sniffed| {
                    named.iter().find(|&&candidate| self.is_a(candidate, sniffed)).copied()
                })
                .unwrap_or(first)
                .to_string();
        }
        match sniffed {
            Some(sniffed) => sniffed.to_string(),
            None if looks_like_text(&start) => TEXT.to_string(),
            None => UNKNOWN.to_string(),
        }
    }

    /// Whether `mime` is `ancestor` or a kind of it.
    pub fn is_a(&self, mime: &str, ancestor: &str) -> bool {
        let ancestor = self.unaliased(ancestor);
        self.with_ancestors(mime).iter().any(|kind| kind == ancestor)
    }

    /// A type and what it is a kind of, the nearest first: its parents by
    /// the database, and for every text type plain text. (What opens the
    /// general kind also opens the special one.)
    pub fn with_ancestors(&self, mime: &str) -> Vec<String> {
        let mut found = vec![self.unaliased(mime).to_string()];
        let mut at = 0;
        while at < found.len() {
            let kind = found[at].clone();
            let mut parents: Vec<String> = self.parents.get(&kind).cloned().unwrap_or_default();
            if kind.starts_with("text/") && kind != TEXT {
                parents.push(TEXT.to_string());
            }
            for parent in parents {
                let parent = self.unaliased(&parent).to_string();
                if !found.contains(&parent) {
                    found.push(parent);
                }
            }
            at += 1;
        }
        found
    }

    /// The names an icon theme may have a type's icon under, the most
    /// exact first: what the database names for it, the type itself with
    /// a stroke for the slash (`text-x-python`), the icon of its family
    /// (`text-x-generic`), and last the icon of anything unknown.
    pub fn icon_names(&self, mime: &str) -> Vec<String> {
        let mime = self.unaliased(mime);
        if mime == FOLDER {
            return vec!["folder".to_string(), "inode-directory".to_string()];
        }
        let mut names: Vec<String> = Vec::new();
        let mut add = |name: String| {
            if !names.contains(&name) {
                names.push(name);
            }
        };
        if let Some(icon) = self.icons.get(mime) {
            add(icon.clone());
        }
        add(mime.replace('/', "-"));
        if let Some(icon) = self.generic_icons.get(mime) {
            add(icon.clone());
        }
        if let Some((family, _)) = mime.split_once('/') {
            add(format!("{family}-x-generic"));
        }
        add("unknown".to_string());
        names
    }
}

impl Rule {
    /// How far into a file this test and those under it reach.
    fn extent(&self) -> usize {
        let own = self.start.saturating_add(self.range).saturating_add(self.value.len());
        self.children.iter().map(Rule::extent).max().unwrap_or(0).max(own)
    }

    /// Whether the value is in the data at one of its places, and, where
    /// tests hang under this one, one of those holds as well.
    fn holds(&self, data: &[u8]) -> bool {
        let length = self.value.len();
        let here = (self.start..self.start.saturating_add(self.range)).any(|at| {
            let Some(found) = at.checked_add(length).and_then(|end| data.get(at..end)) else {
                return false;
            };
            match &self.mask {
                Some(mask) => found
                    .iter()
                    .zip(&self.value)
                    .zip(mask)
                    .all(|((found, value), mask)| found & mask == value & mask),
                None => found == self.value,
            }
        });
        here && (self.children.is_empty() || self.children.iter().any(|child| child.holds(data)))
    }
}

/// A number written in digits at the start of `data`, and what follows.
fn number(data: &[u8]) -> Option<(usize, &[u8])> {
    let digits = data.iter().take_while(|byte| byte.is_ascii_digit()).count();
    let text = std::str::from_utf8(&data[..digits]).ok()?;
    Some((text.parse().ok()?, &data[digits..]))
}

/// One line of a magic section: how deep it hangs, the test, and what
/// follows the line.
fn read_rule(data: &[u8]) -> Option<(usize, Rule, &[u8])> {
    let (indent, rest) = match data.first()? {
        b'>' => (0, data),
        _ => number(data)?,
    };
    let rest = rest.strip_prefix(b">")?;
    let (start, rest) = number(rest)?;
    let rest = rest.strip_prefix(b"=")?;
    let length = usize::from(u16::from_be_bytes([*rest.first()?, *rest.get(1)?]));
    let mut value = rest.get(2..2 + length)?.to_vec();
    let mut rest = &rest[2 + length..];
    let mut mask = None;
    if let Some(after) = rest.strip_prefix(b"&") {
        mask = Some(after.get(..length)?.to_vec());
        rest = &after[length..];
    }
    let mut word = 1;
    if let Some(after) = rest.strip_prefix(b"~") {
        (word, rest) = number(after)?;
    }
    let mut range = 1;
    if let Some(after) = rest.strip_prefix(b"+") {
        (range, rest) = number(after)?;
    }
    let rest = rest.strip_prefix(b"\n")?;
    // A value of words is written with the high byte first; a file has
    // them as the machine does.
    if cfg!(target_endian = "little") && word > 1 && length % word == 0 {
        for bytes in [Some(&mut value), mask.as_mut()].into_iter().flatten() {
            bytes.chunks_exact_mut(word).for_each(<[u8]>::reverse);
        }
    }
    let rule = Rule { start, value, mask, range: range.max(1), children: Vec::new() };
    Some((indent, rule, rest))
}

/// The tests of a section as a tree: a line hangs under the last line
/// that is less deep.
fn nest(flat: &mut std::iter::Peekable<impl Iterator<Item = (usize, Rule)>>, depth: usize) -> Vec<Rule> {
    let mut rules: Vec<Rule> = Vec::new();
    while let Some(indent) = flat.peek().map(|(indent, _)| *indent) {
        if indent < depth {
            break;
        }
        if indent > depth {
            let children = nest(flat, indent);
            // (Deeper lines with nothing above them belong to nothing.)
            if let Some(parent) = rules.last_mut() {
                parent.children.extend(children);
            }
            continue;
        }
        if let Some((_, rule)) = flat.next() {
            rules.push(rule);
        }
    }
    rules
}

/// Whether a name fits a pattern of the database: `*` is anything, `?`
/// one letter, `[a-z]` one of some letters.
fn fits(pattern: &str, name: &str) -> bool {
    // Nearly every pattern is `*.ending`.
    if let Some(ending) = pattern.strip_prefix('*') {
        if !ending.contains(['*', '?', '[']) {
            return name.len() > ending.len() && name.ends_with(ending);
        }
    } else if !pattern.contains(['*', '?', '[']) {
        return pattern == name;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    fits_from(&pattern, &name)
}

fn fits_from(pattern: &[char], name: &[char]) -> bool {
    let Some((&first, rest)) = pattern.split_first() else {
        return name.is_empty();
    };
    match first {
        '*' => (0..=name.len()).any(|skipped| fits_from(rest, &name[skipped..])),
        '?' => !name.is_empty() && fits_from(rest, &name[1..]),
        '[' => {
            let Some(close) = rest.iter().skip(1).position(|&letter| letter == ']').map(|at| at + 1) else {
                // No closing bracket: the bracket is a letter.
                return name.first() == Some(&'[') && fits_from(rest, &name[1..]);
            };
            let Some((&letter, name_rest)) = name.split_first() else {
                return false;
            };
            let (negated, set) = match rest[..close].split_first() {
                Some((&('!' | '^'), set)) => (true, set),
                _ => (false, &rest[..close]),
            };
            let mut inside = false;
            let mut at = 0;
            while at < set.len() {
                if at + 2 < set.len() && set[at + 1] == '-' {
                    inside |= set[at] <= letter && letter <= set[at + 2];
                    at += 3;
                } else {
                    inside |= set[at] == letter;
                    at += 1;
                }
            }
            inside != negated && fits_from(&rest[close + 1..], name_rest)
        }
        letter => name.first() == Some(&letter) && fits_from(rest, &name[1..]),
    }
}

/// The first bytes of a file, as many as are asked for at most.
fn start_of(path: &Path, most: usize) -> Vec<u8> {
    let mut start = Vec::new();
    // (Read without the file counting as used: see `open_quietly`.)
    if let Ok(file) = super::open_quietly(path) {
        let _ = file.take(most as u64).read_to_end(&mut start);
    }
    start
}

/// Whether bytes that no test knew read as text: nothing in the first
/// 128 that text does not have (the specification's own last resort).
fn looks_like_text(start: &[u8]) -> bool {
    !start.is_empty()
        && start
            .iter()
            .take(128)
            .all(|&byte| (byte >= 0x20 && byte != 0x7f) || matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One line of a magic file.
    fn rule(indent: usize, start: usize, value: &[u8], more: &[u8]) -> Vec<u8> {
        let mut line = Vec::new();
        if indent > 0 {
            line.extend(indent.to_string().bytes());
        }
        line.extend(format!(">{start}=").bytes());
        line.extend((value.len() as u16).to_be_bytes());
        line.extend(value);
        line.extend(more);
        line.push(b'\n');
        line
    }

    /// A small database written into `<data>/mime`, as
    /// `update-mime-database` would have: enough kinds to tell names,
    /// bytes, aliases, families and icons apart.
    pub(crate) fn write_database(data: &Path) {
        let mime = data.join("mime");
        std::fs::create_dir_all(&mime).expect("a folder");
        let write = |name: &str, text: &[u8]| std::fs::write(mime.join(name), text).expect("written");
        write(
            "globs2",
            b"# written by a test\n\
              50:text/plain:*.txt\n\
              50:image/png:*.png\n\
              50:image/jpeg:*.jpg\n\
              50:image/jpeg:*.jpeg\n\
              50:application/pdf:*.pdf\n\
              50:application/x-desktop:*.desktop\n\
              50:text/x-python:*.py\n\
              50:text/x-csrc:*.c\n\
              50:text/x-c++src:*.C:cs\n\
              50:application/x-compressed-tar:*.tar.gz\n\
              50:application/gzip:*.gz\n\
              50:text/x-makefile:makefile\n\
              50:text/x-objcsrc:*.m\n\
              50:text/x-matlab:*.m\n\
              60:text/x-readme:readme*\n\
              50:application/x-sharedlib:*.so.[0-9]*\n\
              10:text/x-weak:*.pdf\n",
        );
        let mut magic = b"MIME-Magic\0\n".to_vec();
        magic.extend(b"[50:image/png]\n");
        magic.extend(rule(0, 0, b"\x89PNG\r\n\x1a\n", b""));
        magic.extend(b"[50:application/pdf]\n");
        magic.extend(rule(0, 0, b"%PDF-", b"+1024"));
        magic.extend(b"[60:text/x-matlab]\n");
        magic.extend(rule(0, 0, b"%", b""));
        magic.extend(rule(1, 1, b" matlab", b""));
        magic.extend(b"[50:application/x-riff-wave]\n");
        magic.extend(rule(0, 0, b"RIFF", b""));
        magic.extend(rule(1, 8, b"WAVE", b""));
        magic.extend(rule(1, 8, b"wave", b""));
        magic.extend(b"[40:application/x-masked]\n");
        magic.extend(rule(0, 2, b"\x10\x00", b"&\xf0\x00"));
        magic.extend(b"[40:application/x-words]\n");
        // The 16-bit number 0x0102 as the machine keeps it.
        magic.extend(rule(0, 0, b"\x01\x02", b"~2"));
        write("magic", &magic);
        write("aliases", b"text/x-python3 text/x-python\napplication/x-pdf application/pdf\n");
        write(
            "subclasses",
            b"application/x-compressed-tar application/gzip\ntext/x-python application/x-executable\n",
        );
        write("generic-icons", b"text/x-python:text-x-script\napplication/pdf:x-office-document\n");
        write("icons", b"application/x-desktop:application-x-executable\n");
    }

    fn database() -> (PathBuf, MimeDatabase) {
        let root = crate::zones::linux::testing::folder("mime");
        write_database(&root);
        let database = MimeDatabase::load(std::slice::from_ref(&root));
        (root, database)
    }

    #[test]
    fn a_name_means_a_type_by_the_heaviest_and_longest_pattern() {
        let (root, database) = database();
        for (name, types) in [
            ("notes.txt", vec!["text/plain"]),
            ("NOTES.TXT", vec!["text/plain"]),
            ("photo.JPG", vec!["image/jpeg"]),
            // The longer pattern of two that fit.
            ("backup.tar.gz", vec!["application/x-compressed-tar"]),
            ("log.gz", vec!["application/gzip"]),
            // A pattern that only counts as written.
            ("main.c", vec!["text/x-csrc"]),
            ("main.C", vec!["text/x-c++src"]),
            // A whole name, a start, a set of letters.
            ("Makefile", vec!["text/x-makefile"]),
            ("README.md", vec!["text/x-readme"]),
            ("libz.so.1.2", vec!["application/x-sharedlib"]),
            ("libz.so.x", vec![]),
            // The heavier of two for one name.
            ("paper.pdf", vec!["application/pdf"]),
            // Two the database does not choose between.
            ("code.m", vec!["text/x-objcsrc", "text/x-matlab"]),
            ("nothing", vec![]),
            (".txt", vec![]),
        ] {
            assert_eq!(database.by_name(name), types, "{name}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bytes_mean_a_type_where_a_test_and_one_under_it_hold() {
        let (root, database) = database();
        assert_eq!(database.by_content(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        // Anywhere in the first kilobyte.
        assert_eq!(database.by_content(b"\n\n  %PDF-1.7"), Some("application/pdf"));
        // The upper test alone is not enough; one of the lower ones is.
        assert_eq!(database.by_content(b"RIFF....WAVEfmt "), Some("application/x-riff-wave"));
        assert_eq!(database.by_content(b"RIFF....wave"), Some("application/x-riff-wave"));
        assert_eq!(database.by_content(b"RIFF....AVI "), None);
        // Under a mask only the bits it names count.
        assert_eq!(database.by_content(b"zz\x1f\xff"), Some("application/x-masked"));
        assert_eq!(database.by_content(b"zz\x2f\xff"), None);
        // A value of words is compared as the machine keeps words.
        assert_eq!(database.by_content(&0x0102u16.to_ne_bytes()), Some("application/x-words"));
        // The stronger test first.
        assert_eq!(database.by_content(b"% matlab code"), Some("text/x-matlab"));
        assert_eq!(database.by_content(b""), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_is_told_by_its_name_and_by_its_bytes_where_the_name_does_not_say() {
        let (root, database) = database();
        let file = |name: &str, bytes: &[u8]| {
            let path = root.join(name);
            std::fs::write(&path, bytes).expect("written");
            database.of_file(&path)
        };
        assert_eq!(file("a.txt", b"hello"), "text/plain");
        // The name is believed even where the bytes say otherwise.
        assert_eq!(file("really-a-picture.txt", b"\x89PNG\r\n\x1a\n"), "text/plain");
        // No ending: the bytes.
        assert_eq!(file("picture", b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(file("words", b"just some words\n"), "text/plain");
        assert_eq!(file("bytes", b"\x00\x01\x02\x03"), "application/octet-stream");
        assert_eq!(file("empty", b""), "application/x-zerosize");
        // Two types for the name: the bytes choose, and the first stands
        // where they do not.
        assert_eq!(file("one.m", b"% matlab code"), "text/x-matlab");
        assert_eq!(file("two.m", b"#import <Foundation/Foundation.h>"), "text/x-objcsrc");
        std::fs::create_dir_all(root.join("folder.txt")).expect("a folder");
        assert_eq!(database.of_file(&root.join("folder.txt")), "inode/directory");
        // A link is what it leads to; one that leads nowhere is a link.
        std::os::unix::fs::symlink(root.join("picture"), root.join("link")).expect("a link");
        assert_eq!(database.of_file(&root.join("link")), "image/png");
        std::os::unix::fs::symlink(root.join("gone"), root.join("broken")).expect("a link");
        assert_eq!(database.of_file(&root.join("broken")), "inode/symlink");
        assert_eq!(database.of_file(&root.join("not-there")), "application/octet-stream");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn types_have_other_names_families_and_icons() {
        let (root, database) = database();
        assert_eq!(database.unaliased("application/x-pdf"), "application/pdf");
        assert_eq!(
            database.with_ancestors("text/x-python3"),
            ["text/x-python", "application/x-executable", "text/plain"]
        );
        assert!(database.is_a("application/x-compressed-tar", "application/gzip"));
        assert!(database.is_a("text/x-csrc", "text/plain"));
        assert!(!database.is_a("application/gzip", "application/x-compressed-tar"));
        assert_eq!(
            database.icon_names("text/x-python"),
            ["text-x-python", "text-x-script", "text-x-generic", "unknown"]
        );
        assert_eq!(
            database.icon_names("application/x-desktop"),
            ["application-x-executable", "application-x-desktop", "application-x-generic", "unknown"]
        );
        assert_eq!(database.icon_names("inode/directory")[0], "folder");
        assert_eq!(database.icon_names("image/png"), ["image-png", "image-x-generic", "unknown"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_more_important_directory_takes_back_what_a_lesser_one_said() {
        let root = crate::zones::linux::testing::folder("mime-two");
        let (user, system) = (root.join("user"), root.join("system"));
        write_database(&system);
        std::fs::create_dir_all(user.join("mime")).expect("a folder");
        std::fs::write(
            user.join("mime/globs2"),
            "0:text/plain:__NOGLOBS__\n50:text/x-log:*.txt\n80:text/x-mine:*.py\n",
        )
        .expect("written");
        let mut magic = b"MIME-Magic\0\n[50:image/png]\n__NOMAGIC__\n".to_vec();
        magic.extend(rule(0, 0, b"MINE", b""));
        std::fs::write(user.join("mime/magic"), magic).expect("written");
        let database = MimeDatabase::load(&[user, system]);
        assert_eq!(database.by_name("a.txt"), ["text/x-log"]);
        assert_eq!(database.by_name("a.py"), ["text/x-mine"]);
        assert_eq!(database.by_name("a.pdf"), ["application/pdf"]);
        assert_eq!(database.by_content(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(database.by_content(b"MINE"), Some("image/png"));
        // A database that is not there, or is not one, is no types.
        let none = MimeDatabase::load(&[root.join("nowhere")]);
        assert!(none.by_name("a.txt").is_empty());
        std::fs::write(root.join("system/mime/magic"), b"not a magic file").expect("written");
        let _ = MimeDatabase::load(&[root.join("system")]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
