//! The installed applications.
//!
//! On Linux they are the desktop entries (`*.desktop`) of the data
//! directories, read by the rules of the desktop-entry specification as
//! far as a launcher needs them: an entry that says it is hidden, or not
//! for this desktop, or whose program is not there, is left out; names
//! come in the user's language where the entry has them; an entry in the
//! user's own directory stands in front of the system's of the same name.
//!
//! On Windows the Start menu is the source ([`crate::start_menu`]): its
//! shortcuts and the packaged applications. An [`App`] reads the same
//! there with a few words meaning Windows' things: its `id` is the
//! shortcut's place under `Programs` or the packaged application's
//! identifier, its `icon` is what the shell draws the icon of
//! ([`crate::icons::picture`]), its `entry` is the shortcut (empty for a
//! packaged application), and it is started by `start`, as the Start
//! menu starts it, rather than by running its `command`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The name gevrek has in `OnlyShowIn` and `NotShowIn`.
pub const DESKTOP: &str = "gevrek";

/// One application that can be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// The name of its entry without `.desktop` (`org.mozilla.firefox`).
    pub id: String,
    pub name: String,
    /// What kind of thing it is ("Web Browser"), where the entry says.
    pub generic_name: String,
    pub comment: String,
    pub keywords: Vec<String>,
    /// The name of its icon, or the path of a picture.
    pub icon: String,
    /// The program and its arguments, ready to be started.
    pub command: Vec<String>,
    /// It wants a terminal to run in.
    pub terminal: bool,
    /// It is started by a message on the session's bus
    /// (`DBusActivatable`): its name there is its `id`. The `command` is
    /// what to do where that fails, and is empty where the entry has none.
    pub dbus_activatable: bool,
    /// The directory to start it in, where the entry names one.
    pub directory: Option<String>,
    /// The entry it was read from.
    pub entry: PathBuf,
    /// What else it can be started to do ("New private window"), where
    /// its entry names such things.
    pub actions: Vec<AppAction>,
}

/// One of the things an application can be started to do besides being
/// opened (a desktop entry's `Actions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAction {
    pub id: String,
    pub name: String,
    /// The program and its arguments, ready to be started.
    pub command: Vec<String>,
}

/// The fields of a desktop entry's main group, as they are written.
#[derive(Debug, Clone, Default)]
pub struct Fields<'a>(Vec<(&'a str, &'a str)>);

/// The main group (`[Desktop Entry]`) of an entry's text. The actions
/// have groups of their own, which are not in it.
pub fn main_group(text: &str) -> Fields<'_> {
    let mut fields: Vec<(&str, &str)> = Vec::new();
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.push((key.trim(), value.trim()));
        }
    }
    Fields(fields)
}

impl<'a> Fields<'a> {
    /// A field as it is written.
    pub fn plain(&self, key: &str) -> Option<&'a str> {
        self.0
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
    }

    /// A field as text in the user's language: `Name[tr]` before `Name`.
    pub fn localised(&self, key: &str, locales: &[String]) -> String {
        locales
            .iter()
            .find_map(|locale| self.plain(&format!("{key}[{locale}]")))
            .or_else(|| self.plain(key))
            .map(unescape)
            .unwrap_or_default()
    }

    pub fn yes(&self, key: &str) -> bool {
        self.plain(key) == Some("true")
    }

    /// A field that is a list (`a;b;`).
    pub fn list(&self, key: &str) -> Vec<String> {
        self.plain(key)
            .map(|value| {
                value
                    .split(';')
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Read one desktop entry. `locales` are the user's languages, the most
/// wanted first (`tr_TR`, `tr`). `None` when the entry is not an
/// application to show on this desktop.
pub fn parse(text: &str, id: &str, entry: &Path, locales: &[String]) -> Option<App> {
    parse_as(text, id, entry, locales, true)
}

/// Read one desktop entry as [`parse`] does. With `for_showing` false,
/// an entry that asks not to be listed (`NoDisplay`, `OnlyShowIn`,
/// `NotShowIn`) is read all the same: such an entry is not for a
/// launcher's list, but it still is the program files are opened with,
/// and it is what a launcher lying on the desktop is. One that says it
/// was deleted (`Hidden`) is no application either way.
pub fn parse_as(
    text: &str,
    id: &str,
    entry: &Path,
    locales: &[String],
    for_showing: bool,
) -> Option<App> {
    let fields = main_group(text);
    let plain = |key: &str| fields.plain(key);
    let localised = |key: &str| fields.localised(key, locales);
    let yes = |key: &str| fields.yes(key);
    let list = |key: &str| fields.list(key);
    if plain("Type") != Some("Application") || yes("Hidden") {
        return None;
    }
    if for_showing {
        if yes("NoDisplay") {
            return None;
        }
        let only = list("OnlyShowIn");
        if !only.is_empty()
            && !only
                .iter()
                .any(|desktop| desktop.eq_ignore_ascii_case(DESKTOP))
        {
            return None;
        }
        if list("NotShowIn")
            .iter()
            .any(|desktop| desktop.eq_ignore_ascii_case(DESKTOP))
        {
            return None;
        }
    }
    let name = localised("Name");
    if name.is_empty() {
        return None;
    }
    // Started over the bus, an entry needs no `Exec`; its name there has
    // to be one the bus takes.
    let dbus_activatable = yes("DBusActivatable") && bus_path_of(id).is_some();
    let icon = plain("Icon").unwrap_or_default().to_string();
    let command = plain("Exec")
        .map(|exec| command_of(exec, &name, &icon, entry))
        .unwrap_or_default();
    if command.is_empty() && !dbus_activatable {
        return None;
    }
    let keywords = locales
        .iter()
        .find_map(|locale| plain(&format!("Keywords[{locale}]")))
        .or_else(|| plain("Keywords"))
        .map(|value| {
            value
                .split(';')
                .filter(|part| !part.is_empty())
                .map(unescape)
                .collect()
        })
        .unwrap_or_default();
    // Its actions: each named in `Actions` has a group of its own with a
    // name and a command; one without either is left out.
    let actions = list("Actions")
        .into_iter()
        .filter_map(|action| {
            let group = format!("[Desktop Action {action}]");
            let mut inside = false;
            let mut action_fields: Vec<(&str, &str)> = Vec::new();
            for line in text.lines().map(str::trim) {
                if line.starts_with('[') {
                    inside = line == group;
                } else if inside && !line.starts_with('#') {
                    if let Some((key, value)) = line.split_once('=') {
                        action_fields.push((key.trim(), value.trim()));
                    }
                }
            }
            let of = |key: &str| {
                action_fields
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| *value)
            };
            let action_name = locales
                .iter()
                .find_map(|locale| of(&format!("Name[{locale}]")))
                .or_else(|| of("Name"))
                .map(unescape)
                .filter(|name| !name.is_empty())?;
            let command = of("Exec")
                .map(|exec| command_of(exec, &action_name, &icon, entry))
                .unwrap_or_default();
            (!command.is_empty() || dbus_activatable).then_some(AppAction {
                id: action,
                name: action_name,
                command,
            })
        })
        .collect();
    Some(App {
        id: id.to_string(),
        actions,
        name,
        generic_name: localised("GenericName"),
        comment: localised("Comment"),
        keywords,
        icon,
        command,
        terminal: yes("Terminal"),
        dbus_activatable,
        directory: plain("Path")
            .filter(|path| !path.is_empty())
            .map(str::to_string),
        entry: entry.to_path_buf(),
    })
}

/// Where an application that is started over the bus answers: the path
/// of the name it has there (`org.gnome.Calculator` is at
/// `/org/gnome/Calculator`, a `-` is a `_` in the path). `None` when the
/// name is not one the bus takes: at least two parts, of letters, digits,
/// `_` and `-`, none empty or starting with a digit.
pub fn bus_path_of(id: &str) -> Option<String> {
    let parts: Vec<&str> = id.split('.').collect();
    let good = |part: &&str| {
        !part.is_empty()
            && !part.starts_with(|letter: char| letter.is_ascii_digit())
            && part
                .chars()
                .all(|letter| letter.is_ascii_alphanumeric() || letter == '_' || letter == '-')
    };
    (parts.len() >= 2 && id.len() <= 255 && parts.iter().all(good))
        .then(|| format!("/{}", id.replace('.', "/").replace('-', "_")))
}

/// A value of an entry as text: `\s`, `\n`, `\t`, `\\` are what they stand for.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut letters = value.chars();
    while let Some(letter) = letters.next() {
        if letter != '\\' {
            out.push(letter);
            continue;
        }
        match letters.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The program and arguments an `Exec` line means when the application
/// is started without a file: quoted parts are one argument, the
/// places for files and addresses are dropped, `%i`, `%c` and `%k` are
/// filled in.
pub fn command_of(exec: &str, name: &str, icon: &str, entry: &Path) -> Vec<String> {
    commands_for(exec, name, icon, entry, &[])
        .pop()
        .unwrap_or_default()
}

/// One thing an application is started with, both ways an `Exec` line
/// can ask for it: as a file (`%f`: a path) and as an address (`%u`: a
/// URI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub file: String,
    pub address: String,
}

/// The words of an `Exec` line: parts between blanks, a quoted part one
/// word, a backslash inside quotes taking the next letter as it is. No
/// shell reads the line, here or later.
fn exec_words(exec: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut has_word = false;
    let mut quoted = false;
    let mut letters = exec.chars();
    while let Some(letter) = letters.next() {
        match letter {
            '"' => {
                quoted = !quoted;
                has_word = true;
            }
            '\\' if quoted => {
                if let Some(next) = letters.next() {
                    word.push(next);
                }
            }
            ' ' | '\t' if !quoted => {
                if has_word {
                    words.push(std::mem::take(&mut word));
                    has_word = false;
                }
            }
            other => {
                word.push(other);
                has_word = true;
            }
        }
    }
    if has_word {
        words.push(word);
    }
    words
}

/// The programs to start, each with its arguments, for an `Exec` line
/// and the things to open with it.
///
/// `%F` and `%U` take all of them, each an argument of its own, in one
/// program. `%f` and `%u` take one, so a line with only those is started
/// once for each thing, as the specification says. A line with no place
/// for a file gets each as its last argument, as if it ended in `%f`
/// (what GLib and KDE do). The codes the specification retired (`%d`,
/// `%D`, `%n`, `%N`, `%v`, `%m`) are dropped; `%i`, `%c`, `%k` and `%%`
/// are filled in. With nothing to open there is one program, without
/// the places for files. An empty line gives nothing.
pub fn commands_for(
    exec: &str,
    name: &str,
    icon: &str,
    entry: &Path,
    opened: &[Opened],
) -> Vec<Vec<String>> {
    let words = exec_words(exec);
    if words.is_empty() {
        return Vec::new();
    }
    // (`%%` is a percent sign, not the start of a code.)
    let codes = |word: &str, of: &[&str]| {
        let plain = word.replace("%%", "");
        of.iter().any(|code| plain.contains(code))
    };
    let takes_all = words.iter().any(|word| codes(word, &["%F", "%U"]));
    let takes_one = words.iter().any(|word| codes(word, &["%f", "%u"]));
    let build = |one: Option<&Opened>, all: &[Opened], append: bool| {
        let mut command = Vec::new();
        for word in &words {
            match word.as_str() {
                "%f" => command.extend(one.map(|opened| opened.file.clone())),
                "%u" => command.extend(one.map(|opened| opened.address.clone())),
                "%F" => command.extend(all.iter().map(|opened| opened.file.clone())),
                "%U" => command.extend(all.iter().map(|opened| opened.address.clone())),
                "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
                "%i" => {
                    if !icon.is_empty() {
                        command.push("--icon".to_string());
                        command.push(icon.to_string());
                    }
                }
                "%c" => command.push(name.to_string()),
                "%k" => command.push(entry.to_string_lossy().into_owned()),
                _ => command.push(fill(word, name, entry, one)),
            }
        }
        if append {
            command.extend(one.map(|opened| opened.file.clone()));
        }
        command
    };
    if opened.is_empty() {
        return vec![build(None, &[], false)];
    }
    if takes_all {
        // (A line with both kinds of place: the single ones get the first.)
        return vec![build(opened.first(), opened, false)];
    }
    opened
        .iter()
        .map(|one| build(Some(one), &[], !takes_one))
        .collect()
}

/// A word of an `Exec` line with the codes inside it filled in
/// (`--name=%c`), read once from the left: what is put in is never read
/// again, so a file named `%c` stays that name. `%%` is a percent sign;
/// the lists (`%F`, `%U`) and the retired codes have no place inside a
/// word and leave nothing.
fn fill(word: &str, name: &str, entry: &Path, one: Option<&Opened>) -> String {
    let mut filled = String::with_capacity(word.len());
    let mut letters = word.chars();
    while let Some(letter) = letters.next() {
        if letter != '%' {
            filled.push(letter);
            continue;
        }
        match letters.next() {
            Some('%') => filled.push('%'),
            Some('c') => filled.push_str(name),
            Some('k') => filled.push_str(&entry.to_string_lossy()),
            Some('f') => filled.push_str(one.map_or("", |opened| opened.file.as_str())),
            Some('u') => filled.push_str(one.map_or("", |opened| opened.address.as_str())),
            Some('F' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm' | 'i') => {}
            // Not a code: as it is written.
            Some(other) => {
                filled.push('%');
                filled.push(other);
            }
            None => filled.push('%'),
        }
    }
    filled
}

/// The directories applications are looked for in, the user's first.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::data_dir() {
        dirs.push(home);
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let system = if system.is_empty() {
        "/usr/local/share:/usr/share".to_string()
    } else {
        system
    };
    dirs.extend(
        system
            .split(':')
            .filter(|part| !part.is_empty())
            .map(PathBuf::from),
    );
    // Flatpak's exports, where the session did not put them on the list.
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/flatpak/exports/share"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// The user's languages for names, the most wanted first: `tr_TR`, `tr`.
pub fn locales() -> Vec<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default();
    locales_of(&raw)
}

fn locales_of(raw: &str) -> Vec<String> {
    // `tr_TR.UTF-8@euro` is `tr_TR`, then `tr`.
    let base = raw.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }
    let mut found = vec![base.to_string()];
    if let Some((language, _)) = base.split_once('_') {
        found.push(language.to_string());
    }
    found
}

/// Whether a program can be found: a path that is a file, or a name on
/// the `PATH`.
pub fn program_exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// Every application of `dirs`, by name. An entry in an earlier directory
/// stands in front of one with the same id in a later one, also when it
/// only says that the application is hidden.
pub fn installed_in(dirs: &[PathBuf], locales: &[String]) -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs {
        let root = dir.join("applications");
        let mut entries = Vec::new();
        collect(&root, &root, &mut entries, 0);
        entries.sort();
        for (id, path) in entries {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(app) = parse(&text, &id, &path, locales) else {
                continue;
            };
            // `TryExec`: the entry is only good when that program is there.
            let try_exec = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("TryExec="))
                .map(str::trim)
                .filter(|program| !program.is_empty());
            if try_exec.is_some_and(|program| !program_exists(program)) {
                continue;
            }
            apps.push(app);
        }
    }
    apps.sort_by_key(|one| one.name.to_lowercase());
    apps
}

/// The entries below `dir`, with their ids: a file in a folder has the
/// folder's name in front (`kde/konsole.desktop` is `kde-konsole`).
fn collect(root: &Path, dir: &Path, found: &mut Vec<(String, PathBuf)>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 3 {
                collect(root, &path, found, depth + 1);
            }
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("desktop") {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        // By the parts of the path, not by its text: the separator is not
        // `/` on every system.
        let id = relative
            .with_extension("")
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("-");
        found.push((id, path));
    }
}

/// The applications installed on this system.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn installed() -> Vec<App> {
    installed_in(&data_dirs(), &locales())
}

/// The folders macOS's applications are in.
#[cfg(target_os = "macos")]
pub fn application_folders() -> Vec<PathBuf> {
    let mut folders = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join("Applications"));
    }
    folders
}

/// The applications in these folders: each `.app` bundle, by its name, and
/// started by `open` (as the Finder does). The first of two of one name
/// stands.
pub fn bundles_in(folders: &[PathBuf]) -> Vec<App> {
    let mut found: Vec<App> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("app") {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if found.iter().any(|app| app.name == name) {
                continue;
            }
            found.push(App {
                id: path.to_string_lossy().into_owned(),
                name,
                generic_name: String::new(),
                comment: String::new(),
                keywords: Vec::new(),
                icon: String::new(),
                command: vec![
                    "/usr/bin/open".to_string(),
                    "-a".to_string(),
                    path.to_string_lossy().into_owned(),
                ],
                terminal: false,
                dbus_activatable: false,
                directory: None,
                entry: path,
                actions: Vec::new(),
            });
        }
    }
    found.sort_by_key(|app| app.name.to_lowercase());
    found
}

/// The applications installed on this Mac: the bundles of its application
/// folders.
#[cfg(target_os = "macos")]
pub fn installed() -> Vec<App> {
    bundles_in(&application_folders())
}

/// The applications installed on this system: those of the Start menu.
/// It asks the shell about every shortcut, which takes a moment: not for
/// the thread that draws.
#[cfg(windows)]
pub fn installed() -> Vec<App> {
    crate::start_menu::installed()
}

/// What the folders of desktop entries looked like when they were read:
/// each folder (and the folders in it) with when it last changed. A
/// folder changes when an entry comes, goes or is put in place anew,
/// which is how programs are installed, removed and updated.
pub fn stamp_of(dirs: &[PathBuf]) -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
    fn walk(dir: &Path, depth: usize, found: &mut Vec<(PathBuf, Option<std::time::SystemTime>)>) {
        let changed = std::fs::metadata(dir).and_then(|data| data.modified()).ok();
        found.push((dir.to_path_buf(), changed));
        if depth >= 3 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut inner: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        inner.sort();
        for path in inner {
            walk(&path, depth + 1, found);
        }
    }
    let mut found = Vec::new();
    for dir in dirs {
        walk(&dir.join("applications"), 0, &mut found);
    }
    found
}

/// The installed applications, kept: read again only when a folder of
/// desktop entries changed (or after a while, for an entry that was
/// written over in place, which changes no folder).
#[derive(Default)]
pub struct Kept {
    apps: Vec<App>,
    stamp: Vec<(PathBuf, Option<std::time::SystemTime>)>,
    read: Option<std::time::Instant>,
}

impl Kept {
    /// How long a list is trusted without looking at every entry again.
    const TRUSTED: std::time::Duration = std::time::Duration::from_secs(600);

    /// The applications of these folders, read again where needed.
    /// Whether they were read again is the second answer.
    pub fn of(&mut self, dirs: &[PathBuf], locales: &[String]) -> (&[App], bool) {
        let stamp = stamp_of(dirs);
        let stale = self
            .read
            .is_none_or(|read| read.elapsed() > Self::TRUSTED || self.stamp != stamp);
        if stale {
            self.apps = installed_in(dirs, locales);
            self.stamp = stamp;
            self.read = Some(std::time::Instant::now());
        }
        (&self.apps, stale)
    }

    /// The installed applications of this session's folders.
    #[cfg(not(windows))]
    pub fn installed(&mut self) -> Vec<App> {
        self.of(&data_dirs(), &locales()).0.to_vec()
    }
}

/// Start an application as the Start menu does (Windows; on Linux the
/// window manager runs an application's `command`, in the session's
/// environment). The answer is the number of the process, where the
/// shell says.
#[cfg(windows)]
pub fn start(app: &App) -> std::io::Result<Option<u32>> {
    crate::start_menu::start(app)
}

/// Start an application with an administrator's rights (Windows asks the
/// person first). An error where it cannot be started so (a packaged
/// application), and where the person said no.
#[cfg(windows)]
pub fn start_elevated(app: &App) -> std::io::Result<Option<u32>> {
    crate::start_menu::start_as(app, true)
}

/// Start an application on a Mac: its command (`open` and its bundle).
/// `open` ends at once: the number of the application's own process is
/// not known.
#[cfg(target_os = "macos")]
pub fn start(app: &App) -> std::io::Result<Option<u32>> {
    let (program, arguments) = app
        .command
        .split_first()
        .ok_or_else(|| std::io::Error::other(format!("{} has nothing to start", app.name)))?;
    std::process::Command::new(program)
        .args(arguments)
        .spawn()?;
    Ok(None)
}

/// macOS asks for an administrator's rights in the program that needs them,
/// not when it is started.
#[cfg(target_os = "macos")]
pub fn start_elevated(app: &App) -> std::io::Result<Option<u32>> {
    Err(std::io::Error::other(format!(
        "{} is not started as an administrator on macOS: a program asks for that itself",
        app.name
    )))
}

/// How well an application answers what was typed; higher is better,
/// `None` is not at all. The name counts most: its start, the start of
/// one of its words, a part of it, its letters in order; then what kind
/// of thing it is, its keywords, its description.
pub fn score(app: &App, typed: &str) -> Option<u32> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return Some(1);
    }
    let name = app.name.to_lowercase();
    if name == typed {
        return Some(1000);
    }
    if name.starts_with(&typed) {
        return Some(900 - name.len().min(100) as u32);
    }
    if name
        .split(|letter: char| !letter.is_alphanumeric())
        .any(|word| word.starts_with(&typed))
    {
        return Some(700);
    }
    if name.contains(&typed) {
        return Some(600);
    }
    let id = app.id.to_lowercase();
    if id
        .rsplit('.')
        .next()
        .is_some_and(|last| last.starts_with(&typed))
    {
        return Some(550);
    }
    if app.generic_name.to_lowercase().contains(&typed) {
        return Some(450);
    }
    if app
        .keywords
        .iter()
        .any(|keyword| keyword.to_lowercase().starts_with(&typed))
    {
        return Some(400);
    }
    if in_order(&name, &typed) {
        return Some(300);
    }
    if app.comment.to_lowercase().contains(&typed) {
        return Some(200);
    }
    None
}

/// Whether the letters of `typed` are in `text` in that order, beginning
/// with the first letter of a word ("tbd" finds Thunderbird; "fire" does
/// not find LibreOffice Writer, whose letters merely contain it).
fn in_order(text: &str, typed: &str) -> bool {
    let letters: Vec<char> = text.chars().collect();
    let Some(first) = typed.chars().next() else {
        return true;
    };
    (0..letters.len())
        .filter(|&at| letters[at] == first && (at == 0 || !letters[at - 1].is_alphanumeric()))
        .any(|at| {
            let mut rest = letters[at..].iter();
            typed
                .chars()
                .all(|wanted| rest.any(|letter| *letter == wanted))
        })
}

/// The applications that answer what was typed, the best first.
pub fn search<'a>(apps: &'a [App], typed: &str) -> Vec<&'a App> {
    let mut found: Vec<(u32, &App)> = apps
        .iter()
        .filter_map(|app| score(app, typed).map(|score| (score, app)))
        .collect();
    found.sort_by(|one, other| {
        other
            .0
            .cmp(&one.0)
            .then_with(|| one.1.name.to_lowercase().cmp(&other.1.name.to_lowercase()))
    });
    found.into_iter().map(|(_, app)| app).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_started_over_the_bus_needs_no_command() {
        let calculator = "[Desktop Entry]\nType=Application\nName=Calculator\nDBusActivatable=true\nActions=new;\n\n[Desktop Action new]\nName=New Window\n";
        let app = parse(
            calculator,
            "org.gnome.Calculator",
            Path::new("/usr/share/applications/org.gnome.Calculator.desktop"),
            &[],
        )
        .expect("an application");
        assert!(app.dbus_activatable);
        assert!(app.command.is_empty());
        assert_eq!(app.actions.len(), 1);
        assert_eq!(app.actions[0].id, "new");
        // The same under a name the bus would not take is no application:
        // there is no way to start it.
        assert!(parse(
            calculator,
            "calculator",
            Path::new("/x/calculator.desktop"),
            &[]
        )
        .is_none());
        // With a command it stays one, started the plain way.
        let plain = parse(
            &calculator.replace("Name=Calculator", "Name=Calculator\nExec=gnome-calculator"),
            "calculator",
            Path::new("/x/calculator.desktop"),
            &[],
        )
        .expect("an application");
        assert!(!plain.dbus_activatable);
        assert_eq!(plain.command, ["gnome-calculator"]);
    }

    #[test]
    fn the_path_on_the_bus_is_the_name_with_strokes() {
        assert_eq!(
            bus_path_of("org.gnome.Calculator").as_deref(),
            Some("/org/gnome/Calculator")
        );
        assert_eq!(
            bus_path_of("org.kde.kate-editor").as_deref(),
            Some("/org/kde/kate_editor")
        );
        assert_eq!(bus_path_of("firefox"), None);
        assert_eq!(bus_path_of("org..x"), None);
        assert_eq!(bus_path_of("org.7zip.App"), None);
        assert_eq!(bus_path_of("org.x y.App"), None);
    }

    #[test]
    fn the_kept_list_is_read_again_only_when_a_folder_changed() {
        let home = std::env::temp_dir().join(format!("gevrek-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let folder = home.join("applications");
        std::fs::create_dir_all(&folder).expect("a folder");
        let entry =
            |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n");
        std::fs::write(folder.join("one.desktop"), entry("One")).expect("a file");
        let dirs = [home.clone()];
        let mut kept = Kept::default();
        let (apps, read) = kept.of(&dirs, &[]);
        assert!(read && apps.len() == 1);
        // Nothing changed: not read again.
        let (apps, read) = kept.of(&dirs, &[]);
        assert!(!read && apps.len() == 1);
        // An entry comes (into a folder of its own, a moment later: a
        // folder's time has the clock's grain).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir_all(folder.join("kde")).expect("a folder");
        std::fs::write(folder.join("kde").join("two.desktop"), entry("Two")).expect("a file");
        let (apps, read) = kept.of(&dirs, &[]);
        assert!(read, "a new folder is a change");
        assert_eq!(apps.len(), 2);
        let (_, read) = kept.of(&dirs, &[]);
        assert!(!read);
        // The stamp names the folder and the one in it.
        assert_eq!(stamp_of(&dirs).len(), 2);
        assert_eq!(stamp_of(&[home.join("nothing")]).len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    const FIREFOX: &str = "[Desktop Entry]\nVersion=1.0\nType=Application\nName=Firefox\nName[tr]=Firefox Web Tarayıcı\nGenericName=Web Browser\nComment=Browse the Web\nExec=firefox %u\nIcon=firefox\nTerminal=false\nKeywords=Internet;WWW;Browser;\nCategories=Network;WebBrowser;\n\n[Desktop Action new-private-window]\nName=New Private Window\nExec=firefox --private-window %u\n";

    fn read(text: &str) -> Option<App> {
        parse(
            text,
            "test",
            Path::new("/usr/share/applications/test.desktop"),
            &[],
        )
    }

    #[test]
    fn an_entry_is_read_as_an_application() {
        let app = read(FIREFOX).expect("an application");
        assert_eq!(app.name, "Firefox");
        assert_eq!(app.generic_name, "Web Browser");
        assert_eq!(app.command, ["firefox"]);
        assert_eq!(app.keywords, ["Internet", "WWW", "Browser"]);
        assert_eq!(app.icon, "firefox");
        assert!(!app.terminal);
        // The action's own Name and Exec are not the application's.
        assert!(!app.command.contains(&"--private-window".to_string()));
    }

    #[test]
    fn names_come_in_the_users_language() {
        let path = Path::new("/x/firefox.desktop");
        let turkish = parse(
            FIREFOX,
            "firefox",
            path,
            &["tr_TR".to_string(), "tr".to_string()],
        )
        .expect("an application");
        assert_eq!(turkish.name, "Firefox Web Tarayıcı");
        let german = parse(FIREFOX, "firefox", path, &["de".to_string()]).expect("an application");
        assert_eq!(german.name, "Firefox");
        assert_eq!(locales_of("tr_TR.UTF-8"), ["tr_TR", "tr"]);
        assert_eq!(locales_of("en_GB.UTF-8@euro"), ["en_GB", "en"]);
        assert!(locales_of("C").is_empty());
        assert!(locales_of("").is_empty());
    }

    #[test]
    fn what_is_not_for_showing_is_left_out() {
        let base = "[Desktop Entry]\nType=Application\nName=X\nExec=x\n";
        assert!(read(base).is_some());
        assert!(read(&format!("{base}NoDisplay=true\n")).is_none());
        assert!(read(&format!("{base}Hidden=true\n")).is_none());
        assert!(read(&format!("{base}OnlyShowIn=GNOME;KDE;\n")).is_none());
        assert!(read(&format!("{base}OnlyShowIn=GNOME;gevrek;\n")).is_some());
        assert!(read(&format!("{base}NotShowIn=gevrek;\n")).is_none());
        assert!(read(&format!("{base}NotShowIn=KDE;\n")).is_some());
        assert!(read("[Desktop Entry]\nType=Link\nName=X\nURL=http://x\n").is_none());
        assert!(read("[Desktop Entry]\nType=Application\nName=X\n").is_none());
    }

    #[test]
    fn an_exec_line_becomes_a_program_and_its_arguments() {
        let entry = Path::new("/usr/share/applications/a.desktop");
        let command = |exec: &str| command_of(exec, "My App", "my-icon", entry);
        assert_eq!(command("gimp %U"), ["gimp"]);
        assert_eq!(
            command("env GDK_BACKEND=x11 \"/opt/My App/run\" --flag %f"),
            ["env", "GDK_BACKEND=x11", "/opt/My App/run", "--flag"]
        );
        assert_eq!(
            command("app %i --caption %c"),
            ["app", "--icon", "my-icon", "--caption", "My App"]
        );
        assert_eq!(
            command("app --title=%c %k"),
            ["app", "--title=My App", "/usr/share/applications/a.desktop"]
        );
        assert_eq!(
            command("sh -c \"echo 100%% \\\"done\\\"\""),
            ["sh", "-c", "echo 100% \"done\""]
        );
        assert!(command("").is_empty());
    }

    #[test]
    fn an_exec_line_takes_what_is_opened_where_it_asks_for_it() {
        let entry = Path::new("/usr/share/applications/a.desktop");
        let opened = |path: &str| Opened {
            file: path.to_string(),
            address: format!("file://{}", path.replace(' ', "%20")),
        };
        let two = [opened("/home/ada/a b.txt"), opened("/home/ada/c.txt")];
        let commands = |exec: &str, opened: &[Opened]| commands_for(exec, "App", "", entry, opened);
        // All of them in one program, each an argument of its own.
        assert_eq!(
            commands("edit --new %F", &two),
            [vec![
                "edit",
                "--new",
                "/home/ada/a b.txt",
                "/home/ada/c.txt"
            ]]
        );
        assert_eq!(
            commands("view %U", &two),
            [vec![
                "view",
                "file:///home/ada/a%20b.txt",
                "file:///home/ada/c.txt"
            ]]
        );
        // One at a time: a program for each.
        assert_eq!(
            commands("show %f", &two),
            [
                vec!["show", "/home/ada/a b.txt"],
                vec!["show", "/home/ada/c.txt"]
            ]
        );
        assert_eq!(
            commands("show --uri=%u", &two[..1]),
            [vec!["show", "--uri=file:///home/ada/a%20b.txt"]]
        );
        // No place for a file: it comes last.
        assert_eq!(
            commands("old-tool --flag", &two[..1]),
            [vec!["old-tool", "--flag", "/home/ada/a b.txt"]]
        );
        // The retired codes are dropped, a percent sign stays one, and
        // nothing a file is named can become a second argument or a code.
        let odd = [opened("/tmp/%F $(rm -rf) ;.txt"), opened("/tmp/%c%k")];
        assert_eq!(
            commands("tool %d %n %m 100%% %f", &odd[..1]),
            [vec!["tool", "100%", "/tmp/%F $(rm -rf) ;.txt"]]
        );
        assert_eq!(
            commands("tool --file=%f --name=%c", &odd[1..]),
            [vec!["tool", "--file=/tmp/%c%k", "--name=App"]]
        );
        // `%%f` is the text `%f`, and no place for a file.
        assert_eq!(
            commands("tool %%f", &two[..1]),
            [vec!["tool", "%f", "/home/ada/a b.txt"]]
        );
        assert_eq!(commands("show %f", &[]), [vec!["show"]]);
        assert!(commands("", &two).is_empty());
    }

    #[test]
    fn an_entry_that_is_not_for_the_list_still_opens_files() {
        let hidden = "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer %f\nNoDisplay=true\nOnlyShowIn=GNOME;\n";
        let path = Path::new("/x/viewer.desktop");
        assert!(parse(hidden, "viewer", path, &[]).is_none());
        let app = parse_as(hidden, "viewer", path, &[], false).expect("an application");
        assert_eq!(app.command, ["viewer"]);
        // Deleted is deleted.
        assert!(parse_as(
            &format!("{hidden}Hidden=true\n"),
            "viewer",
            path,
            &[],
            false
        )
        .is_none());
        let fields = main_group(hidden);
        assert_eq!(fields.plain("Exec"), Some("viewer %f"));
        assert_eq!(fields.list("OnlyShowIn"), ["GNOME"]);
        assert!(fields.yes("NoDisplay"));
    }

    #[test]
    fn the_users_entry_stands_in_front_of_the_systems() {
        let base = std::env::temp_dir().join(format!("gevrek-system-apps-{}", std::process::id()));
        let user = base.join("user");
        let system = base.join("system");
        for dir in [&user, &system] {
            std::fs::create_dir_all(dir.join("applications/kde")).expect("a folder");
        }
        let write = |dir: &Path, name: &str, text: &str| {
            std::fs::write(dir.join("applications").join(name), text).expect("written")
        };
        write(
            &system,
            "editor.desktop",
            "[Desktop Entry]\nType=Application\nName=System Editor\nExec=sh\n",
        );
        write(
            &user,
            "editor.desktop",
            "[Desktop Entry]\nType=Application\nName=My Editor\nExec=sh\n",
        );
        // Hidden by the user: the system's does not come back.
        write(
            &system,
            "games.desktop",
            "[Desktop Entry]\nType=Application\nName=Games\nExec=sh\n",
        );
        write(
            &user,
            "games.desktop",
            "[Desktop Entry]\nType=Application\nName=Games\nExec=sh\nHidden=true\n",
        );
        write(
            &system,
            "kde/konsole.desktop",
            "[Desktop Entry]\nType=Application\nName=Konsole\nExec=sh\n",
        );
        write(
            &system,
            "ghost.desktop",
            "[Desktop Entry]\nType=Application\nName=Ghost\nExec=sh\nTryExec=/no/such/program\n",
        );
        let apps = installed_in(&[user, system], &[]);
        let names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["Konsole", "My Editor"]);
        assert_eq!(apps[0].id, "kde-konsole");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_best_answer_to_what_was_typed_is_first() {
        let app = |name: &str, generic: &str, keywords: &[&str]| App {
            actions: Vec::new(),
            id: name.to_lowercase().replace(' ', "-"),
            name: name.to_string(),
            generic_name: generic.to_string(),
            comment: String::new(),
            keywords: keywords.iter().map(|keyword| keyword.to_string()).collect(),
            icon: String::new(),
            command: vec!["x".to_string()],
            terminal: false,
            dbus_activatable: false,
            directory: None,
            entry: PathBuf::new(),
        };
        let apps = vec![
            app("Thunderbird", "Mail Client", &["Email"]),
            app("Firefox", "Web Browser", &["Internet"]),
            app("Files", "File Manager", &[]),
            app("LibreOffice Writer", "Word Processor", &["Text"]),
            app("Terminal", "", &["shell"]),
        ];
        let names = |typed: &str| -> Vec<&str> {
            search(&apps, typed)
                .into_iter()
                .map(|app| app.name.as_str())
                .collect()
        };
        // The starts of names first; "fi" inside "LibreOffice" counts, last.
        assert_eq!(names("fi"), ["Files", "Firefox", "LibreOffice Writer"]);
        assert_eq!(names("fire"), ["Firefox"]);
        assert_eq!(names("writer"), ["LibreOffice Writer"]);
        assert_eq!(names("browser"), ["Firefox"]);
        assert_eq!(names("email"), ["Thunderbird"]);
        assert_eq!(names("tbd"), ["Thunderbird"]);
        assert!(names("zzz").is_empty());
        // Nothing typed: all of them, by name.
        assert_eq!(names("").len(), 5);
        assert_eq!(names("")[0], "Files");
    }

    #[test]
    fn an_entrys_actions_are_read_with_their_names_and_commands() {
        let text =
            "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser %u\nIcon=browser\n\
Actions=new-window;private;broken;missing;\n\n\
[Desktop Action new-window]\nName=New Window\nName[tr]=Yeni Pencere\nExec=browser --new-window\n\n\
[Desktop Action private]\nName=New Private Window\nExec=browser --private %u\n\n\
[Desktop Action broken]\nName=No command\n";
        let app = parse(
            text,
            "browser",
            Path::new("/x/browser.desktop"),
            &["tr".to_string()],
        )
        .expect("an application");
        let actions: Vec<(&str, &str, Vec<&str>)> = app
            .actions
            .iter()
            .map(|action| {
                (
                    action.id.as_str(),
                    action.name.as_str(),
                    action.command.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            actions,
            [
                (
                    "new-window",
                    "Yeni Pencere",
                    vec!["browser", "--new-window"]
                ),
                (
                    "private",
                    "New Private Window",
                    vec!["browser", "--private"]
                ),
            ],
            "in the entry's order; one without a command or a group is left out"
        );
        // An entry without actions has none.
        let plain = parse(
            "[Desktop Entry]\nType=Application\nName=Plain\nExec=plain\n",
            "plain",
            Path::new("/x/plain.desktop"),
            &[],
        )
        .expect("an application");
        assert!(plain.actions.is_empty());
    }
}
