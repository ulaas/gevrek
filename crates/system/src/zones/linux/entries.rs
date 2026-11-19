//! Desktop entries that lie in a folder a zone shows: launchers and
//! links, and the question whether one may be started.
//!
//! A `.desktop` file is a small program in all but name: starting it runs
//! whatever its `Exec` line says. One that arrived in a download or out
//! of an archive can show any name and any icon. So a desktop only starts
//! the ones it has reason to trust, and shows the others as the plain
//! files they are.
//!
//! What the desktops do: Nautilus (when it drew the desktop) and the
//! "Desktop Icons NG" extension want the executable bit **and** a mark of
//! their own, `metadata::trusted`, set by "Allow Launching", and refuse a
//! file others may write. KDE trusts entries in the system's folders and
//! otherwise wants the executable bit and the person (or root) as owner.
//! Xfce wants the executable bit and, since an archive keeps that bit, a
//! checksum of the file it wrote down when the person said "trust".
//!
//! **The rule here**, which is the strictest of these put together:
//!
//! 1. An entry whose real place is in an `applications` folder of the
//!    data directories is trusted: it was installed.
//! 2. Any other is trusted only when all of this holds: it is a regular
//!    file owned by the person (or root); neither the group nor others
//!    may write it; its owner may execute it; and its bytes are exactly
//!    the bytes the person allowed (`allow`), which gevrek keeps a copy
//!    of. A file that was changed since, or replaced by another of the
//!    same name, is not trusted any more.
//!
//! `metadata::trusted` itself is not read: it lives in GVfs's own store,
//! which only GIO reads. A launcher allowed under GNOME has to be allowed
//! once more here.

use super::places::Places;
use crate::apps::{self, App};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// A desktop entry larger than this is not read: none that is honest is.
const LARGEST: u64 = 1024 * 1024;

/// What a trusted entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// It starts a program (`Type=Application`).
    Application {
        app: Box<App>,
        /// Its `Exec` line as written, for when it is started with files.
        exec: String,
    },
    /// It leads to an address (`Type=Link`).
    Link { name: String, icon: String, url: String },
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Entry::Application { app, .. } => &app.name,
            Entry::Link { name, .. } => name,
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Entry::Application { app, .. } => &app.icon,
            Entry::Link { icon, .. } => icon,
        }
    }
}

/// What a file whose name ends in `.desktop` turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopFile {
    Trusted(Entry),
    /// A desktop entry the person has not allowed: shown as a file.
    Untrusted,
    /// Not a desktop entry that starts or leads to anything.
    NotAnEntry,
}

/// The number the system knows the person by. Asked of the system
/// without any library of C: a process's own folder under `/proc` is
/// owned by whoever it runs as; where there is no `/proc`, the home
/// folder's owner is the next best answer.
pub fn own_uid(places: &Places) -> Option<u32> {
    std::fs::metadata("/proc/self")
        .or_else(|_| std::fs::metadata(&places.home))
        .map(|data| data.uid())
        .ok()
}

/// Read an entry's text, where the file is one that may be read.
fn text_of(path: &Path) -> Option<String> {
    let data = std::fs::metadata(path).ok()?;
    if !data.is_file() || data.len() > LARGEST {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// What an entry's text says it is, without asking whether it is trusted.
pub fn read(text: &str, path: &Path, locales: &[String]) -> Option<Entry> {
    let fields = apps::main_group(text);
    match fields.plain("Type")? {
        "Application" => {
            let id = path.file_stem()?.to_string_lossy();
            let app = apps::parse_as(text, &id, path, locales, false)?;
            Some(Entry::Application {
                app: Box::new(app),
                exec: fields.plain("Exec").unwrap_or_default().to_string(),
            })
        }
        "Link" => {
            let url = fields.plain("URL").filter(|url| !url.is_empty())?;
            let name = fields.localised("Name", locales);
            Some(Entry::Link {
                name: if name.is_empty() { url.to_string() } else { name },
                icon: fields.plain("Icon").unwrap_or_default().to_string(),
                url: url.to_string(),
            })
        }
        _ => None,
    }
}

/// Where the copy of an allowed entry is kept. Its name is only a way to
/// find it; what counts is that the bytes are the same.
fn record_of(places: &Places, bytes: &[u8]) -> PathBuf {
    places
        .trusted_launchers()
        .join(format!("{}.desktop", super::md5::hex(bytes)))
}

/// Whether the entry at `path` is trusted by the rule at the top.
pub fn trusted(places: &Places, uid: Option<u32>, path: &Path) -> bool {
    // (Through links: a link on the desktop to an installed entry is the
    // usual way to put a program there.)
    let Ok(real) = std::fs::canonicalize(path) else {
        return false;
    };
    let Ok(data) = std::fs::metadata(&real) else {
        return false;
    };
    if !data.is_file() || data.len() > LARGEST {
        return false;
    }
    let installed = places.all_data_dirs().iter().any(|dir| {
        std::fs::canonicalize(dir.join("applications")).is_ok_and(|folder| real.starts_with(folder))
    });
    if installed {
        return true;
    }
    let mode = data.permissions().mode();
    let owned = uid.is_some_and(|uid| data.uid() == uid) || data.uid() == 0;
    if !owned || mode & 0o100 == 0 || mode & 0o022 != 0 {
        return false;
    }
    let Ok(bytes) = std::fs::read(&real) else {
        return false;
    };
    std::fs::read(record_of(places, &bytes)).is_ok_and(|allowed| allowed == bytes)
}

/// What the `.desktop` file at `path` is.
pub fn look(places: &Places, uid: Option<u32>, locales: &[String], path: &Path) -> DesktopFile {
    let Some(entry) = text_of(path).and_then(|text| read(&text, path, locales)) else {
        return DesktopFile::NotAnEntry;
    };
    if trusted(places, uid, path) {
        DesktopFile::Trusted(entry)
    } else {
        DesktopFile::Untrusted
    }
}

/// The person allows the entry at `path` to be started ("Allow
/// launching"). It is made executable for its owner and unwritable for
/// everyone else, and its bytes as they are now are written down. Refused
/// for a file that is no desktop entry, or not the person's own: trusting
/// what someone else can still change would be trusting them.
pub fn allow(places: &Places, uid: Option<u32>, path: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let real = std::fs::canonicalize(path)?;
    let data = std::fs::metadata(&real)?;
    let text = text_of(&real).ok_or_else(|| Error::new(ErrorKind::InvalidData, "not a desktop entry"))?;
    if read(&text, &real, &[]).is_none() {
        return Err(Error::new(ErrorKind::InvalidData, "not a desktop entry"));
    }
    if uid.is_none_or(|uid| data.uid() != uid) {
        return Err(Error::new(ErrorKind::PermissionDenied, "the file is someone else's"));
    }
    let mode = data.permissions().mode();
    let wanted = (mode | 0o100) & !0o022;
    if wanted != mode {
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(wanted))?;
    }
    // Read once more after the rights are set: what is written down is
    // what the file is now, with nobody else able to change it.
    let bytes = std::fs::read(&real)?;
    if bytes != text.as_bytes() {
        return Err(Error::new(ErrorKind::InvalidData, "the file changed while it was read"));
    }
    let record = record_of(places, &bytes);
    super::private_folder(&places.trusted_launchers())?;
    super::write_private(&record, &bytes)
}

/// Take back that the entry at `path` may be started: its owner may not
/// execute it any more. (The copy that was written down stays; without
/// the right to execute it means nothing.)
pub fn disallow(path: &Path) -> std::io::Result<()> {
    let real = std::fs::canonicalize(path)?;
    let mode = std::fs::metadata(&real)?.permissions().mode();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(mode & !0o111))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAUNCHER: &str =
        "[Desktop Entry]\nType=Application\nName=Tool\nName[tr]=Alet\nExec=tool --open %f\nIcon=tool\n";

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("rights");
    }

    #[test]
    fn a_launcher_is_only_trusted_when_the_person_allowed_exactly_this_file() {
        let root = crate::zones::linux::testing::folder("trust");
        let places = Places::under(&root);
        let uid = own_uid(&places);
        assert!(uid.is_some());
        let desktop = places.desktop();
        std::fs::create_dir_all(&desktop).expect("a folder");
        let path = desktop.join("tool.desktop");
        std::fs::write(&path, LAUNCHER).expect("written");
        let seen = || look(&places, uid, &[], &path);

        // As it arrived: a file.
        set_mode(&path, 0o644);
        assert_eq!(seen(), DesktopFile::Untrusted);
        // The executable bit alone is what an archive would carry too.
        set_mode(&path, 0o755);
        assert_eq!(seen(), DesktopFile::Untrusted);

        // Allowed: executable, nobody else may write, and it starts.
        set_mode(&path, 0o664);
        allow(&places, uid, &path).expect("allowed");
        let mode = std::fs::metadata(&path).expect("there").permissions().mode() & 0o777;
        assert_eq!(mode, 0o744);
        match seen() {
            DesktopFile::Trusted(Entry::Application { app, exec }) => {
                assert_eq!(app.name, "Tool");
                assert_eq!(app.command, ["tool", "--open"]);
                assert_eq!(exec, "tool --open %f");
            }
            other => panic!("not trusted: {other:?}"),
        }

        // Others may write it: no more.
        set_mode(&path, 0o766);
        assert_eq!(seen(), DesktopFile::Untrusted);
        set_mode(&path, 0o744);
        assert!(matches!(seen(), DesktopFile::Trusted(_)));
        // Not executable: no more.
        set_mode(&path, 0o644);
        assert_eq!(seen(), DesktopFile::Untrusted);
        set_mode(&path, 0o744);

        // Changed after it was allowed: no more, whatever its rights.
        std::fs::write(&path, LAUNCHER.replace("tool --open", "rm -rf")).expect("written");
        assert_eq!(seen(), DesktopFile::Untrusted);
        // The same bytes again are the same file again.
        std::fs::write(&path, LAUNCHER).expect("written");
        assert!(matches!(seen(), DesktopFile::Trusted(_)));

        // Taken back.
        disallow(&path).expect("taken back");
        assert_eq!(seen(), DesktopFile::Untrusted);

        // Someone else's file cannot be allowed.
        let stranger = uid.map(|uid| uid + 1);
        assert!(allow(&places, stranger, &path).is_err());
        assert_eq!(look(&places, stranger, &[], &path), DesktopFile::Untrusted);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_installed_entry_is_trusted_also_through_a_link() {
        let root = crate::zones::linux::testing::folder("trust-installed");
        let places = Places::under(&root);
        let uid = own_uid(&places);
        let applications = places.data_dirs[1].join("applications");
        let desktop = places.desktop();
        std::fs::create_dir_all(&applications).expect("a folder");
        std::fs::create_dir_all(&desktop).expect("a folder");
        let installed = applications.join("tool.desktop");
        std::fs::write(&installed, LAUNCHER).expect("written");
        set_mode(&installed, 0o644);
        let link = desktop.join("Tool.desktop");
        std::os::unix::fs::symlink(&installed, &link).expect("a link");
        let tr = ["tr".to_string()];
        match look(&places, uid, &tr, &link) {
            DesktopFile::Trusted(entry) => assert_eq!(entry.name(), "Alet"),
            other => panic!("not trusted: {other:?}"),
        }
        // A copy of it on the desktop is a file like any other.
        let copy = desktop.join("copy.desktop");
        std::fs::copy(&installed, &copy).expect("copied");
        assert_eq!(look(&places, uid, &[], &copy), DesktopFile::Untrusted);
        // A link to an entry that is not installed gains nothing.
        let outside = root.join("elsewhere.desktop");
        std::fs::write(&outside, LAUNCHER).expect("written");
        set_mode(&outside, 0o755);
        let sly = desktop.join("sly.desktop");
        std::os::unix::fs::symlink(&outside, &sly).expect("a link");
        assert_eq!(look(&places, uid, &[], &sly), DesktopFile::Untrusted);
        // A folder of that name inside the desktop is not the system's.
        let fake = desktop.join("applications");
        std::fs::create_dir_all(&fake).expect("a folder");
        std::fs::write(fake.join("x.desktop"), LAUNCHER).expect("written");
        assert_eq!(look(&places, uid, &[], &fake.join("x.desktop")), DesktopFile::Untrusted);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_link_entry_and_what_is_no_entry() {
        let path = Path::new("/home/ada/Desktop/site.desktop");
        let link = "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org/a\nIcon=web\n";
        assert_eq!(
            read(link, path, &[]),
            Some(Entry::Link {
                name: "Site".to_string(),
                icon: "web".to_string(),
                url: "https://example.org/a".to_string()
            })
        );
        assert_eq!(read("[Desktop Entry]\nType=Link\nName=No address\n", path, &[]), None);
        assert_eq!(read("[Desktop Entry]\nType=Directory\nName=Folder\n", path, &[]), None);
        assert_eq!(read("just text", path, &[]), None);
        // An entry that hides itself from lists is still a launcher where
        // it lies.
        let hidden = format!("{LAUNCHER}NoDisplay=true\n");
        assert!(matches!(read(&hidden, path, &[]), Some(Entry::Application { .. })));
        let root = crate::zones::linux::testing::folder("trust-none");
        let places = Places::under(&root);
        std::fs::write(root.join("notes.desktop"), "hello").expect("written");
        assert_eq!(
            look(&places, own_uid(&places), &[], &root.join("notes.desktop")),
            DesktopFile::NotAnEntry
        );
        assert!(allow(&places, own_uid(&places), &root.join("notes.desktop")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
