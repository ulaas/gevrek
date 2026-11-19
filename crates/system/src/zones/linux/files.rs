//! What portals and rules do to files: copy, move, rename, a new folder.
//! Each says what takes it back. (Throwing away and putting back are in
//! `trash`.)
//!
//! Nothing here writes over anything. A file that arrives where one of
//! its name is gets another name (`report (2).pdf`), as a file manager
//! would give it; a rename onto a name that is taken is refused.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

/// A name as what comes before its ending, and the ending with its dot
/// (`report`, `.pdf`). A name that only begins with a dot has no ending.
pub fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => name.split_at(dot),
        _ => (name, ""),
    }
}

/// A name for something arriving in `folder` that nothing there has:
/// `name`, else `name (2)`, `name (3)` and so on before the ending.
pub fn free_name(folder: &Path, name: &str) -> std::io::Result<PathBuf> {
    let (stem, ending) = split_name(name);
    for count in 1..10_000u32 {
        let candidate = if count == 1 {
            folder.join(name)
        } else {
            folder.join(format!("{stem} ({count}){ending}"))
        };
        if candidate.symlink_metadata().is_err() {
            return Ok(candidate);
        }
    }
    Err(Error::new(ErrorKind::AlreadyExists, "no free name"))
}

/// Whether a text can be the name of a file: one part of a path.
pub fn is_a_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// A folder that has to be there to take something.
fn folder_there(folder: &Path) -> std::io::Result<()> {
    if folder.is_dir() {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::NotFound, "the folder is not there"))
    }
}

/// What is told while something is copied: how many bytes are done of
/// how many (0: not counted). An error for an answer ends the copy with
/// that error, and nothing of the copy is left: `Interrupted` is how a
/// copy is called off. (A test plays a disk that is full this way.)
pub type Tick<'a> = &'a mut dyn FnMut(u64, u64) -> std::io::Result<()>;

/// How far a copy is.
struct Counted<'a> {
    done: u64,
    total: u64,
    tick: Tick<'a>,
}

impl Counted<'_> {
    /// So many bytes more are done. An error where the copy is to end.
    fn more(&mut self, bytes: u64) -> std::io::Result<()> {
        self.done += bytes;
        (self.tick)(self.done, self.total)
    }
}

/// A name beside where something is to arrive, under which it is
/// written until it is whole. It begins with a dot, so no zone and no
/// file manager shows it; what a run that was killed leaves is such a
/// file, never half a file under a real name.
fn part_name(folder: &Path, name: &str) -> PathBuf {
    static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    folder.join(format!(".{name}.gevrek-part-{}-{count}", std::process::id()))
}

/// Give what was written whole under a part name its real name: `name`
/// in `folder`, or the next free one. Never over anything: a name that
/// was taken meanwhile is passed over. The answer is where it is.
fn settle(part: &Path, folder: &Path, name: &str) -> std::io::Result<PathBuf> {
    let is_folder = std::fs::symlink_metadata(part)?.is_dir();
    for _ in 0..16 {
        let to = free_name(folder, name)?;
        let placed = if is_folder {
            std::fs::rename(part, &to)
        } else {
            // (A second name for the same file is only ever made where
            // there is none: that is what makes this safe against a
            // file that appears in the same moment.)
            match std::fs::hard_link(part, &to) {
                Ok(()) => std::fs::remove_file(part),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                // (A file system without second names.)
                Err(_) if to.symlink_metadata().is_err() => std::fs::rename(part, &to),
                Err(_) => continue,
            }
        };
        return placed.map(|()| to);
    }
    Err(Error::new(ErrorKind::AlreadyExists, "no free name"))
}

/// Copy `from` into `folder` under `name` (or the next free name): first
/// whole under a part name beside it, then named. On any error nothing
/// of it is left.
fn arrive(from: &Path, folder: &Path, name: &str, tick: Tick<'_>) -> std::io::Result<PathBuf> {
    let part = part_name(folder, name);
    let mut counted = Counted { done: 0, total: size_of_tree(from), tick };
    let arrived = copy_tree(from, &part, &mut counted).and_then(|()| settle(&part, folder, name));
    if arrived.is_err() {
        // Half a copy is not left lying.
        let _ = remove_tree(&part);
    }
    arrived
}

/// The bytes of the files at and below `path`.
fn size_of_tree(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(data) if data.is_dir() => std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| size_of_tree(&entry.path()))
            .sum(),
        Ok(data) if data.is_file() => data.len(),
        _ => 0,
    }
}

/// Copy what is at `from`, whatever it is, to `to`, which must not be
/// there: a folder with everything in it, a link as a link.
fn copy_tree(from: &Path, to: &Path, counted: &mut Counted<'_>) -> std::io::Result<()> {
    let data = std::fs::symlink_metadata(from)?;
    let kind = data.file_type();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
    } else if kind.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            // (Between two files is a place to be called off at, also
            // where the files are empty.)
            counted.more(0)?;
            copy_tree(&entry.path(), &to.join(entry.file_name()), counted)?;
        }
        // The folder's own rights last: one that may not be written to
        // could not have been filled.
        std::fs::set_permissions(to, data.permissions())
    } else if kind.is_file() {
        // (Made anew and never over something: `to` appearing meanwhile
        // is an error, not a file lost.)
        let mut source = std::fs::File::open(from)?;
        let mut target = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
        // (A piece at a time, so that it can say how far it is and be
        // called off between two pieces.)
        let mut piece = vec![0u8; 1 << 20];
        loop {
            let read = std::io::Read::read(&mut source, &mut piece)?;
            if read == 0 {
                break;
            }
            std::io::Write::write_all(&mut target, &piece[..read])?;
            counted.more(read as u64)?;
        }
        target.set_permissions(data.permissions())?;
        if let Ok(modified) = data.modified() {
            let _ = target.set_modified(modified);
        }
        Ok(())
    } else {
        Err(Error::new(ErrorKind::Unsupported, "neither a file, a folder nor a link"))
    }
}

/// Copy `from` into `folder`. The answer is where the copy is.
pub fn copy(from: &Path, folder: &Path) -> std::io::Result<PathBuf> {
    copy_told(from, folder, &mut |_, _| Ok(()))
}

/// As `copy`, telling `tick` how far it is, which may end it (then
/// nothing of the copy is left, and the error is the one `tick` gave).
pub fn copy_told(from: &Path, folder: &Path, tick: Tick<'_>) -> std::io::Result<PathBuf> {
    std::fs::symlink_metadata(from)?;
    folder_there(folder)?;
    let name = from.file_name().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
    if std::fs::canonicalize(folder)?.starts_with(std::fs::canonicalize(from)?) && from.is_dir() && !from.is_symlink() {
        return Err(Error::new(ErrorKind::InvalidInput, "a folder cannot go into itself"));
    }
    arrive(from, folder, &name.to_string_lossy(), tick)
}

/// Take away what a copy that failed left: only ever something this
/// module just made.
fn remove_tree(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(data) if data.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

/// Move `from` into `folder`. The answer is where it is now. Within one
/// file system it is renamed; to another it is copied whole and only
/// then taken away where it was.
pub fn move_into(from: &Path, folder: &Path) -> std::io::Result<PathBuf> {
    move_into_told(from, folder, &mut |_, _| Ok(()))
}

/// As `move_into`, telling `tick` how far a move to another file system
/// is, which may call it off (then the file is where it was, whole).
pub fn move_into_told(from: &Path, folder: &Path, tick: Tick<'_>) -> std::io::Result<PathBuf> {
    std::fs::symlink_metadata(from)?;
    folder_there(folder)?;
    let name = from.file_name().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
    if from.parent().is_some_and(|parent| same_place(parent, folder)) {
        // It is there already.
        return Ok(from.to_path_buf());
    }
    let to = free_name(folder, &name.to_string_lossy())?;
    match std::fs::rename(from, &to) {
        Ok(()) => Ok(to),
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            // Whole at the other end first, and only then taken away
            // here: called off or failed, it is where it was.
            let to = arrive(from, folder, &name.to_string_lossy(), tick)?;
            remove_tree(from)?;
            Ok(to)
        }
        Err(error) => Err(error),
    }
}

/// Whether two paths are the same folder, through whatever links.
fn same_place(one: &Path, other: &Path) -> bool {
    match (std::fs::canonicalize(one), std::fs::canonicalize(other)) {
        (Ok(one), Ok(other)) => one == other,
        _ => one == other,
    }
}

/// Give `path` another name in its folder. The answer is its new path.
pub fn rename(path: &Path, new_name: &str) -> std::io::Result<PathBuf> {
    if !is_a_name(new_name) {
        return Err(Error::new(ErrorKind::InvalidInput, "not a name"));
    }
    std::fs::symlink_metadata(path)?;
    let folder = path.parent().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no folder"))?;
    let to = folder.join(new_name);
    if to == path {
        return Ok(to);
    }
    if to.symlink_metadata().is_ok() {
        return Err(Error::new(ErrorKind::AlreadyExists, "the name is taken"));
    }
    std::fs::rename(path, &to)?;
    Ok(to)
}

/// Make a folder named `name` in `parent`; where the name is taken, with
/// the next free one. The answer is its path.
pub fn new_folder(parent: &Path, name: &str) -> std::io::Result<PathBuf> {
    if !is_a_name(name) {
        return Err(Error::new(ErrorKind::InvalidInput, "not a name"));
    }
    folder_there(parent)?;
    for _ in 0..16 {
        // (A folder's name has no ending to keep the number before.)
        let (mut count, mut path) = (1u32, parent.join(name));
        while path.symlink_metadata().is_ok() {
            count += 1;
            path = parent.join(format!("{name} ({count})"));
        }
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            // Someone else made it in between: the next name.
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(Error::new(ErrorKind::AlreadyExists, "no free name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_split_before_its_ending() {
        for (name, parts) in [
            ("report.pdf", ("report", ".pdf")),
            ("archive.tar.gz", ("archive.tar", ".gz")),
            (".profile", (".profile", "")),
            ("Makefile", ("Makefile", "")),
            ("ends.", ("ends.", "")),
        ] {
            assert_eq!(split_name(name), parts, "{name}");
        }
        assert!(is_a_name("a b.txt") && is_a_name("..."));
        for bad in ["", ".", "..", "a/b", "a\0b"] {
            assert!(!is_a_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn files_are_copied_moved_renamed_and_never_written_over() {
        let root = crate::zones::linux::testing::folder("files");
        let (desk, archive) = (root.join("desk"), root.join("archive"));
        std::fs::create_dir_all(&desk).expect("a folder");
        std::fs::create_dir_all(&archive).expect("a folder");
        let file = desk.join("report.pdf");
        std::fs::write(&file, "first").expect("written");

        // A copy; a second copy gets a name of its own.
        let copy_one = copy(&file, &archive).expect("copied");
        assert_eq!(copy_one, archive.join("report.pdf"));
        let copy_two = copy(&file, &archive).expect("copied");
        assert_eq!(copy_two, archive.join("report (2).pdf"));
        assert_eq!(std::fs::read_to_string(&copy_two).expect("read"), "first");
        assert!(file.exists());

        // A move: gone here, there under a free name.
        let moved = move_into(&file, &archive).expect("moved");
        assert_eq!(moved, archive.join("report (3).pdf"));
        assert!(!file.exists());
        // Into the folder it is in: nothing happens.
        assert_eq!(move_into(&moved, &archive).expect("there already"), moved);

        // A rename, refused onto a name that is taken or is none.
        let renamed = rename(&moved, "final.pdf").expect("renamed");
        assert_eq!(renamed, archive.join("final.pdf"));
        assert_eq!(rename(&renamed, "report.pdf").expect_err("taken").kind(), ErrorKind::AlreadyExists);
        assert_eq!(rename(&renamed, "a/b").expect_err("no name").kind(), ErrorKind::InvalidInput);
        assert_eq!(std::fs::read_to_string(archive.join("report.pdf")).expect("read"), "first");

        // New folders.
        assert_eq!(new_folder(&desk, "New folder").expect("made"), desk.join("New folder"));
        assert_eq!(new_folder(&desk, "New folder").expect("made"), desk.join("New folder (2)"));
        assert!(new_folder(&root.join("nowhere"), "x").is_err());

        // What is not there, and a folder that is not there.
        assert_eq!(copy(&desk.join("none"), &archive).expect_err("absent").kind(), ErrorKind::NotFound);
        assert_eq!(move_into(&renamed, &root.join("nowhere")).expect_err("absent").kind(), ErrorKind::NotFound);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_is_copied_with_what_is_in_it_and_links_stay_links() {
        let root = crate::zones::linux::testing::folder("files-tree");
        let (from, to) = (root.join("from"), root.join("to"));
        std::fs::create_dir_all(from.join("Album/inner")).expect("folders");
        std::fs::create_dir_all(&to).expect("a folder");
        std::fs::write(from.join("Album/inner/a.txt"), "a").expect("written");
        std::os::unix::fs::symlink("inner/a.txt", from.join("Album/link")).expect("a link");
        let copied = copy(&from.join("Album"), &to).expect("copied");
        assert_eq!(std::fs::read_to_string(copied.join("inner/a.txt")).expect("read"), "a");
        assert_eq!(std::fs::read_link(copied.join("link")).expect("a link"), Path::new("inner/a.txt"));
        // Not into itself.
        assert!(copy(&from.join("Album"), &from.join("Album/inner")).is_err());
        assert!(!from.join("Album/inner/Album").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
