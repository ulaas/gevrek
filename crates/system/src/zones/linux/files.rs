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

/// Copy what is at `from`, whatever it is, to `to`, which must not be
/// there: a folder with everything in it, a link as a link.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let data = std::fs::symlink_metadata(from)?;
    let kind = data.file_type();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
    } else if kind.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        // The folder's own rights last: one that may not be written to
        // could not have been filled.
        std::fs::set_permissions(to, data.permissions())
    } else if kind.is_file() {
        // (Made anew and never over something: `to` appearing meanwhile
        // is an error, not a file lost.)
        let mut source = std::fs::File::open(from)?;
        let mut target = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
        std::io::copy(&mut source, &mut target)?;
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
    std::fs::symlink_metadata(from)?;
    folder_there(folder)?;
    let name = from.file_name().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?;
    if std::fs::canonicalize(folder)?.starts_with(std::fs::canonicalize(from)?) && from.is_dir() && !from.is_symlink() {
        return Err(Error::new(ErrorKind::InvalidInput, "a folder cannot go into itself"));
    }
    let to = free_name(folder, &name.to_string_lossy())?;
    if let Err(error) = copy_tree(from, &to) {
        // Half a copy is not left lying.
        let _ = remove_tree(&to);
        return Err(error);
    }
    Ok(to)
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
            if let Err(error) = copy_tree(from, &to) {
                let _ = remove_tree(&to);
                return Err(error);
            }
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
