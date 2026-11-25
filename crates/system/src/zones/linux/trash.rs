//! The trash, by the FreeDesktop.org Trash specification: a file that is
//! thrown away is moved into a trash can, with a note of where it was and
//! when it went, and can be put back from there by any file manager.
//!
//! A trash can is a folder with two folders in it: `files/` holds what
//! was thrown away, and `info/` has for each of them a `<name>.trashinfo`
//! saying where it came from (`Path=`, written as in an address) and when
//! (`DeletionDate=`, the local time).
//!
//! - **The home trash** is `$XDG_DATA_HOME/Trash`. Files of the same file
//!   system as it go there, with their whole path noted.
//! - **A file on another file system** (a stick, another disk) goes into
//!   a trash can at the top of that file system, so that throwing it
//!   away does not mean copying it: `$top/.Trash/$uid` where an
//!   administrator made `$top/.Trash` (a real folder with the sticky
//!   bit, as the specification demands before it is trusted), else
//!   `$top/.Trash-$uid`, made when needed. There the path is noted
//!   relative to `$top`, so that it still fits when the disk is mounted
//!   elsewhere.
//!
//! Nothing is ever deleted here: where no trash can can be had, throwing
//! away fails and the file stays.
//!
//! Written by hand rather than with the `trash` crate: that one finds
//! its folders in the environment of the whole process and looks through
//! every mounted file system to put one file back, so it can neither be
//! tried against a folder of a test nor asked for one file.

use super::uris;
use std::io::{Error, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Where a file went: its note and itself, in a trash can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    pub info: PathBuf,
    pub file: PathBuf,
}

/// One note of a trash can, read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    trashed: Trashed,
    /// Where the file was.
    original: PathBuf,
    /// When it went, as written (which sorts as time does).
    deleted: String,
    /// When the note was written: tells two of one second apart.
    written: Option<std::time::SystemTime>,
}

/// The trash of one person.
#[derive(Debug, Clone)]
pub struct Trash {
    /// `$XDG_DATA_HOME/Trash`.
    home: PathBuf,
    uid: u32,
}

/// The top of the file system a folder is on: the highest folder above
/// it that is still on the same one.
pub fn top_of(folder: &Path) -> std::io::Result<PathBuf> {
    let mut top = std::fs::canonicalize(folder)?;
    let device = std::fs::metadata(&top)?.dev();
    while let Some(parent) = top.parent() {
        if std::fs::metadata(parent)?.dev() != device {
            break;
        }
        top = parent.to_path_buf();
    }
    Ok(top)
}

/// The local time now, as a note says it (`2026-11-19T14:03:27`).
fn now() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S").to_string()
}

impl Trash {
    /// The trash whose home can is `home` (`$XDG_DATA_HOME/Trash`), of
    /// the person with this number.
    pub fn new(home: PathBuf, uid: u32) -> Self {
        Self { home, uid }
    }

    /// The can a file in `folder` goes into, and the top its path is
    /// noted from (none for the home trash, which notes whole paths).
    fn can_for(&self, folder: &Path) -> std::io::Result<(PathBuf, Option<PathBuf>)> {
        super::private_folder(&self.home.join("files"))?;
        super::private_folder(&self.home.join("info"))?;
        if std::fs::metadata(folder)?.dev() == std::fs::metadata(&self.home)?.dev() {
            return Ok((self.home.clone(), None));
        }
        let top = top_of(folder)?;
        let can = self.can_at(&top)?;
        Ok((can, Some(top)))
    }

    /// The can at the top of a file system, made where it has to be.
    fn can_at(&self, top: &Path) -> std::io::Result<PathBuf> {
        let shared = top.join(".Trash");
        // The administrator's folder is only used when nobody could have
        // put something else in its place: a real folder, not a link, and
        // sticky, so that one person cannot move another's can away.
        let usable = std::fs::symlink_metadata(&shared)
            .is_ok_and(|data| data.is_dir() && data.permissions().mode() & 0o1000 != 0);
        if usable {
            let can = shared.join(self.uid.to_string());
            if super::private_folder(&can.join("files")).is_ok()
                && super::private_folder(&can.join("info")).is_ok()
            {
                return Ok(can);
            }
        }
        let can = top.join(format!(".Trash-{}", self.uid));
        super::private_folder(&can.join("files"))?;
        super::private_folder(&can.join("info"))?;
        Ok(can)
    }

    /// The cans a file that was at `original` may be in: the home one,
    /// and those at the top of the file system its folder is on.
    fn cans_of(&self, original: &Path) -> Vec<(PathBuf, Option<PathBuf>)> {
        let mut cans = vec![(self.home.clone(), None)];
        // (The folder itself may be gone with the file; the nearest one
        // above it that is there is on the same file system.)
        let there = original.ancestors().skip(1).find(|folder| folder.is_dir());
        if let Some(top) = there.and_then(|folder| top_of(folder).ok()) {
            for can in [
                top.join(".Trash").join(self.uid.to_string()),
                top.join(format!(".Trash-{}", self.uid)),
            ] {
                cans.push((can, Some(top.clone())));
            }
        }
        cans
    }

    /// Throw the file or folder at `path` away. A link is thrown away
    /// itself, not what it leads to.
    pub fn put(&self, path: &Path) -> std::io::Result<Trashed> {
        let (folder, name) = match (path.parent(), path.file_name()) {
            (Some(folder), Some(name)) if path.is_absolute() => (folder, name),
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "not the path of a file",
                ))
            }
        };
        std::fs::symlink_metadata(path)?;
        // (By its real place, whatever links lead to its folder.)
        let original = std::fs::canonicalize(folder)?.join(name);
        if self.home.starts_with(&original) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the trash cannot be thrown away",
            ));
        }
        let (can, top) = self.can_for(folder)?;
        self.put_into(&can, top.as_deref(), &original)
    }

    /// Move `original` into the can `can`, noting its path whole, or
    /// from `top` on.
    fn put_into(
        &self,
        can: &Path,
        top: Option<&Path>,
        original: &Path,
    ) -> std::io::Result<Trashed> {
        let name = original
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
        let noted = match top {
            Some(top) => original
                .strip_prefix(top)
                .map_err(|_| Error::new(ErrorKind::InvalidInput, "not on that file system"))?,
            None => original,
        };
        let note = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            uris::escape(noted.as_os_str().as_bytes()),
            now()
        );
        // The note comes first and is made so that it cannot take the
        // place of another: that claims the name for the file as well.
        let as_text = name.to_string_lossy();
        let (stem, ending) = super::files::split_name(&as_text);
        for attempt in 1..10_000u32 {
            let kept_as = if attempt == 1 {
                name.to_os_string()
            } else {
                format!("{stem}.{attempt}{ending}").into()
            };
            let mut info_name = kept_as.clone();
            info_name.push(".trashinfo");
            let info = can.join("info").join(info_name);
            let made = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&info);
            let mut file = match made {
                Ok(file) => file,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            let kept = can.join("files").join(&kept_as);
            let moved = std::io::Write::write_all(&mut file, note.as_bytes())
                .and_then(|()| file.sync_all())
                .and_then(|()| std::fs::rename(original, &kept));
            return match moved {
                Ok(()) => Ok(Trashed { info, file: kept }),
                Err(error) => {
                    // The file stays where it was; no note without one.
                    if let Err(again) = std::fs::remove_file(&info) {
                        log::debug!(
                            "zones: the trash's note {} is left without its file: {again}",
                            info.display()
                        );
                    }
                    Err(error)
                }
            };
        }
        Err(Error::new(
            ErrorKind::AlreadyExists,
            "no free name in the trash",
        ))
    }

    /// The notes of one can.
    fn notes(can: &Path, top: Option<&Path>) -> Vec<Note> {
        let Ok(entries) = std::fs::read_dir(can.join("info")) else {
            return Vec::new();
        };
        let mut notes = Vec::new();
        for entry in entries.flatten() {
            let info = entry.path();
            // (By its bytes: a name need not be text.)
            let Some(kept_as) = info
                .file_name()
                .and_then(|name| name.as_bytes().strip_suffix(b".trashinfo"))
                .map(|name| std::ffi::OsStr::from_bytes(name).to_os_string())
            else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&info) else {
                continue;
            };
            let said = |key: &str| {
                text.lines()
                    .skip_while(|line| line.trim() != "[Trash Info]")
                    .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            };
            let Some(path) = said("Path").and_then(uris::unescape) else {
                continue;
            };
            let path = PathBuf::from(
                <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(path),
            );
            // A whole path as it is; a relative one from the top of the
            // can's file system. (A relative one in the home can, or one
            // that climbs out with `..`, is not a note to act on.)
            let original = match (path.is_absolute(), top) {
                (true, _) => path,
                (false, Some(top))
                    if !path
                        .components()
                        .any(|part| part == std::path::Component::ParentDir) =>
                {
                    top.join(path)
                }
                _ => continue,
            };
            notes.push(Note {
                trashed: Trashed {
                    file: can.join("files").join(kept_as),
                    info: info.clone(),
                },
                original,
                deleted: said("DeletionDate").unwrap_or_default().trim().to_string(),
                written: entry.metadata().and_then(|data| data.modified()).ok(),
            });
        }
        notes
    }

    /// A place as the trash notes it: by the real place of its folder,
    /// whatever links lead there (`put`). Where the folder is gone, as it
    /// is said. (What was thrown away from a folder reached through a
    /// link was noted by the real place and looked for by the said one:
    /// it was never found again to be put back.)
    fn as_noted(original: &Path) -> PathBuf {
        match (original.parent(), original.file_name()) {
            (Some(folder), Some(name)) => std::fs::canonicalize(folder)
                .map(|folder| folder.join(name))
                .unwrap_or_else(|_| original.to_path_buf()),
            _ => original.to_path_buf(),
        }
    }

    /// Put back what was last thrown away from `original`. An error where
    /// nothing from there is in the trash, and where something is at that
    /// place again: nothing is ever written over.
    pub fn restore(&self, original: &Path) -> std::io::Result<Trashed> {
        let noted = Self::as_noted(original);
        let original = noted.as_path();
        let mut found: Vec<Note> = self
            .cans_of(original)
            .iter()
            .flat_map(|(can, top)| Self::notes(can, top.as_deref()))
            .filter(|note| {
                note.original == original && note.trashed.file.symlink_metadata().is_ok()
            })
            .collect();
        found.sort_by(|one, other| {
            (&one.deleted, one.written).cmp(&(&other.deleted, other.written))
        });
        let note = found
            .pop()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "nothing from there is in the trash"))?;
        self.restore_this(&note.trashed, original)?;
        Ok(note.trashed)
    }

    /// Move one trashed file back to `original` and take its note away.
    fn restore_this(&self, trashed: &Trashed, original: &Path) -> std::io::Result<()> {
        if original.symlink_metadata().is_ok() {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                "something is there again",
            ));
        }
        if let Some(folder) = original.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::rename(&trashed.file, original)?;
        if let Err(error) = std::fs::remove_file(&trashed.info) {
            log::debug!(
                "zones: the trash's note {} stays after its file came back: {error}",
                trashed.info.display()
            );
        }
        Ok(())
    }

    /// Where everything thrown away from `original` is now, the last
    /// thrown away last.
    pub fn find(&self, original: &Path) -> Vec<Trashed> {
        let noted = Self::as_noted(original);
        let original = noted.as_path();
        let mut found: Vec<Note> = self
            .cans_of(original)
            .iter()
            .flat_map(|(can, top)| Self::notes(can, top.as_deref()))
            .filter(|note| note.original == original)
            .collect();
        found.sort_by(|one, other| {
            (&one.deleted, one.written).cmp(&(&other.deleted, other.written))
        });
        found.into_iter().map(|note| note.trashed).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trash_in(root: &Path) -> Trash {
        Trash::new(root.join("home/.local/share/Trash"), 1000)
    }

    #[test]
    fn a_file_goes_into_the_home_trash_with_its_note_and_comes_back() {
        let root = crate::zones::linux::testing::folder("trash");
        let root = std::fs::canonicalize(&root).expect("real");
        let trash = trash_in(&root);
        let desktop = root.join("home/Desktop");
        std::fs::create_dir_all(&desktop).expect("a folder");
        let file = desktop.join("çay notları %1.txt");
        std::fs::write(&file, "one").expect("written");

        let trashed = trash.put(&file).expect("thrown away");
        assert!(!file.exists());
        assert_eq!(
            trashed.file,
            root.join("home/.local/share/Trash/files/çay notları %1.txt")
        );
        assert_eq!(
            trashed.info,
            root.join("home/.local/share/Trash/info/çay notları %1.txt.trashinfo")
        );
        assert_eq!(std::fs::read_to_string(&trashed.file).expect("read"), "one");
        let note = std::fs::read_to_string(&trashed.info).expect("read");
        let mut lines = note.lines();
        assert_eq!(lines.next(), Some("[Trash Info]"));
        // The whole path, written as in an address.
        let path = lines.next().expect("a path");
        assert_eq!(
            path,
            format!(
                "Path={}/home/Desktop/%C3%A7ay%20notlar%C4%B1%20%251.txt",
                uris::escape(root.as_os_str().as_bytes())
            )
        );
        // The time as the specification writes it: 2026-11-19T14:03:27.
        let date = lines
            .next()
            .expect("a date")
            .strip_prefix("DeletionDate=")
            .expect("named");
        let shape: String = date
            .chars()
            .map(|letter| if letter.is_ascii_digit() { '0' } else { letter })
            .collect();
        assert_eq!(shape, "0000-00-00T00:00:00");
        // The can is the person's alone.
        let rights = std::fs::metadata(root.join("home/.local/share/Trash/files")).expect("there");
        assert_eq!(rights.permissions().mode() & 0o777, 0o700);

        // A second file of the same name gets another name in the can.
        std::fs::write(&file, "two").expect("written");
        let second = trash.put(&file).expect("thrown away");
        assert_eq!(
            second.file,
            root.join("home/.local/share/Trash/files/çay notları %1.2.txt")
        );
        assert_eq!(trash.find(&file).len(), 2);

        // Back comes one of them, to where it was; the other stays.
        let back = trash.restore(&file).expect("put back");
        assert!(file.exists() && !back.file.exists() && !back.info.exists());
        assert_eq!(trash.find(&file).len(), 1);
        // Something is there again: nothing is written over.
        let error = trash.restore(&file).expect_err("in the way");
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        let first = std::fs::read_to_string(&file).expect("read");
        std::fs::remove_file(&file).expect("removed");
        trash.restore(&file).expect("put back");
        let second = std::fs::read_to_string(&file).expect("read");
        let mut both = [first.as_str(), second.as_str()];
        both.sort_unstable();
        assert_eq!(both, ["one", "two"]);
        // Nothing from there is left.
        std::fs::remove_file(&file).expect("removed");
        assert_eq!(
            trash.restore(&file).expect_err("nothing left").kind(),
            ErrorKind::NotFound
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_was_thrown_away_through_a_link_to_its_folder_comes_back_by_the_same_way() {
        let root = crate::zones::linux::testing::folder("trash-through-a-link");
        let root = std::fs::canonicalize(&root).expect("real");
        let trash = trash_in(&root);
        let real = root.join("home/Documents/Desktop");
        std::fs::create_dir_all(&real).expect("a folder");
        // The desktop is a link to a folder elsewhere.
        let desktop = root.join("home/Desktop");
        std::os::unix::fs::symlink(&real, &desktop).expect("a link");
        let file = desktop.join("a.txt");
        std::fs::write(&file, "one").expect("written");
        trash.put(&file).expect("thrown away");
        assert!(!file.exists());
        // Found and put back by the place as it was said.
        assert_eq!(trash.find(&file).len(), 1);
        trash.restore(&file).expect("put back");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "one");
    }

    #[test]
    fn a_folder_and_a_link_are_thrown_away_as_they_are() {
        let root = crate::zones::linux::testing::folder("trash-kinds");
        let root = std::fs::canonicalize(&root).expect("real");
        let trash = trash_in(&root);
        let desktop = root.join("home/Desktop");
        std::fs::create_dir_all(desktop.join("Folder/inner")).expect("folders");
        std::fs::write(desktop.join("Folder/inner/a.txt"), "a").expect("written");
        std::fs::write(desktop.join("target.txt"), "kept").expect("written");
        std::os::unix::fs::symlink(desktop.join("target.txt"), desktop.join("link"))
            .expect("a link");

        trash.put(&desktop.join("Folder")).expect("thrown away");
        let link = trash.put(&desktop.join("link")).expect("thrown away");
        // The link went, not what it leads to.
        assert!(desktop.join("target.txt").exists());
        assert!(link
            .file
            .symlink_metadata()
            .expect("there")
            .file_type()
            .is_symlink());
        // The folder that held a file is gone too; putting the file's
        // folder back brings everything in it.
        trash.restore(&desktop.join("Folder")).expect("put back");
        assert_eq!(
            std::fs::read_to_string(desktop.join("Folder/inner/a.txt")).expect("read"),
            "a"
        );
        // What is not there cannot be thrown away, nor the trash itself.
        assert_eq!(
            trash
                .put(&desktop.join("nothing"))
                .expect_err("absent")
                .kind(),
            ErrorKind::NotFound
        );
        assert!(trash.put(&root.join("home/.local/share")).is_err());
        assert!(trash.put(Path::new("relative.txt")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn on_another_file_system_the_can_is_at_its_top_and_notes_from_there() {
        // No second file system can be had in a test, so the top of one
        // is played by a folder: everything but finding the top is real.
        let root = crate::zones::linux::testing::folder("trash-volume");
        let root = std::fs::canonicalize(&root).expect("real");
        let trash = trash_in(&root);
        let top = root.join("stick");
        std::fs::create_dir_all(top.join("photos")).expect("folders");
        let file = top.join("photos/a b.jpg");
        std::fs::write(&file, "picture").expect("written");

        // Nobody made `.Trash`: the person's own can at the top.
        let can = trash.can_at(&top).expect("a can");
        assert_eq!(can, top.join(".Trash-1000"));
        let trashed = trash
            .put_into(&can, Some(&top), &file)
            .expect("thrown away");
        assert_eq!(trashed.file, top.join(".Trash-1000/files/a b.jpg"));
        let note = std::fs::read_to_string(&trashed.info).expect("read");
        assert!(note.contains("\nPath=photos/a%20b.jpg\n"), "{note}");
        // Read from the can, the path is under the top again.
        let notes = Trash::notes(&can, Some(&top));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].original, file);
        trash
            .restore_this(&notes[0].trashed, &file)
            .expect("put back");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "picture");

        // An administrator's `.Trash` is used only when it is a real
        // folder and sticky.
        let shared = top.join(".Trash");
        std::fs::create_dir_all(&shared).expect("a folder");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).expect("rights");
        assert_eq!(trash.can_at(&top).expect("a can"), top.join(".Trash-1000"));
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).expect("rights");
        assert_eq!(trash.can_at(&top).expect("a can"), shared.join("1000"));
        std::fs::remove_dir_all(&shared).expect("removed");
        std::fs::create_dir_all(root.join("elsewhere")).expect("a folder");
        std::fs::set_permissions(
            root.join("elsewhere"),
            std::fs::Permissions::from_mode(0o1777),
        )
        .expect("rights");
        std::os::unix::fs::symlink(root.join("elsewhere"), &shared).expect("a link");
        assert_eq!(trash.can_at(&top).expect("a can"), top.join(".Trash-1000"));

        // A note that climbs out of its file system is not acted on.
        std::fs::write(
            can.join("info/evil.trashinfo"),
            "[Trash Info]\nPath=../../etc/passwd\nDeletionDate=2026-01-01T00:00:00\n",
        )
        .expect("written");
        assert!(Trash::notes(&can, Some(&top)).is_empty());
        // The top of a real folder is found, and is above it.
        let found = top_of(&top).expect("a top");
        assert!(top.starts_with(&found));
        let _ = std::fs::remove_dir_all(&root);
    }
}
