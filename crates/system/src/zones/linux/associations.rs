//! Which program opens which kind of file, by the MIME Applications
//! specification ("mime-apps-spec").
//!
//! The person's choices and the system's are in files named
//! `mimeapps.list`, read in this order, each also in a form for this
//! desktop alone (`gevrek-mimeapps.list`) that comes just before it:
//!
//! 1. `$XDG_CONFIG_HOME`
//! 2. each of `$XDG_CONFIG_DIRS`
//! 3. `$XDG_DATA_HOME/applications` (the old place)
//! 4. each of `$XDG_DATA_DIRS/applications`
//!
//! A file has three groups: `[Default Applications]` (the program that
//! opens a type), `[Added Associations]` (programs that can, besides what
//! they say themselves) and `[Removed Associations]` (programs that are
//! not to be offered though they say they can). Beside them, each
//! `applications` folder has a `mimeinfo.cache` listing which of its
//! programs say they open which type; where a folder has none, the
//! entries themselves are read for their `MimeType`.
//!
//! **The default** for a type is the first valid program named as its
//! default, files taken in order; where none is named, the first program
//! that can open it. **A removal** takes a program away from everything
//! that is read after it, and from the cache of its own folder. A type
//! that has nothing falls back to what it is a kind of (a Python script
//! is opened by what opens text).

use super::mime::MimeDatabase;
use super::places::Places;
use crate::apps::{self, App};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A program files can be opened with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The name of its entry, with `.desktop` (`org.gnome.TextEditor.desktop`):
    /// what `open_with` takes.
    pub id: String,
    pub name: String,
    /// The name of its icon, or the path of a picture.
    pub icon: String,
    pub app: App,
    /// Its `Exec` line as written.
    pub exec: String,
}

/// One `mimeapps.list`, or one folder's cache: who is named for a type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Layer {
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, Vec<String>>,
    /// What the programs of the folder say of themselves.
    cache: HashMap<String, Vec<String>>,
}

/// Everything that was read, the most important first.
#[derive(Debug, Clone, Default)]
pub struct Associations {
    layers: Vec<Layer>,
    /// The `applications` folders, the most important first.
    folders: Vec<PathBuf>,
    locales: Vec<String>,
}

/// The lines of one group of a file as `key -> list`.
fn group(text: &str, name: &str) -> HashMap<String, Vec<String>> {
    let mut found = HashMap::new();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) == Some(name);
            continue;
        }
        if !inside || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let ids: Vec<String> = value
                .split(';')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect();
            found.insert(key.trim().to_string(), ids);
        }
    }
    found
}

fn list_layer(path: &Path) -> Option<Layer> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(Layer {
        defaults: group(&text, "Default Applications"),
        added: group(&text, "Added Associations"),
        removed: group(&text, "Removed Associations"),
        cache: HashMap::new(),
    })
}

/// What the entries of a folder say they open, read from the entries
/// themselves (the folder has no cache).
fn scan(folder: &Path) -> HashMap<String, Vec<String>> {
    fn walk(root: &Path, dir: &Path, depth: usize, found: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 {
                    walk(root, &path, depth + 1, found);
                }
            } else if path.extension().is_some_and(|ending| ending == "desktop") {
                if let Ok(relative) = path.strip_prefix(root) {
                    let id = relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("-");
                    found.push((id, path));
                }
            }
        }
    }
    let mut entries = Vec::new();
    walk(folder, folder, 0, &mut entries);
    entries.sort();
    let mut cache: HashMap<String, Vec<String>> = HashMap::new();
    for (id, path) in entries {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for mime in apps::main_group(&text).list("MimeType") {
            cache.entry(mime).or_default().push(id.clone());
        }
    }
    cache
}

impl Associations {
    /// Read the lists and caches of a session's places.
    pub fn load(places: &Places, locales: &[String]) -> Self {
        let mut layers = Vec::new();
        let mut folders = Vec::new();
        let lists = |dir: &Path, layers: &mut Vec<Layer>| {
            for desktop in &places.desktops {
                layers.extend(list_layer(&dir.join(format!("{desktop}-mimeapps.list"))));
            }
            layers.extend(list_layer(&dir.join("mimeapps.list")));
        };
        lists(&places.config_home, &mut layers);
        for dir in &places.config_dirs {
            lists(dir, &mut layers);
        }
        for data in places.all_data_dirs() {
            let folder = data.join("applications");
            lists(&folder, &mut layers);
            let cache = match std::fs::read_to_string(folder.join("mimeinfo.cache")) {
                Ok(text) => group(&text, "MIME Cache"),
                Err(_) => scan(&folder),
            };
            layers.push(Layer { cache, ..Layer::default() });
            folders.push(folder);
        }
        Self { layers, folders, locales: locales.to_vec() }
    }

    /// The entry a program's id names: `kde-konsole.desktop` is
    /// `konsole.desktop` in a folder `kde` as well. The first folder that
    /// has it decides, also when its entry says the program is gone.
    fn entry_of(&self, id: &str) -> Option<PathBuf> {
        let mut names = vec![id.to_string()];
        let mut at = 0;
        while let Some(dash) = id[at..].find('-') {
            let cut = at + dash;
            names.push(format!("{}/{}", id[..cut].replace('-', "/"), &id[cut + 1..]));
            at = cut + 1;
        }
        self.folders.iter().find_map(|folder| {
            names
                .iter()
                .map(|name| folder.join(name))
                .find(|path| path.is_file())
        })
    }

    /// The program an id names, where it is one that can be started.
    pub fn program(&self, id: &str) -> Option<Program> {
        let id = if id.ends_with(".desktop") { id.to_string() } else { format!("{id}.desktop") };
        let entry = self.entry_of(&id)?;
        let text = std::fs::read_to_string(&entry).ok()?;
        let app = apps::parse_as(&text, id.trim_end_matches(".desktop"), &entry, &self.locales, false)?;
        let fields = apps::main_group(&text);
        // `TryExec`: the entry only counts when that program is there.
        if fields
            .plain("TryExec")
            .is_some_and(|program| !program.is_empty() && !apps::program_exists(program))
        {
            return None;
        }
        Some(Program {
            id,
            name: app.name.clone(),
            icon: app.icon.clone(),
            exec: fields.plain("Exec").unwrap_or_default().to_string(),
            app,
        })
    }

    /// The ids named for exactly this type, the most wanted first:
    /// defaults, then what was added, then what programs say themselves;
    /// each without what a list read before it (or beside it) removed.
    fn ids_for(&self, mime: &str) -> Vec<String> {
        let mut removed: Vec<&String> = Vec::new();
        let mut defaults: Vec<&String> = Vec::new();
        let mut others: Vec<&String> = Vec::new();
        for layer in &self.layers {
            removed.extend(layer.removed.get(mime).into_iter().flatten());
            let allowed = |id: &&String| !removed.contains(id);
            defaults.extend(layer.defaults.get(mime).into_iter().flatten().filter(allowed));
            others.extend(layer.added.get(mime).into_iter().flatten().filter(allowed));
            others.extend(layer.cache.get(mime).into_iter().flatten().filter(allowed));
        }
        let mut ids: Vec<String> = Vec::new();
        for id in defaults.into_iter().chain(others) {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids
    }

    /// The programs that can open a type, the default first, then the
    /// others in the order the lists give; after those, what opens the
    /// kinds the type is a kind of.
    pub fn candidates(&self, mimes: &MimeDatabase, mime: &str) -> Vec<Program> {
        let mut found: Vec<Program> = Vec::new();
        for kind in mimes.with_ancestors(mime) {
            for id in self.ids_for(&kind) {
                if found.iter().any(|program| program.id == id) {
                    continue;
                }
                found.extend(self.program(&id));
            }
        }
        found
    }

    /// The program that opens a type when the person just opens a file.
    pub fn default_for(&self, mimes: &MimeDatabase, mime: &str) -> Option<Program> {
        mimes
            .with_ancestors(mime)
            .iter()
            .find_map(|kind| self.ids_for(kind).iter().find_map(|id| self.program(id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(folder: &Path, id: &str, name: &str, more: &str) {
        let path = folder.join(id);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("folders");
        std::fs::write(
            path,
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec={} %f\n{more}", name.to_lowercase()),
        )
        .expect("written");
    }

    fn names(programs: &[Program]) -> Vec<&str> {
        programs.iter().map(|program| program.name.as_str()).collect()
    }

    #[test]
    fn the_lists_are_read_in_the_order_the_specification_gives() {
        let root = crate::zones::linux::testing::folder("mimeapps");
        let places = Places::under(&root);
        let user_apps = places.data_home.join("applications");
        let system_apps = places.data_dirs[1].join("applications");
        for (folder, id, name) in [
            (&system_apps, "editor.desktop", "Editor"),
            (&system_apps, "viewer.desktop", "Viewer"),
            (&system_apps, "browser.desktop", "Browser"),
            (&system_apps, "kde/kate.desktop", "Kate"),
            (&user_apps, "mine.desktop", "Mine"),
        ] {
            entry(folder, id, name, "");
        }
        // What the programs say themselves.
        std::fs::write(
            system_apps.join("mimeinfo.cache"),
            "[MIME Cache]\ntext/plain=viewer.desktop;editor.desktop;kde-kate.desktop;\ntext/html=browser.desktop;\nimage/png=viewer.desktop;\n",
        )
        .expect("written");
        let mimes = MimeDatabase::default();
        let read = || Associations::load(&places, &[]);

        // Nothing chosen: the first that says it can.
        let none = read();
        assert_eq!(none.default_for(&mimes, "text/plain").expect("one").name, "Viewer");
        assert_eq!(names(&none.candidates(&mimes, "text/plain")), ["Viewer", "Editor", "Kate"]);
        assert!(none.default_for(&mimes, "video/mp4").is_none());
        // A kind of text is opened by what opens text, after its own.
        assert_eq!(names(&none.candidates(&mimes, "text/html")), ["Browser", "Viewer", "Editor", "Kate"]);

        // The system's list in the data directory names a default.
        std::fs::write(
            system_apps.join("mimeapps.list"),
            "[Default Applications]\ntext/plain=editor.desktop;\n",
        )
        .expect("written");
        assert_eq!(read().default_for(&mimes, "text/plain").expect("one").name, "Editor");

        // The system's configuration comes before it.
        std::fs::create_dir_all(&places.config_dirs[0]).expect("a folder");
        std::fs::write(
            places.config_dirs[0].join("mimeapps.list"),
            "[Default Applications]\ntext/plain=kde-kate.desktop;\n",
        )
        .expect("written");
        assert_eq!(read().default_for(&mimes, "text/plain").expect("one").name, "Kate");

        // The person's own before that; a default that is not installed
        // is passed over; what was added stands before what programs say.
        std::fs::create_dir_all(&places.config_home).expect("a folder");
        std::fs::write(
            places.config_home.join("mimeapps.list"),
            "[Default Applications]\ntext/plain=gone.desktop;mine.desktop;\n\n[Added Associations]\nimage/png=mine.desktop;\n\n[Removed Associations]\ntext/plain=viewer.desktop;\n",
        )
        .expect("written");
        let all = read();
        assert_eq!(all.default_for(&mimes, "text/plain").expect("one").name, "Mine");
        // The defaults of every list in order, then the rest; the removed
        // one is gone though its own folder's cache names it.
        assert_eq!(names(&all.candidates(&mimes, "text/plain")), ["Mine", "Kate", "Editor"]);
        assert_eq!(names(&all.candidates(&mimes, "image/png")), ["Mine", "Viewer"]);

        // The desktop's own list comes before the plain one beside it.
        std::fs::write(
            places.config_home.join("gevrek-mimeapps.list"),
            "[Default Applications]\ntext/plain=editor.desktop;\n",
        )
        .expect("written");
        assert_eq!(read().default_for(&mimes, "text/plain").expect("one").name, "Editor");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_without_a_cache_is_read_entry_by_entry() {
        let root = crate::zones::linux::testing::folder("mimeapps-scan");
        let places = Places::under(&root);
        let user_apps = places.data_home.join("applications");
        entry(&user_apps, "paint.desktop", "Paint", "MimeType=image/png;image/jpeg;\n");
        entry(&user_apps, "hidden.desktop", "Hidden", "MimeType=image/png;\nHidden=true\n");
        entry(&user_apps, "quiet.desktop", "Quiet", "MimeType=image/png;\nNoDisplay=true\n");
        entry(&user_apps, "absent.desktop", "Absent", "MimeType=image/png;\nTryExec=/no/such/program\n");
        let mimes = MimeDatabase::default();
        let read = Associations::load(&places, &[]);
        // One that says it was deleted, or whose program is not there, is
        // not offered; one that only keeps out of lists is.
        assert_eq!(names(&read.candidates(&mimes, "image/png")), ["Paint", "Quiet"]);
        assert_eq!(names(&read.candidates(&mimes, "image/jpeg")), ["Paint"]);
        let paint = read.program("paint").expect("a program");
        assert_eq!((paint.id.as_str(), paint.exec.as_str()), ("paint.desktop", "paint %f"));
        assert!(read.program("nothing.desktop").is_none());
        // The person's entry of a name stands in front of the system's.
        let system_apps = places.data_dirs[1].join("applications");
        entry(&system_apps, "paint.desktop", "System Paint", "MimeType=image/png;\n");
        let read = Associations::load(&places, &[]);
        assert_eq!(names(&read.candidates(&mimes, "image/png")), ["Paint", "Quiet"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
