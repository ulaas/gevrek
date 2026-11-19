//! The person's folders that smart zones look into, as Windows names
//! them: where downloads go, and where the shell keeps its record of the
//! files that were opened lately.
//!
//! They are asked of Windows once and handed to whoever needs them, so
//! that a test can hand over folders of its own instead: nothing that is
//! tested ever looks into the person's real Downloads or Recent.

use super::shell::taken;
use std::path::{Path, PathBuf};
use windows::core::GUID;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::UI::Shell::{
    FOLDERID_Downloads, FOLDERID_Recent, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

/// The folders, and gevrek's own folder for what it keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    /// Where downloads go (`FOLDERID_Downloads`).
    pub downloads: Option<PathBuf>,
    /// Where the shell puts a shortcut to every file that is opened
    /// (`FOLDERID_Recent`).
    pub recent: Option<PathBuf>,
    /// gevrek's state folder: the launcher's counts and the zones' pins.
    pub state: PathBuf,
}

fn known(folder: &GUID) -> Option<PathBuf> {
    // SAFETY: the text is the shell's to give and ours to free.
    let path = unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, HANDLE::default()) }.ok()?;
    let path = unsafe { taken(path) };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

impl Places {
    /// The folders of the person this program runs for, with gevrek's
    /// state under `state`.
    pub fn of_session(state: &Path) -> Self {
        Self {
            downloads: known(&FOLDERID_Downloads),
            recent: known(&FOLDERID_Recent),
            state: state.to_path_buf(),
        }
    }

    /// Pretend folders under `root` (`Downloads`, `Recent`, `state`),
    /// for a test. They are not made here.
    pub fn under(root: &Path) -> Self {
        Self {
            downloads: Some(root.join("Downloads")),
            recent: Some(root.join("Recent")),
            state: root.join("state"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read only: the two folders are asked of Windows and are folders.
    /// Nothing in them is looked at.
    #[test]
    fn windows_names_the_persons_folders() {
        let places = Places::of_session(Path::new("C:\\state"));
        for folder in [&places.downloads, &places.recent] {
            let folder = folder.as_ref().expect("Windows names it");
            assert!(folder.is_absolute(), "{}", folder.display());
        }
        assert_eq!(places.state, Path::new("C:\\state"));
        let pretend = Places::under(Path::new("C:\\t"));
        assert_eq!(pretend.recent.as_deref(), Some(Path::new("C:\\t\\Recent")));
        assert_eq!(pretend.downloads.as_deref(), Some(Path::new("C:\\t\\Downloads")));
    }
}
