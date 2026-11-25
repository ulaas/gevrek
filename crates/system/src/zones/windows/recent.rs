//! The files the person opened lately, as the shell of Windows keeps
//! them: the Recent folder (`FOLDERID_Recent`), into which the shell puts
//! a shortcut to every file that is opened through it (a double click, a
//! program's "Open" window, `SHAddToRecentDocs`). The shortcut's own time
//! of writing is when the file was last opened.
//!
//! Two things are made of that folder:
//!
//! - **The list** for a zone of recent files: the files the newest
//!   shortcuts lead to, the newest first ([`Reader::read`]).
//! - **When each file was last opened** ([`Opened`]), which becomes the
//!   fact `accessed` of that file wherever a zone shows it. The file
//!   system's own time of last access is not used on Windows: it is
//!   switched off on most machines (`NtfsDisableLastAccessUpdate`, the
//!   system's own choice for large drives since Windows 10 1803), is only
//!   kept to the hour where it is on, and is moved by any program that
//!   reads the file, this one's making of small pictures among them. What
//!   the shell wrote down is the person's own opening of the file, and
//!   nothing else.
//!
//! A shortcut is read as it is written: its target, and what the shell
//! noted of the target when it made the shortcut (size, times, whether it
//! is a folder). The target itself is only gone to where that is quick (a
//! drive of this machine): a file on another machine is shown by what
//! the shortcut says, so that a machine which does not answer holds
//! nothing up.

use super::shell::{self, Types};
use crate::com::Com;
use gevrek_zones::ids::ItemId;
use gevrek_zones::item::{extension_of, Item, ItemFacts, ItemKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, WIN32_FIND_DATAW};
use windows::Win32::System::Com::{
    CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// How many files a list of recent ones holds at most.
pub const MOST: usize = 100;

/// When each file was last opened, by its path in small letters, as
/// seconds since 1970. Shared between whoever reads the Recent folder
/// and everyone who reads the facts of items.
#[derive(Debug, Clone, Default)]
pub struct Opened(Arc<RwLock<HashMap<String, u64>>>);

impl Opened {
    /// When the file at `path` was last opened, if the shell wrote it
    /// down.
    pub fn at(&self, path: &str) -> Option<u64> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&path.to_lowercase())
            .copied()
    }

    /// Note an opening. The answer is whether that is news.
    fn note(&self, path: &str, when: u64) -> bool {
        let mut known = self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known.insert(path.to_lowercase(), when) != Some(when)
    }
}

/// What a shortcut says of what it leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    target: String,
    folder: bool,
    size: u64,
    created: Option<u64>,
    modified: Option<u64>,
}

/// A time as Windows counts it (tenths of a microsecond since 1601) as
/// seconds since 1970. Nothing for no time at all, and for one before
/// 1970.
pub fn seconds_of(time: FILETIME) -> Option<u64> {
    let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    (ticks / 10_000_000)
        .checked_sub(11_644_473_600)
        .filter(|_| ticks != 0)
}

/// Read one shortcut as it is written. The shell is not asked to look
/// for a target that moved (which can take long, and can ask the person).
/// Nothing (`Ok(None)`) where it leads to no file; an error where it could
/// not be read at all, as while the shell that writes it still holds it.
///
/// # Safety
/// Called on a thread that is set up for the shell's objects.
unsafe fn read_link(file: &Path) -> Result<Option<Link>, ()> {
    let wide = HSTRING::from(file.as_os_str());
    let link: IShellLinkW =
        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|_| ())?;
    let stored: IPersistFile = link.cast().map_err(|_| ())?;
    stored
        .Load(PCWSTR(wide.as_ptr()), STGM_READ)
        .map_err(|_| ())?;
    let mut target = [0u16; 1024];
    let mut data = WIN32_FIND_DATAW::default();
    if link.GetPath(&mut target, &mut data, 0).is_err() {
        return Ok(None);
    }
    let end = target
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(target.len());
    let target = String::from_utf16_lossy(&target[..end]);
    if !super::facts::is_path(&target) {
        // (It leads to something that is no file: a place of the
        // shell's, a setting.)
        return Ok(None);
    }
    const DIRECTORY: u32 = 0x10;
    Ok(Some(Link {
        target,
        folder: data.dwFileAttributes & DIRECTORY != 0,
        size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
        created: seconds_of(data.ftCreationTime),
        modified: seconds_of(data.ftLastWriteTime),
    }))
}

/// Whether asking the file system about a path can take long: it is on
/// another machine, by its name (`\\server\share`) or by its drive (a
/// letter that stands for a share).
pub fn far(path: &str) -> bool {
    if path.starts_with("\\\\") {
        return true;
    }
    let Some(root) = path.get(..3) else {
        return true;
    };
    const REMOTE: u32 = 4;
    // SAFETY: the text ends with a zero.
    unsafe { GetDriveTypeW(&HSTRING::from(root)) == REMOTE }
}

/// The item a shortcut leads to, by what the shortcut itself says.
fn item_of_link(link: &Link, types: &mut Types) -> Item {
    let name = gevrek_zones::paths::file_name(&link.target).to_string();
    let extension = if link.folder {
        None
    } else {
        extension_of(&name)
    };
    Item {
        id: ItemId::new(link.target.clone()),
        kind: if link.folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        },
        path: Some(link.target.clone()),
        facts: ItemFacts {
            mime: if link.folder {
                Some(super::facts::FOLDER_TYPE.to_string())
            } else {
                extension
                    .as_deref()
                    .and_then(|extension| types.of(extension))
            },
            extension,
            size: (!link.folder).then_some(link.size),
            created: link.created,
            modified: link.modified,
            accessed: None,
            shortcut_target: None,
        },
        name,
    }
}

/// Reads the Recent folder, again and again as it changes. A shortcut
/// that was read is not read again until it is written anew.
#[derive(Debug, Default)]
pub struct Reader {
    /// What each shortcut said when it was last read, by its file and
    /// the time that file was written.
    links: HashMap<PathBuf, (SystemTime, Option<Link>)>,
}

/// What one reading of the folder gave.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Read {
    /// The files opened lately, the newest first.
    pub items: Vec<Item>,
    /// The files whose time of last opening is other than it was known
    /// (all of them at the first reading).
    pub opened_anew: Vec<String>,
    /// A shortcut could not be read: the shell was still writing it.
    /// Its being let go of does not always tell the file system's watch
    /// anything, so whoever reads asks again soon (see `read_recent`).
    pub held: bool,
}

impl Reader {
    /// Read the folder: note when each file was opened (`opened`), and
    /// give the `most` files opened last. A file that is gone is left
    /// out (where that is quick to know).
    pub fn read(&mut self, folder: &Path, opened: &Opened, types: &mut Types, most: usize) -> Read {
        let _com = Com::new();
        let mut shortcuts: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let is_shortcut = path
                    .extension()
                    .is_some_and(|ending| ending.eq_ignore_ascii_case("lnk"));
                let written = entry.metadata().ok()?.modified().ok()?;
                is_shortcut.then_some((path, written))
            })
            .collect();
        // The newest first; of two written in the same moment, by name,
        // so that the order is always the same.
        shortcuts.sort_by(|one, other| other.1.cmp(&one.1).then_with(|| one.0.cmp(&other.0)));
        self.links
            .retain(|file, _| shortcuts.iter().any(|(known, _)| known == file));
        let mut read = Read::default();
        let mut listed: Vec<String> = Vec::new();
        for (file, written) in shortcuts {
            let link = match self.links.get(&file) {
                Some((known, link)) if *known == written => link.clone(),
                // SAFETY: this thread is set up for the shell (`_com`).
                _ => match unsafe { read_link(&file) } {
                    Ok(link) => {
                        self.links.insert(file.clone(), (written, link.clone()));
                        link
                    }
                    // (Not to be read yet: the shell is still writing it.
                    // It is read again at the next reading, which its
                    // being written asks for, though its time may be the
                    // one it has now.)
                    Err(()) => {
                        self.links.remove(&file);
                        read.held = true;
                        None
                    }
                },
            };
            let (Some(link), Some(when)) = (link, super::facts::seconds(Ok(written))) else {
                continue;
            };
            let key = link.target.to_lowercase();
            // (Two shortcuts to one file: the newer one counts, and came
            // first.)
            if listed.contains(&key) {
                continue;
            }
            if opened.note(&link.target, when) {
                read.opened_anew.push(link.target.clone());
            }
            if read.items.len() >= most {
                listed.push(key);
                continue;
            }
            let item = if far(&link.target) {
                Some(item_of_link(&link, types))
            } else {
                // (As a zone that shows its folder has it: the same id,
                // the name the shell shows.)
                shell::item_by_id(&ItemId::new(link.target.clone()), types)
            };
            if let Some(mut item) = item {
                item.facts.accessed = Some(when);
                read.items.push(item);
            }
            listed.push(key);
        }
        read
    }
}

/// Write a shortcut to `target` at `file`, as the shell does when a file
/// is opened. For tests, which make a Recent folder of their own.
#[cfg(test)]
pub fn write_link(file: &Path, target: &str) -> bool {
    use windows::Win32::Foundation::BOOL;
    let _com = Com::new();
    let wide = HSTRING::from(file.as_os_str());
    // SAFETY: the shell's objects are used on this thread alone.
    unsafe {
        let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER)
        else {
            return false;
        };
        if link.SetPath(&HSTRING::from(target)).is_err() {
            return false;
        }
        let Ok(stored) = link.cast::<IPersistFile>() else {
            return false;
        };
        stored.Save(PCWSTR(wide.as_ptr()), BOOL::from(true)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("gevrek-zones-recent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        PathBuf::from(
            std::fs::canonicalize(&folder)
                .expect("the folder")
                .to_string_lossy()
                .trim_start_matches("\\\\?\\"),
        )
    }

    /// Say that a shortcut was written at `seconds` since 1970.
    fn written(file: &Path, seconds: u64) {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
        std::fs::File::options()
            .write(true)
            .open(file)
            .expect("opened")
            .set_modified(time)
            .expect("a time");
    }

    #[test]
    fn windows_times_become_seconds_since_1970() {
        let time = |ticks: u64| FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        assert_eq!(seconds_of(time(0)), None);
        assert_eq!(seconds_of(time(116_444_736_000_000_000)), Some(0));
        assert_eq!(
            seconds_of(time(116_444_736_000_000_000 + 1_700_000_000 * 10_000_000)),
            Some(1_700_000_000)
        );
        assert_eq!(seconds_of(time(5)), None);
        assert!(far("\\\\server\\share\\a.txt"));
        assert!(!far(&std::env::temp_dir().to_string_lossy()));
    }

    #[test]
    fn the_recent_folder_gives_the_files_opened_last_and_when() {
        let root = folder("read");
        let (recent, files) = (root.join("Recent"), root.join("files"));
        std::fs::create_dir_all(&recent).expect("a folder");
        std::fs::create_dir_all(files.join("Folder")).expect("folders");
        let file = |name: &str| files.join(name).to_string_lossy().into_owned();
        for name in ["old.txt", "new.png", "gone.txt"] {
            std::fs::write(files.join(name), b"x").expect("written");
        }
        let link = |name: &str, target: &str, at: u64| {
            let link = recent.join(format!("{name}.lnk"));
            assert!(write_link(&link, target), "{name}");
            written(&link, at);
        };
        link("old", &file("old.txt"), 1_700_000_000);
        link("new", &file("new.png"), 1_700_000_500);
        link("folder", &file("Folder"), 1_700_000_300);
        link("gone", &file("gone.txt"), 1_700_000_900);
        // A second, older shortcut to the same file; and a file in the
        // folder that is no shortcut.
        link("new (2)", &file("new.png"), 1_700_000_100);
        std::fs::write(recent.join("desktop.ini"), b"").expect("written");
        std::fs::remove_file(files.join("gone.txt")).expect("removed");

        let opened = Opened::default();
        let mut reader = Reader::default();
        let mut types = Types::default();
        let read = reader.read(&recent, &opened, &mut types, MOST);
        // (Told by the files they are, not by the names they are shown
        // with: the shell leaves the ending away where Explorer is set to
        // hide the endings of known types, as it is on a new Windows.)
        let seen: Vec<(&str, ItemKind, Option<u64>)> = read
            .items
            .iter()
            .map(|item| {
                (
                    gevrek_zones::paths::file_name(item.id.as_str()),
                    item.kind,
                    item.facts.accessed,
                )
            })
            .collect();
        // The newest first; what is gone is not shown; each file once.
        assert_eq!(
            seen,
            [
                ("new.png", ItemKind::File, Some(1_700_000_500)),
                ("Folder", ItemKind::Folder, Some(1_700_000_300)),
                ("old.txt", ItemKind::File, Some(1_700_000_000)),
            ]
        );
        // The item is the file itself, as a zone of its folder has it.
        assert_eq!(read.items[0].id.as_str(), file("new.png"));
        assert_eq!(read.items[0].facts.extension.as_deref(), Some("png"));
        assert_eq!(read.items[0].facts.size, Some(1));
        // When each was opened is known, also of the one that is gone.
        assert_eq!(opened.at(&file("OLD.TXT")), Some(1_700_000_000));
        assert_eq!(opened.at(&file("new.png")), Some(1_700_000_500));
        assert_eq!(opened.at(&file("gone.txt")), Some(1_700_000_900));
        assert_eq!(opened.at(&file("never.txt")), None);
        assert_eq!(read.opened_anew.len(), 4);

        // Read again with nothing changed: the same list, and no news.
        let again = reader.read(&recent, &opened, &mut types, MOST);
        assert_eq!(again.items, read.items);
        assert_eq!(again.opened_anew, Vec::<String>::new());

        // The old file is opened once more: it is the newest now, and
        // that alone is news. No more than asked for are given.
        written(&recent.join("old.lnk"), 1_700_001_000);
        let then = reader.read(&recent, &opened, &mut types, 2);
        assert_eq!(
            then.items
                .iter()
                .map(|item| gevrek_zones::paths::file_name(item.id.as_str()))
                .collect::<Vec<_>>(),
            ["old.txt", "new.png"]
        );
        assert_eq!(then.opened_anew, [file("old.txt")]);
        assert_eq!(opened.at(&file("old.txt")), Some(1_700_001_000));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A shortcut that is not to be read yet (the shell writing it holds
    /// it) is read at the next reading, though its time of writing is the
    /// same then: what could not be read is not taken for what it says.
    #[test]
    fn a_shortcut_held_while_it_is_written_is_read_when_it_is_let_go_of() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = folder("held");
        let (recent, files) = (root.join("Recent"), root.join("files"));
        std::fs::create_dir_all(&recent).expect("a folder");
        std::fs::create_dir_all(&files).expect("a folder");
        let target = files.join("held.txt");
        std::fs::write(&target, b"x").expect("written");
        let link = recent.join("held.lnk");
        assert!(write_link(&link, &target.to_string_lossy()));
        let written = std::fs::metadata(&link)
            .and_then(|data| data.modified())
            .expect("a time");

        let (opened, mut reader, mut types) =
            (Opened::default(), Reader::default(), Types::default());
        let held = std::fs::File::options()
            .read(true)
            .share_mode(0)
            .open(&link)
            .expect("held");
        assert_eq!(reader.read(&recent, &opened, &mut types, MOST).items, []);
        drop(held);
        assert_eq!(
            std::fs::metadata(&link)
                .and_then(|data| data.modified())
                .expect("a time"),
            written
        );
        let read = reader.read(&recent, &opened, &mut types, MOST);
        assert_eq!(
            read.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [target.to_string_lossy().as_ref()]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file on another machine is shown by what its shortcut says: the
    /// machine is not gone to.
    #[test]
    fn a_file_on_another_machine_is_taken_from_what_the_shortcut_says() {
        let link = Link {
            target: "\\\\nowhere.invalid\\share\\Films\\a film.MKV".to_string(),
            folder: false,
            size: 7_000_000_000,
            created: Some(1_600_000_000),
            modified: Some(1_650_000_000),
        };
        let asked = std::time::Instant::now();
        let item = item_of_link(&link, &mut Types::default());
        assert!(asked.elapsed() < Duration::from_secs(1));
        assert_eq!(item.name, "a film.MKV");
        assert_eq!(item.kind, ItemKind::File);
        assert_eq!(item.facts.extension.as_deref(), Some("mkv"));
        assert_eq!(item.facts.size, Some(7_000_000_000));
        assert_eq!(item.facts.modified, Some(1_650_000_000));
        let folder = item_of_link(
            &Link {
                folder: true,
                ..link
            },
            &mut Types::default(),
        );
        assert_eq!(folder.kind, ItemKind::Folder);
        assert_eq!(folder.facts.size, None);
        assert_eq!(folder.facts.mime.as_deref(), Some("inode/directory"));
    }

    /// Read only: the person's own Recent folder can be listed. Nothing
    /// in it is opened, and nothing is written.
    #[test]
    fn the_real_recent_folder_can_be_listed() {
        let places = super::super::places::Places::of_session(Path::new("C:\\unused"));
        let recent = places.recent.expect("Windows names it");
        let listed = std::fs::read_dir(&recent).expect("it can be listed");
        let shortcuts = listed
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ending| ending.eq_ignore_ascii_case("lnk"))
            })
            .count();
        println!("{} shortcuts in the Recent folder", shortcuts);
    }
}
